using System.Text;
using System.Text.Json;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics;
using Windows.UI;
using WinRT.Interop;

namespace Covenant.Setup.Ui;

internal sealed class InstallerUiWindow : Window, IInstallerView
{
    private readonly string _pipeName;
    private readonly TextBlock _statusText;
    private readonly ProgressBar _progressBar;
    private readonly TextBox _logBox;
    private readonly Button _copyErrorButton;
    private readonly Button _saveErrataButton;
    private readonly Button _cancelButton;
    private readonly StringBuilder _logText = new();
    private bool _canClose;
    private bool _closeRequested;
    private string? _errataJson;
    private string? _failureMessage;
    private string? _errorDetails;
    private InstallerSessionController? _sessionController;
    private int _currentStep;
    private volatile bool _cancelRequested;

    // Assigned by BuildContent/BuildWelcomePanel before the constructor returns.
    private TextBlock _welcomeHeaderTitle = null!;
    private TextBlock _welcomeInfoText = null!;
    private TextBlock _welcomePathText = null!;
    private Border _welcomePathBorder = null!;
    private Image _brandingImage = null!;
    private FrameworkElement _brandingPlaceholder = null!;
    private TextBlock _welcomeTitle = null!;
    private Grid _welcomePanel = null!;
    private Grid _progressPanel = null!;
    private TaskCompletionSource<string>? _welcomeTcs;

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
        AutomationProperties.SetAutomationId(_progressBar, "ProgressBar");

        _logBox = new TextBox
        {
            AcceptsReturn = true,
            FontFamily = new FontFamily("Consolas"),
            FontSize = 13,
            IsReadOnly = true,
            TextWrapping = TextWrapping.NoWrap
        };
        AutomationProperties.SetAutomationId(_logBox, "LogBox");
        ScrollViewer.SetVerticalScrollBarVisibility(_logBox, ScrollBarVisibility.Auto);
        ScrollViewer.SetHorizontalScrollBarVisibility(_logBox, ScrollBarVisibility.Auto);

        _copyErrorButton = new Button
        {
            Content = "Copy",
            IsEnabled = false,
            MinWidth = 88,
            Visibility = Visibility.Collapsed
        };
        AutomationProperties.SetAutomationId(_copyErrorButton, "CopyErrorButton");
        _copyErrorButton.Click += (_, _) => CopyErrorToClipboard();

        _saveErrataButton = new Button
        {
            Content = "Save error data to local errata.json file?",
            IsEnabled = false,
            MinWidth = 320,
            Visibility = Visibility.Collapsed
        };
        AutomationProperties.SetAutomationId(_saveErrataButton, "SaveErrataButton");
        _saveErrataButton.Click += async (_, _) => await SaveErrataAsync();

        _cancelButton = new Button
        {
            Content = "Cancel",
            IsEnabled = true,
            MinWidth = 88
        };
        AutomationProperties.SetAutomationId(_cancelButton, "CancelButton");
        _cancelButton.Click += (_, _) =>
        {
            // _canClose tracks which face the button shows: false = "Cancel"
            // during the run, true = "Close" after the terminal message.
            if (_canClose)
            {
                _closeRequested = true;
                Close();
            }
            else
            {
                RequestCancel();
            }
        };

        Content = BuildContent();
        ConfigureWindow();
    }

    public void StartPipeLoop()
    {
        _sessionController = new InstallerSessionController(_pipeName, this);
        _ = Task.Run(_sessionController.Run);
    }

    private Grid BuildContent()
    {
        var root = new Grid();

        // 1. Progress Panel
        _progressPanel = new Grid
        {
            Padding = new Thickness(12),
            RowSpacing = 12,
            Visibility = Visibility.Collapsed
        };
        _progressPanel.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        _progressPanel.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        _progressPanel.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        _progressPanel.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        Grid.SetRow(_statusText, 0);
        _progressPanel.Children.Add(_statusText);

        Grid.SetRow(_progressBar, 1);
        _progressPanel.Children.Add(_progressBar);

        Grid.SetRow(_logBox, 2);
        _progressPanel.Children.Add(_logBox);

        var buttonPanel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Spacing = 8
        };
        buttonPanel.Children.Add(_copyErrorButton);
        buttonPanel.Children.Add(_saveErrataButton);
        buttonPanel.Children.Add(_cancelButton);

        Grid.SetRow(buttonPanel, 3);
        _progressPanel.Children.Add(buttonPanel);

        // 2. Welcome Panel
        _welcomePanel = BuildWelcomePanel();
        _welcomePanel.Visibility = Visibility.Collapsed; // Collapsed initially until ShowWelcomeAsync is called

        root.Children.Add(_progressPanel);
        root.Children.Add(_welcomePanel);

        return root;
    }

    private Grid BuildWelcomePanel()
    {
        var grid = new Grid
        {
            Padding = new Thickness(16),
            ColumnSpacing = 16
        };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(260) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        // Left Column: Branding Image Frame
        _brandingImage = new Image
        {
            Stretch = Stretch.UniformToFill,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            VerticalAlignment = VerticalAlignment.Stretch,
            Visibility = Visibility.Collapsed
        };

        // Fallback Gradient Placeholder
        var placeholderBorder = new Border
        {
            CornerRadius = new CornerRadius(8),
            Background = new LinearGradientBrush
            {
                StartPoint = new Windows.Foundation.Point(0, 0),
                EndPoint = new Windows.Foundation.Point(1, 1),
                GradientStops =
                {
                    new GradientStop { Color = Color.FromArgb(255, 99, 102, 241), Offset = 0 }, // Indigo
                    new GradientStop { Color = Color.FromArgb(255, 168, 85, 247), Offset = 0.5 }, // Purple
                    new GradientStop { Color = Color.FromArgb(255, 236, 72, 153), Offset = 1 } // Pink
                }
            }
        };

        var placeholderContent = new StackPanel
        {
            VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Center,
            Spacing = 16
        };

        placeholderContent.Children.Add(new FontIcon
        {
            FontFamily = new FontFamily("Segoe MDL2 Assets"),
            FontSize = 56,
            Glyph = "\uE7B8", // Package/Box icon
            Foreground = new SolidColorBrush(Colors.White)
        });

        _welcomeTitle = new TextBlock
        {
            FontSize = 20,
            FontWeight = Microsoft.UI.Text.FontWeights.Bold,
            Foreground = new SolidColorBrush(Colors.White),
            TextAlignment = TextAlignment.Center,
            TextWrapping = TextWrapping.Wrap,
            MaxWidth = 220
        };
        placeholderContent.Children.Add(_welcomeTitle);
        placeholderBorder.Child = placeholderContent;
        _brandingPlaceholder = placeholderBorder;

        var brandingBorder = new Border
        {
            CornerRadius = new CornerRadius(8),
            Background = new SolidColorBrush(Color.FromArgb(15, 128, 128, 128)),
            BorderThickness = new Thickness(1),
            BorderBrush = new SolidColorBrush(Color.FromArgb(30, 128, 128, 128)),
            Child = new Grid
            {
                Children = { _brandingPlaceholder, _brandingImage }
            }
        };
        Grid.SetColumn(brandingBorder, 0);
        grid.Children.Add(brandingBorder);

        // Right Column: Information Panel
        var rightCol = new Grid
        {
            RowSpacing = 16
        };
        rightCol.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        rightCol.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        rightCol.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        // Row 0: Title and Subtitle
        var titlePanel = new StackPanel { Spacing = 4 };
        _welcomeHeaderTitle = new TextBlock
        {
            Text = "App Installer",
            FontSize = 24,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            TextWrapping = TextWrapping.Wrap
        };

        var subtitleText = new TextBlock
        {
            Text = "powered by Covenant Setup",
            FontSize = 12,
            Foreground = new SolidColorBrush(Color.FromArgb(180, 128, 128, 128)),
            FontStyle = Windows.UI.Text.FontStyle.Italic
        };
        titlePanel.Children.Add(_welcomeHeaderTitle);
        titlePanel.Children.Add(subtitleText);
        Grid.SetRow(titlePanel, 0);
        rightCol.Children.Add(titlePanel);

        // Row 1: Info and Path box
        var infoPanel = new StackPanel { Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        _welcomeInfoText = new TextBlock
        {
            Text = "This installer will install the application to the folder specified below:",
            TextWrapping = TextWrapping.Wrap,
            FontSize = 14
        };

        _welcomePathBorder = new Border
        {
            Background = new SolidColorBrush(Color.FromArgb(15, 128, 128, 128)),
            BorderBrush = new SolidColorBrush(Color.FromArgb(30, 128, 128, 128)),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(6),
            Padding = new Thickness(12),
            Margin = new Thickness(0, 4, 0, 0)
        };

        _welcomePathText = new TextBlock
        {
            FontFamily = new FontFamily("Consolas"),
            FontSize = 13,
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true
        };
        _welcomePathBorder.Child = _welcomePathText;

        infoPanel.Children.Add(_welcomeInfoText);
        infoPanel.Children.Add(_welcomePathBorder);
        Grid.SetRow(infoPanel, 1);
        rightCol.Children.Add(infoPanel);

        // Row 2: Buttons
        var welcomeButtons = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Spacing = 8
        };

        var welcomeCancelBtn = new Button
        {
            Content = "Cancel",
            MinWidth = 88
        };
        AutomationProperties.SetAutomationId(welcomeCancelBtn, "WelcomeCancelButton");
        welcomeCancelBtn.Click += (_, _) => OnWelcomeCancelClicked();

        var welcomeInstallBtn = new Button
        {
            Content = "Install",
            MinWidth = 88,
            Style = (Style)Microsoft.UI.Xaml.Application.Current.Resources["AccentButtonStyle"]
        };
        AutomationProperties.SetAutomationId(welcomeInstallBtn, "WelcomeInstallButton");
        welcomeInstallBtn.Click += (_, _) => OnWelcomeInstallClicked();

        welcomeButtons.Children.Add(welcomeCancelBtn);
        welcomeButtons.Children.Add(welcomeInstallBtn);
        Grid.SetRow(welcomeButtons, 2);
        rightCol.Children.Add(welcomeButtons);

        Grid.SetColumn(rightCol, 1);
        grid.Children.Add(rightCol);

        return grid;
    }

    private void ConfigureWindow()
    {
        var hwnd = WindowNative.GetWindowHandle(this);
        var windowId = Win32Interop.GetWindowIdFromWindow(hwnd);
        var appWindow = AppWindow.GetFromWindowId(windowId);
        appWindow.Title = Title;
        appWindow.Closing += (_, args) =>
        {
            if (_welcomeTcs != null && !_welcomeTcs.Task.IsCompleted)
            {
                _welcomeTcs.TrySetResult("cancel");
                _closeRequested = true;
                return;
            }

            if (!_canClose && !_closeRequested)
            {
                // Keep the window open through the rollback; SetCanClose(true)
                // on the terminal message makes X work again. RequestCancel
                // guards itself against firing twice.
                args.Cancel = true;
                RequestCancel();
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

    public Task<string> ShowWelcomeAsync(string appName, string installDir, string? brandingImage)
    {
        _welcomeTcs = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);

        BeginInvokeSafe(() =>
        {
            _welcomePanel.Visibility = Visibility.Visible;
            _progressPanel.Visibility = Visibility.Collapsed;

            _welcomeHeaderTitle.Text = $"{appName} Installer";
            // Manifests without file or directory targets have no install
            // root to show; the consent page still appears for them.
            var hasInstallDir = !string.IsNullOrWhiteSpace(installDir);
            _welcomeInfoText.Text = hasInstallDir
                ? $"This installer will install {appName} to:"
                : $"This installer will install {appName}.";
            _welcomePathText.Text = installDir;
            _welcomePathBorder.Visibility = hasInstallDir ? Visibility.Visible : Visibility.Collapsed;
            _welcomeTitle.Text = appName;

            if (!string.IsNullOrEmpty(brandingImage) && File.Exists(brandingImage))
            {
                try
                {
                    _brandingImage.Source = new Microsoft.UI.Xaml.Media.Imaging.BitmapImage(new Uri(brandingImage));
                    _brandingImage.Visibility = Visibility.Visible;
                    _brandingPlaceholder.Visibility = Visibility.Collapsed;
                }
                catch
                {
                    _brandingImage.Visibility = Visibility.Collapsed;
                    _brandingPlaceholder.Visibility = Visibility.Visible;
                }
            }
            else
            {
                _brandingImage.Visibility = Visibility.Collapsed;
                _brandingPlaceholder.Visibility = Visibility.Visible;
            }
        });

        return _welcomeTcs.Task;
    }

    private void OnWelcomeInstallClicked()
    {
        _welcomeTcs?.TrySetResult("install");
        BeginInvokeSafe(() =>
        {
            _welcomePanel.Visibility = Visibility.Collapsed;
            _progressPanel.Visibility = Visibility.Visible;
        });
    }

    private void OnWelcomeCancelClicked()
    {
        _welcomeTcs?.TrySetResult("cancel");
        _closeRequested = true;
        Close();
    }

    public void ShowInit(string title, string message)
    {
        BeginInvokeSafe(() =>
        {
            Title = title;
            _statusText.Text = message;
            _progressBar.Value = 0;
            _cancelButton.Content = "Cancel";
            _cancelButton.IsEnabled = true;

            _welcomePanel.Visibility = Visibility.Collapsed;
            _progressPanel.Visibility = Visibility.Visible;
        });
    }

    public void ShowProgress(int percent, string? message, int currentStep = 0)
    {
        BeginInvokeSafe(() =>
        {
            _currentStep = currentStep;
            if (!string.IsNullOrWhiteSpace(message))
            {
                _statusText.Text = message;
            }
            _progressBar.Value = percent;
        });
    }

    public void AppendLog(string message)
    {
        BeginInvokeSafe(() =>
        {
            if (string.IsNullOrWhiteSpace(message))
            {
                return;
            }

            if (_logText.Length > 0)
            {
                _logText.AppendLine();
            }
            _logText.Append(message);
            _logBox.Text = _logText.ToString();
            _logBox.SelectionStart = _logBox.Text.Length;
        });
    }

    public void ShowFinished(string message)
    {
        BeginInvokeSafe(() =>
        {
            // The window stays open showing the result; the user dismisses it
            // with the Close button (the engine waits for the process to exit).
            _statusText.Text = message;
            _progressBar.Value = 100;
            _cancelButton.Content = "Close";
            SetCanClose(true);
        });
    }

    public void ShowFailure(string failureMessage, string? errorDetails, string errataJson)
    {
        BeginInvokeSafe(() =>
        {
            var supportContact = ParseSupportContact(errataJson);
            _statusText.Text = FormatFailureMessage(failureMessage, supportContact);
            _progressBar.Value = 100;
            AppendLog(failureMessage);
            if (!string.IsNullOrWhiteSpace(errorDetails))
            {
                AppendLog("Error details: " + errorDetails);
            }
            if (!string.IsNullOrEmpty(supportContact))
            {
                AppendLog($"Support contact: {supportContact}");
            }

            _errataJson = errataJson;
            _failureMessage = failureMessage;
            _errorDetails = errorDetails;
            _saveErrataButton.IsEnabled = !string.IsNullOrWhiteSpace(_errataJson);
            _saveErrataButton.Visibility = Visibility.Visible;
            _copyErrorButton.IsEnabled = true;
            _copyErrorButton.Visibility = Visibility.Visible;
            _cancelButton.Content = "Close";
            SetCanClose(true);
        });
    }

    public Task<string> ShowPromptAsync(UiMessage message)
    {
        var tcs = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        if (_cancelRequested)
        {
            tcs.SetResult("none");
            return tcs.Task;
        }
        if (!DispatcherQueue.TryEnqueue(async () =>
        {
            try
            {
                if (_cancelRequested)
                {
                    tcs.SetResult("none");
                    return;
                }
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
                tcs.SetResult(MapDialogResult(result, buttons));
            }
            catch (Exception ex)
            {
                tcs.SetException(ex);
            }
        }))
        {
            tcs.SetResult("none");
        }
        return tcs.Task;
    }

    public void CloseView()
    {
        BeginInvokeSafe(() =>
        {
            _closeRequested = true;
            Close();
        });
    }

    public void ShowPipeError(string message)
    {
        BeginInvokeSafe(() =>
        {
            AppendLog("UI pipe error: " + message);
            _cancelButton.Content = "Close";
            SetCanClose(true);
        });
    }

    private void RequestCancel()
    {
        if (_cancelRequested)
        {
            return;
        }
        _cancelRequested = true;

        // Disable (don't hide) so the button cannot double-fire; the terminal
        // finish/fail message re-enables it as "Close" via SetCanClose(true).
        _cancelButton.IsEnabled = false;
        _statusText.Text = "Cancelling - reverting changes...";
        UiTrace.Write("cancel_requested", new { _currentStep });

        // The pipe stays open: the engine rolls back over the same session and
        // sends rollback progress followed by the terminal message.
        _sessionController?.RequestCancel();
    }

    internal static string BuildCopyText(string? failureMessage, string? errorDetails, string? errataJson)
    {
        var builder = new StringBuilder();
        if (!string.IsNullOrWhiteSpace(failureMessage))
        {
            builder.AppendLine(failureMessage);
        }
        if (!string.IsNullOrWhiteSpace(errorDetails))
        {
            builder.AppendLine("Error details: " + errorDetails);
        }
        if (!string.IsNullOrWhiteSpace(errataJson))
        {
            builder.AppendLine("Errata:");
            builder.AppendLine(errataJson);
        }
        return builder.ToString().TrimEnd();
    }

    private void CopyErrorToClipboard()
    {
        try
        {
            var text = BuildCopyText(_failureMessage, _errorDetails, _errataJson);
            if (string.IsNullOrEmpty(text))
            {
                return;
            }

            var package = new DataPackage();
            package.SetText(text);
            Clipboard.SetContent(package);
            AppendLog("Copied error details to clipboard");
        }
        catch (Exception ex)
        {
            UiTrace.Write("copy_error_failed", new { ex.Message, ex.GetType().FullName });
            AppendLog("Unable to copy error details: " + ex.Message);
        }
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

    internal static string? ParseSupportContact(string errataJson)
    {
        try
        {
            using var doc = JsonDocument.Parse(errataJson);
            if (doc.RootElement.ValueKind == JsonValueKind.Object &&
                doc.RootElement.TryGetProperty("support_contact", out var contactProp) &&
                contactProp.ValueKind == JsonValueKind.String)
            {
                return contactProp.GetString();
            }
        }
        catch (JsonException)
        {
        }
        return null;
    }

    internal static string FormatFailureMessage(string failureMessage, string? supportContact) =>
        string.IsNullOrEmpty(supportContact)
            ? failureMessage
            : $"{failureMessage}\n\nFor support, please contact: {supportContact}";

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
        _cancelButton.IsEnabled = canClose;
    }
}
