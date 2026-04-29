using System.IO.Pipes;
using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Covenant.Setup.Ui;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        var pipeName = ReadPipeName(args);
        UiTrace.Write("process_start", new { ProcessId = Environment.ProcessId, PipeName = pipeName });
        if (string.IsNullOrWhiteSpace(pipeName))
        {
            UiTrace.Write("missing_pipe_argument");
            MessageBox.Show("Missing named pipe argument.", "covenant-setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return;
        }

        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new InstallerUiForm(pipeName));
    }

    internal static string? ReadPipeName(string[] args)
    {
        for (var i = 0; i < args.Length - 1; i++)
        {
            if (string.Equals(args[i], "--pipe", StringComparison.OrdinalIgnoreCase))
            {
                var value = args[i + 1];
                const string pipePrefix = @"\\.\pipe\";
                if (value.StartsWith(pipePrefix, StringComparison.OrdinalIgnoreCase))
                {
                    value = value[pipePrefix.Length..];
                }
                return value;
            }
        }

        return null;
    }
}

internal sealed class InstallerUiForm : Form
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
    };

    private readonly string _pipeName;
    private readonly Label _statusLabel;
    private readonly ProgressBar _progressBar;
    private readonly TextBox _logBox;
    private readonly Button _saveErrataButton;
    private readonly Button _closeButton;
    private StreamWriter? _writer;
    private readonly object _writerLock = new();
    private bool _closeRequested;
    private string? _errataJson;

    public InstallerUiForm(string pipeName)
    {
        _pipeName = pipeName;

        Text = "covenant-setup";
        StartPosition = FormStartPosition.CenterScreen;
        ClientSize = new Size(720, 420);
        MinimumSize = new Size(560, 320);
        Font = new Font("Segoe UI", 9F);

        _statusLabel = new Label
        {
            AutoEllipsis = true,
            Location = new Point(12, 12),
            Size = new Size(ClientSize.Width - 24, 24),
            Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right,
            Text = "Preparing..."
        };

        _progressBar = new ProgressBar
        {
            Location = new Point(12, 44),
            Size = new Size(ClientSize.Width - 24, 24),
            Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right,
            Minimum = 0,
            Maximum = 100
        };

        _logBox = new TextBox
        {
            Location = new Point(12, 80),
            Size = new Size(ClientSize.Width - 24, ClientSize.Height - 128),
            Anchor = AnchorStyles.Top | AnchorStyles.Bottom | AnchorStyles.Left | AnchorStyles.Right,
            Multiline = true,
            ScrollBars = ScrollBars.Vertical,
            ReadOnly = true,
            Font = new Font("Consolas", 9F)
        };

        _saveErrataButton = new Button
        {
            Text = "Save error data to local errata.json file?",
            Enabled = false,
            Visible = false,
            Size = new Size(320, 28),
            Location = new Point(ClientSize.Width - 432, ClientSize.Height - 40),
            Anchor = AnchorStyles.Bottom | AnchorStyles.Right
        };
        _saveErrataButton.Click += (_, _) => SaveErrata();

        _closeButton = new Button
        {
            Text = "Close",
            Enabled = false,
            Size = new Size(88, 28),
            Location = new Point(ClientSize.Width - 100, ClientSize.Height - 40),
            Anchor = AnchorStyles.Bottom | AnchorStyles.Right
        };
        _closeButton.Click += (_, _) => Close();

        Controls.Add(_statusLabel);
        Controls.Add(_progressBar);
        Controls.Add(_logBox);
        Controls.Add(_saveErrataButton);
        Controls.Add(_closeButton);

        Shown += (_, _) => _ = Task.Run(RunPipeLoop);
        FormClosing += (_, args) =>
        {
            if (!_closeButton.Enabled && !_closeRequested)
            {
                args.Cancel = true;
            }
        };
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
                _closeButton.Enabled = true;
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
                    Text = message.Title ?? "covenant-setup";
                    _statusLabel.Text = message.Message ?? Text;
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
                    _statusLabel.Text = message.Message ?? "Complete";
                    _progressBar.Value = 100;
                    _closeButton.Enabled = true;
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
            _statusLabel.Text = message.Message;
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

        _statusLabel.Text = failureMessage;
        _progressBar.Value = 100;
        AppendLog(failureMessage);
        if (!string.IsNullOrWhiteSpace(message.Error))
        {
            AppendLog("Error details: " + message.Error);
        }

        _errataJson = BuildErrataJson(message);
        _saveErrataButton.Enabled = !string.IsNullOrWhiteSpace(_errataJson);
        _saveErrataButton.Visible = true;
        _closeButton.Enabled = true;
    }

    private void SaveErrata()
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
            MessageBox.Show(this, "Error data saved to " + path, "covenant-setup", MessageBoxButtons.OK, MessageBoxIcon.Information);
        }
        catch (Exception ex)
        {
            UiTrace.Write("errata_save_error", new { ex.Message, ex.GetType().FullName, ex.StackTrace });
            MessageBox.Show(this, "Unable to save errata.json: " + ex.Message, "covenant-setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
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
        if (InvokeRequired)
        {
            return (string)Invoke(new Func<string>(() => ShowPrompt(message)));
        }

        var buttons = MapButtons(message.Buttons);
        var icon = MapIcon(message.Icon);

        var result = MessageBox.Show(
            this,
            message.Message ?? string.Empty,
            message.Title ?? "covenant-setup",
            buttons,
            icon);
        UiTrace.Write("prompt_closed", new { message.Id, Result = result.ToString() });

        return MapDialogResult(result);
    }

    internal static MessageBoxButtons MapButtons(string? buttons) => buttons switch
    {
        "ok_cancel" => MessageBoxButtons.OKCancel,
        "yes_no" => MessageBoxButtons.YesNo,
        _ => MessageBoxButtons.OK
    };

    internal static MessageBoxIcon MapIcon(string? icon) => icon switch
    {
        "error" => MessageBoxIcon.Error,
        "warning" => MessageBoxIcon.Warning,
        _ => MessageBoxIcon.Information
    };

    internal static string MapDialogResult(DialogResult result) => result switch
    {
        DialogResult.OK => "ok",
        DialogResult.Cancel => "cancel",
        DialogResult.Yes => "yes",
        DialogResult.No => "no",
        _ => "none"
    };

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
        if (IsDisposed)
        {
            return;
        }

        try
        {
            BeginInvoke(action);
        }
        catch (InvalidOperationException)
        {
        }
    }

    private void AppendLog(string line)
    {
        if (string.IsNullOrWhiteSpace(line))
        {
            return;
        }

        if (_logBox.TextLength > 0)
        {
            _logBox.AppendText(Environment.NewLine);
        }
        _logBox.AppendText(line);
        _logBox.SelectionStart = _logBox.TextLength;
        _logBox.ScrollToCaret();
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

internal static class UiTrace
{
    private static readonly object Lock = new();
    private static readonly string? TracePath = CreateTracePath();

    public static void Write(string phase, object? detail = null)
    {
        if (TracePath is null)
        {
            return;
        }

        try
        {
            var line = JsonSerializer.Serialize(new
            {
                time = DateTimeOffset.UtcNow.ToString("o"),
                pid = Environment.ProcessId,
                process = Process.GetCurrentProcess().ProcessName,
                phase,
                detail
            }) + Environment.NewLine;
            lock (Lock)
            {
                File.AppendAllText(TracePath, line, Encoding.UTF8);
            }
        }
        catch
        {
        }
    }

    private static string? CreateTracePath()
    {
        try
        {
            var root = Environment.GetEnvironmentVariable("COVENANT_SETUP_TRACE_DIR");
            if (string.IsNullOrWhiteSpace(root))
            {
                return null;
            }

            Directory.CreateDirectory(root);
            return Path.Combine(root, $"csharp-ui-pipe-{Environment.ProcessId}.jsonl");
        }
        catch
        {
            return null;
        }
    }
}

internal sealed class UiMessage
{
    [JsonPropertyName("type")]
    public string? Type { get; set; }

    [JsonPropertyName("id")]
    public string? Id { get; set; }

    [JsonPropertyName("title")]
    public string? Title { get; set; }

    [JsonPropertyName("message")]
    public string? Message { get; set; }

    [JsonPropertyName("app_name")]
    public string? AppName { get; set; }

    [JsonPropertyName("operation")]
    public string? Operation { get; set; }

    [JsonPropertyName("error")]
    public string? Error { get; set; }

    [JsonPropertyName("errata")]
    public JsonElement? Errata { get; set; }

    [JsonPropertyName("current_step")]
    public int? CurrentStep { get; set; }

    [JsonPropertyName("total_steps")]
    public int? TotalSteps { get; set; }

    [JsonPropertyName("buttons")]
    public string? Buttons { get; set; }

    [JsonPropertyName("icon")]
    public string? Icon { get; set; }
}

internal sealed class UiResponse
{
    [JsonPropertyName("type")]
    public string? Type { get; set; }

    [JsonPropertyName("id")]
    public string? Id { get; set; }

    [JsonPropertyName("result")]
    public string? Result { get; set; }
}
