using System.IO.Pipes;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Graphics;
using WinRT.Interop;

namespace Covenant.Setup.Ui;

internal sealed class InstallerUiWindow : Window
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
    };

    private readonly string _pipeName;
    private readonly TextBlock _statusText;
    private readonly ProgressBar _progressBar;
    private readonly TextBox _logBox;
    private readonly Button _saveErrataButton;
    private readonly Button _closeButton;
    private readonly StringBuilder _logText = new();
    private readonly object _writerLock = new();
    private StreamWriter? _writer;
    private bool _canClose;
    private bool _closeRequested;
    private string? _errataJson;

    public InstallerUiWindow(string pipeName)
    {
        _pipeName = pipeName;
        Title = "covenant-setup";

        _statusText = new TextBlock
        {
            Text = "Preparing...",
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center
        };

        _progressBar = new ProgressBar
        {
            Minimum = 0,
            Maximum = 100,
            Height = 24
        };

        _logBox = new TextBox
        {
            AcceptsReturn = true,
            FontFamily = new FontFamily("Consolas"),
            FontSize = 13,
            IsReadOnly = true,
            TextWrapping = TextWrapping.NoWrap
        };
        ScrollViewer.SetVerticalScrollBarVisibility(_logBox, ScrollBarVisibility.Auto);
        ScrollViewer.SetHorizontalScrollBarVisibility(_logBox, ScrollBarVisibility.Auto);

        _saveErrataButton = new Button
        {
            Content = "Save error data to local errata.json file?",
            IsEnabled = false,
            MinWidth = 320,
            Visibility = Visibility.Collapsed
        };
        _saveErrataButton.Click += async (_, _) => await SaveErrataAsync();

        _closeButton = new Button
        {
            Content = "Close",
            IsEnabled = false,
            MinWidth = 88
        };
        _closeButton.Click += (_, _) =>
        {
            _closeRequested = true;
            Close();
        };

        Content = BuildContent();
        ConfigureWindow();
    }

    public void StartPipeLoop()
    {
        _ = Task.Run(RunPipeLoop);
    }

    private Grid BuildContent()
    {
        var root = new Grid
        {
            Padding = new Thickness(12),
            RowSpacing = 12
        };
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        Grid.SetRow(_statusText, 0);
        root.Children.Add(_statusText);

        Grid.SetRow(_progressBar, 1);
        root.Children.Add(_progressBar);

        Grid.SetRow(_logBox, 2);
        root.Children.Add(_logBox);

        var buttonPanel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Spacing = 8
        };
        buttonPanel.Children.Add(_saveErrataButton);
        buttonPanel.Children.Add(_closeButton);

        Grid.SetRow(buttonPanel, 3);
        root.Children.Add(buttonPanel);

        return root;
    }

    private void ConfigureWindow()
    {
        var hwnd = WindowNative.GetWindowHandle(this);
        var windowId = Win32Interop.GetWindowIdFromWindow(hwnd);
        var appWindow = AppWindow.GetFromWindowId(windowId);
        appWindow.Title = Title;
        appWindow.Closing += (_, args) =>
        {
            if (!_canClose && !_closeRequested)
            {
                args.Cancel = true;
            }
        };

        const int width = 760;
        const int height = 480;
        var displayArea = DisplayArea.GetFromWindowId(windowId, DisplayAreaFallback.Primary);
        var workArea = displayArea.WorkArea;
        var x = workArea.X + Math.Max(0, (workArea.Width - width) / 2);
        var y = workArea.Y + Math.Max(0, (workArea.Height - height) / 2);
        appWindow.MoveAndResize(new RectInt32(x, y, width, height));
    }

    private void RunPipeLoop()
    {
        try
        {
            UiTrace.Write("pipe_server_create", new { PipeName = _pipeName });
            using var pipe = new NamedPipeServerStream(
                _pipeName,
                PipeDirection.InOut,
                1,
                PipeTransmissionMode.Byte,
                PipeOptions.None);
            UiTrace.Write("pipe_wait_for_connection", new { PipeName = _pipeName });
            pipe.WaitForConnection();
            UiTrace.Write("pipe_connected", new { PipeName = _pipeName });

            using var reader = new StreamReader(pipe, new UTF8Encoding(false), detectEncodingFromByteOrderMarks: false, bufferSize: 4096, leaveOpen: true);
            using var writer = new StreamWriter(pipe, new UTF8Encoding(false), bufferSize: 4096, leaveOpen: true)
            {
                AutoFlush = true,
                NewLine = "\n"
            };

            lock (_writerLock)
            {
                _writer = writer;
            }

            string? line;
            while ((line = reader.ReadLine()) is not null)
            {
                UiTrace.Write("pipe_receive", SafeMessageSummary(line));
                if (!HandleMessage(line))
                {
                    break;
                }
            }
        }
        catch (Exception ex)
        {
            UiTrace.Write("pipe_error", new { ex.Message, ex.GetType().FullName, ex.StackTrace });
            BeginInvokeSafe(() =>
            {
                AppendLog("UI pipe error: " + ex.Message);
                SetCanClose(true);
            });
        }
        finally
        {
            lock (_writerLock)
            {
                _writer = null;
            }
            UiTrace.Write("pipe_loop_exit");
        }
    }

    private bool HandleMessage(string line)
    {
        var message = JsonSerializer.Deserialize<UiMessage>(line, JsonOptions);
        if (message?.Type is null)
        {
            return true;
        }

        switch (message.Type)
        {
            case "init":
                BeginInvokeSafe(() =>
                {
                    Title = message.Title ?? "covenant-setup";
                    _statusText.Text = message.Message ?? Title;
                    _progressBar.Value = 0;
                });
                return true;

            case "progress":
                BeginInvokeSafe(() => ApplyProgress(message));
                return true;

            case "log":
                BeginInvokeSafe(() => AppendLog(message.Message ?? string.Empty));
                return true;

            case "finish":
                UiTrace.Write("finish_message", new { message.Message });
                BeginInvokeSafe(() =>
                {
                    _statusText.Text = message.Message ?? "Complete";
                    _progressBar.Value = 100;
                    SetCanClose(true);
                    _closeRequested = true;
                    Close();
                });
                return true;

            case "fail":
                UiTrace.Write("fail_message", new { message.AppName, message.Operation, message.Message, message.Error });
                BeginInvokeSafe(() => ApplyFailure(message));
                return true;

            case "prompt":
                UiTrace.Write("prompt_show_requested", new { message.Id, message.Title, message.Buttons, message.Icon });
                var result = ShowPrompt(message);
                UiTrace.Write("prompt_response", new { message.Id, Result = result });
                WriteResponse(new UiResponse
                {
                    Type = "prompt_response",
                    Id = message.Id,
                    Result = result
                });
                return true;

            case "close":
                UiTrace.Write("close_message");
                BeginInvokeSafe(() =>
                {
                    _closeRequested = true;
                    Close();
                });
                return false;

            default:
                return true;
        }
    }

    private void ApplyProgress(UiMessage message)
    {
        if (!string.IsNullOrWhiteSpace(message.Message))
        {
            _statusText.Text = message.Message;
            AppendLog(message.Message);
        }

        var total = Math.Max(1, message.TotalSteps ?? 1);
        var current = Math.Max(0, Math.Min(total, message.CurrentStep ?? 0));
        _progressBar.Value = Math.Max(0, Math.Min(100, current * 100 / total));
    }

    private void ApplyFailure(UiMessage message)
    {
        var operation = string.IsNullOrWhiteSpace(message.Operation) ? "complete" : message.Operation;
        var appName = string.IsNullOrWhiteSpace(message.AppName) ? "unknown" : message.AppName;
        var failureMessage = string.IsNullOrWhiteSpace(message.Message)
            ? $"Error: program {appName} failed to {operation} completely!"
            : message.Message;

        _statusText.Text = failureMessage;
        _progressBar.Value = 100;
        AppendLog(failureMessage);
        if (!string.IsNullOrWhiteSpace(message.Error))
        {
            AppendLog("Error details: " + message.Error);
        }

        _errataJson = BuildErrataJson(message);
        _saveErrataButton.IsEnabled = !string.IsNullOrWhiteSpace(_errataJson);
        _saveErrataButton.Visibility = Visibility.Visible;
        SetCanClose(true);
    }

    private async Task SaveErrataAsync()
    {
        if (string.IsNullOrWhiteSpace(_errataJson))
        {
            return;
        }

        try
        {
            var root = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            if (string.IsNullOrWhiteSpace(root))
            {
                root = Environment.CurrentDirectory;
            }

            var directory = Path.Combine(root, "CovenantSetup");
            Directory.CreateDirectory(directory);
            var path = Path.Combine(directory, "errata.json");
            File.WriteAllText(path, _errataJson, new UTF8Encoding(false));
            AppendLog("Saved error data to " + path);
            await ShowNoticeAsync("Error data saved to " + path, PromptIconKind.Information);
        }
        catch (Exception ex)
        {
            UiTrace.Write("errata_save_error", new { ex.Message, ex.GetType().FullName, ex.StackTrace });
            await ShowNoticeAsync("Unable to save errata.json: " + ex.Message, PromptIconKind.Error);
        }
    }

    internal static string BuildErrataJson(UiMessage message)
    {
        if (message.Errata is JsonElement errata &&
            errata.ValueKind is not JsonValueKind.Undefined and not JsonValueKind.Null)
        {
            return JsonSerializer.Serialize(errata, new JsonSerializerOptions { WriteIndented = true });
        }

        return JsonSerializer.Serialize(new
        {
            app_name = message.AppName,
            operation = message.Operation,
            message = message.Message,
            error = message.Error
        }, new JsonSerializerOptions { WriteIndented = true });
    }

    private string ShowPrompt(UiMessage message)
    {
        var tcs = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        if (!DispatcherQueue.TryEnqueue(async () =>
        {
            try
            {
                tcs.SetResult(await ShowPromptAsync(message));
            }
            catch (Exception ex)
            {
                tcs.SetException(ex);
            }
        }))
        {
            return "none";
        }

        return tcs.Task.GetAwaiter().GetResult();
    }

    private async Task<string> ShowPromptAsync(UiMessage message)
    {
        var buttons = MapButtons(message.Buttons);
        var dialog = new ContentDialog
        {
            Title = message.Title ?? "covenant-setup",
            Content = BuildDialogContent(message.Message ?? string.Empty, MapIcon(message.Icon))
        };
        ConfigureDialogButtons(dialog, buttons);

        if (Content is FrameworkElement root)
        {
            dialog.XamlRoot = root.XamlRoot;
        }

        var result = ToPromptDialogResult(await dialog.ShowAsync());
        UiTrace.Write("prompt_closed", new { message.Id, Result = result.ToString() });
        return MapDialogResult(result, buttons);
    }

    private async Task ShowNoticeAsync(string message, PromptIconKind icon)
    {
        var dialog = new ContentDialog
        {
            Title = "covenant-setup",
            CloseButtonText = "OK",
            Content = BuildDialogContent(message, icon)
        };

        if (Content is FrameworkElement root)
        {
            dialog.XamlRoot = root.XamlRoot;
        }

        _ = await dialog.ShowAsync();
    }

    private static FrameworkElement BuildDialogContent(string message, PromptIconKind icon)
    {
        var panel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 12
        };
        panel.Children.Add(new FontIcon
        {
            FontFamily = new FontFamily("Segoe MDL2 Assets"),
            FontSize = 20,
            Glyph = icon switch
            {
                PromptIconKind.Error => "\uE783",
                PromptIconKind.Warning => "\uE7BA",
                _ => "\uE946"
            },
            Foreground = new SolidColorBrush(icon switch
            {
                PromptIconKind.Error => Colors.Firebrick,
                PromptIconKind.Warning => Colors.DarkGoldenrod,
                _ => Colors.DodgerBlue
            })
        });
        panel.Children.Add(new TextBlock
        {
            Text = message,
            TextWrapping = TextWrapping.Wrap,
            MaxWidth = 480
        });
        return panel;
    }

    internal static PromptButtonSet MapButtons(string? buttons) => buttons switch
    {
        "ok_cancel" => PromptButtonSet.OkCancel,
        "yes_no" => PromptButtonSet.YesNo,
        _ => PromptButtonSet.Ok
    };

    internal static PromptIconKind MapIcon(string? icon) => icon switch
    {
        "error" => PromptIconKind.Error,
        "warning" => PromptIconKind.Warning,
        _ => PromptIconKind.Information
    };

    internal static string MapDialogResult(PromptDialogResult result, PromptButtonSet buttons) => result switch
    {
        PromptDialogResult.Primary when buttons == PromptButtonSet.YesNo => "yes",
        PromptDialogResult.Primary => "ok",
        PromptDialogResult.Secondary when buttons == PromptButtonSet.YesNo => "no",
        PromptDialogResult.Secondary => "cancel",
        PromptDialogResult.Close when buttons == PromptButtonSet.Ok => "ok",
        PromptDialogResult.Close when buttons == PromptButtonSet.OkCancel => "cancel",
        _ => "none"
    };

    private static PromptDialogResult ToPromptDialogResult(ContentDialogResult result) => result switch
    {
        ContentDialogResult.Primary => PromptDialogResult.Primary,
        ContentDialogResult.Secondary => PromptDialogResult.Secondary,
        _ => PromptDialogResult.Close
    };

    private static void ConfigureDialogButtons(ContentDialog dialog, PromptButtonSet buttons)
    {
        switch (buttons)
        {
            case PromptButtonSet.OkCancel:
                dialog.PrimaryButtonText = "OK";
                dialog.CloseButtonText = "Cancel";
                dialog.DefaultButton = ContentDialogButton.Primary;
                break;
            case PromptButtonSet.YesNo:
                dialog.PrimaryButtonText = "Yes";
                dialog.SecondaryButtonText = "No";
                dialog.DefaultButton = ContentDialogButton.Primary;
                break;
            default:
                dialog.CloseButtonText = "OK";
                dialog.DefaultButton = ContentDialogButton.Close;
                break;
        }
    }

    private void WriteResponse(UiResponse response)
    {
        lock (_writerLock)
        {
            _writer?.WriteLine(JsonSerializer.Serialize(response, JsonOptions));
            UiTrace.Write("pipe_send", new { response.Type, response.Id, response.Result });
        }
    }

    private void BeginInvokeSafe(Action action)
    {
        try
        {
            if (DispatcherQueue.HasThreadAccess)
            {
                action();
                return;
            }

            _ = DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Normal, () =>
            {
                try
                {
                    action();
                }
                catch (Exception ex)
                {
                    UiTrace.Write("ui_dispatch_error", new { ex.Message, ex.GetType().FullName, ex.StackTrace });
                }
            });
        }
        catch (Exception ex) when (ex is InvalidOperationException or ObjectDisposedException)
        {
        }
    }

    private void SetCanClose(bool canClose)
    {
        _canClose = canClose;
        _closeButton.IsEnabled = canClose;
    }

    private void AppendLog(string line)
    {
        if (string.IsNullOrWhiteSpace(line))
        {
            return;
        }

        if (_logText.Length > 0)
        {
            _logText.AppendLine();
        }
        _logText.Append(line);
        _logBox.Text = _logText.ToString();
        _logBox.SelectionStart = _logBox.Text.Length;
    }

    internal static object SafeMessageSummary(string line)
    {
        try
        {
            using var document = JsonDocument.Parse(line);
            var root = document.RootElement;
            return new
            {
                Type = root.TryGetProperty("type", out var type) ? type.GetString() : null,
                Id = root.TryGetProperty("id", out var id) ? id.GetString() : null,
                Message = root.TryGetProperty("message", out var message) ? message.GetString() : null
            };
        }
        catch
        {
            return new { RawLength = line.Length };
        }
    }
}
