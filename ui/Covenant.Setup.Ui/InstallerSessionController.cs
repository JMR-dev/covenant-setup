using System.IO.Pipes;
using System.Text;
using System.Text.Json;

namespace Covenant.Setup.Ui;

internal sealed class InstallerSessionController(string pipeName, IInstallerView view)
{
    private readonly object _writerLock = new();
    private TextWriter? _writer;
    private NamedPipeServerStream? _pipe;

    internal TextWriter? ResponseWriter
    {
        get
        {
            lock (_writerLock)
            {
                return _writer;
            }
        }
        set
        {
            lock (_writerLock)
            {
                _writer = value;
            }
        }
    }

    /// <summary>
    /// Asks the engine to stop the current operation and roll back. The pipe
    /// stays open so rollback progress and the terminal message still arrive.
    /// </summary>
    public bool RequestCancel()
    {
        if (!WriteResponse(new UiResponse { Type = "cancel_request" }))
        {
            return false;
        }

        UiTrace.Write("cancel_request_sent");
        return true;
    }

    public void Run()
    {
        try
        {
            UiTrace.Write("pipe_server_create", new { PipeName = pipeName });
            // Asynchronous (overlapped) mode is required: with a synchronous
            // handle Windows serializes I/O, so RequestCancel's write from the
            // UI thread would block behind the message loop's pending ReadLine.
            using (_pipe = new NamedPipeServerStream(
                pipeName,
                PipeDirection.InOut,
                1,
                PipeTransmissionMode.Byte,
                PipeOptions.Asynchronous))
            {
                UiTrace.Write("pipe_wait_for_connection", new { PipeName = pipeName });
                _pipe.WaitForConnection();
                UiTrace.Write("pipe_connected", new { PipeName = pipeName });

                using var reader = new StreamReader(_pipe, new UTF8Encoding(false), detectEncodingFromByteOrderMarks: false, bufferSize: 4096, leaveOpen: true);
                using var writer = new StreamWriter(_pipe, new UTF8Encoding(false), bufferSize: 4096, leaveOpen: true)
                {
                    AutoFlush = true,
                    NewLine = "\n"
                };

                ResponseWriter = writer;

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
        }
        catch (Exception ex)
        {
            UiTrace.Write("pipe_error", new { ex.Message, ex.GetType().FullName, ex.StackTrace });
            view.ShowPipeError(ex.Message);
        }
        finally
        {
            ResponseWriter = null;
            UiTrace.Write("pipe_loop_exit");
        }
    }

    internal bool HandleMessage(string line)
    {
        var message = JsonSerializer.Deserialize<UiMessage>(line, UiJson.Options);
        if (message?.Type is null)
        {
            return true;
        }

        switch (message.Type)
        {
            case "init":
                var title = message.Title ?? "covenant-setup";
                // The engine's explicit show_welcome flag is the single source
                // of truth for the handshake; it awaits a welcome_response
                // exactly when it sent the flag.
                if (message.ShowWelcome == true)
                {
                    UiTrace.Write("welcome_show_requested", new { message.AppName, message.InstallDir });
                    var welcomeResult = view.ShowWelcomeAsync(
                        string.IsNullOrEmpty(message.AppName) ? title : message.AppName,
                        message.InstallDir ?? string.Empty,
                        message.BrandingImage).GetAwaiter().GetResult();
                    UiTrace.Write("welcome_response", new { Result = welcomeResult });

                    WriteResponse(new UiResponse
                    {
                        Type = "welcome_response",
                        Result = welcomeResult
                    });

                    if (welcomeResult == "cancel")
                    {
                        view.CloseView();
                        return false;
                    }
                }
                view.ShowInit(title, message.Message ?? title);
                return true;

            case "progress":
                if (!string.IsNullOrWhiteSpace(message.Message))
                {
                    view.AppendLog(message.Message);
                }
                view.ShowProgress(ComputePercent(message.CurrentStep, message.TotalSteps), message.Message, message.CurrentStep ?? 0);
                return true;

            case "log":
                view.AppendLog(message.Message ?? string.Empty);
                return true;

            case "finish":
                UiTrace.Write("finish_message", new { message.Message });
                view.ShowFinished(message.Message ?? "Complete");
                return true;

            case "fail":
                UiTrace.Write("fail_message", new { message.AppName, message.Operation, message.Message, message.Error });
                view.ShowFailure(BuildFailureMessage(message), message.Error, BuildErrataJson(message));
                return true;

            case "prompt":
                UiTrace.Write("prompt_show_requested", new { message.Id, message.Title, message.Buttons, message.Icon });
                var result = view.ShowPromptAsync(message).GetAwaiter().GetResult();
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
                view.CloseView();
                return false;

            default:
                return true;
        }
    }

    internal static int ComputePercent(int? currentStep, int? totalSteps)
    {
        var total = Math.Max(1, totalSteps ?? 1);
        var current = Math.Max(0, Math.Min(total, currentStep ?? 0));
        return Math.Max(0, Math.Min(100, current * 100 / total));
    }

    internal static string BuildFailureMessage(UiMessage message)
    {
        var operation = string.IsNullOrWhiteSpace(message.Operation) ? "complete" : message.Operation;
        var appName = string.IsNullOrWhiteSpace(message.AppName) ? "unknown" : message.AppName;
        return string.IsNullOrWhiteSpace(message.Message)
            ? $"Error: program {appName} failed to {operation} completely!"
            : message.Message;
    }

    internal static string BuildErrataJson(UiMessage message)
    {
        if (message.Errata is JsonElement errata &&
            errata.ValueKind is not JsonValueKind.Undefined and not JsonValueKind.Null)
        {
            return JsonSerializer.Serialize(errata, UiJson.Indented);
        }

        return JsonSerializer.Serialize(new
        {
            app_name = message.AppName,
            operation = message.Operation,
            message = message.Message,
            error = message.Error
        }, UiJson.Indented);
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

    private bool WriteResponse(UiResponse response)
    {
        lock (_writerLock)
        {
            if (_writer is null)
            {
                return false;
            }

            _writer.WriteLine(JsonSerializer.Serialize(response, UiJson.Options));
            UiTrace.Write("pipe_send", new { response.Type, response.Id, response.Result });
            return true;
        }
    }
}
