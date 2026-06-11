using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Covenant.Setup.Ui.Mocking;

internal sealed class MockEngineOptions
{
    public bool SkipDelays { get; init; }
    public bool StrictExpectations { get; init; } = true;
    public TimeSpan ConnectTimeout { get; init; } = TimeSpan.FromSeconds(10);
}

internal sealed class MockEngineClient(string pipeName, Scenario scenario, MockEngineOptions? options = null)
{
    private readonly MockEngineOptions _options = options ?? new();
    private readonly List<UiResponse> _responses = new();

    public IReadOnlyList<UiResponse> Responses => _responses;

    public async Task RunAsync(CancellationToken ct = default)
    {
        var skipDelays = _options.SkipDelays;
        var strictExpectations = _options.StrictExpectations;

        UiTrace.Write("mock_engine_start", new { PipeName = pipeName, Scenario = scenario.Name });

        using var pipe = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);

        using var connectCts = CancellationTokenSource.CreateLinkedTokenSource(ct);
        connectCts.CancelAfter(_options.ConnectTimeout);

        try
        {
            await pipe.ConnectAsync(connectCts.Token);
        }
        catch (Exception ex)
        {
            UiTrace.Write("mock_engine_connect_error", new { ex.Message });
            throw;
        }

        UiTrace.Write("mock_engine_connected", new { PipeName = pipeName });

        using var reader = new StreamReader(pipe, new UTF8Encoding(false), detectEncodingFromByteOrderMarks: false, bufferSize: 4096, leaveOpen: true);
        using (var writer = new StreamWriter(pipe, new UTF8Encoding(false), bufferSize: 4096, leaveOpen: true)
        {
            AutoFlush = true,
            NewLine = "\n"
        })
        {
            foreach (var step in scenario.Steps)
            {
                if (ct.IsCancellationRequested)
                {
                    break;
                }

                try
                {
                    if (step is SendStep sendStep)
                    {
                        UiTrace.Write("mock_engine_send", new { Line = sendStep.JsonLine });
                        await writer.WriteLineAsync(sendStep.JsonLine);
                    }
                    else if (step is DelayStep delayStep)
                    {
                        if (!skipDelays)
                        {
                            UiTrace.Write("mock_engine_delay", new { DelayMs = delayStep.Delay.TotalMilliseconds });
                            await Task.Delay(delayStep.Delay, ct);
                        }
                    }
                    else if (step is AwaitResponseStep awaitStep)
                    {
                        UiTrace.Write("mock_engine_await_response", new { awaitStep.Id, awaitStep.Expect });
                        string? responseLine = await reader.ReadLineAsync(ct);
                        if (responseLine is null)
                        {
                            throw new ScenarioAssertionException($"EOF reached on pipe while awaiting response for prompt '{awaitStep.Id}'.");
                        }

                        UiTrace.Write("mock_engine_response_received", new { Line = responseLine });
                        var response = JsonSerializer.Deserialize<UiResponse>(responseLine, UiJson.Options);
                        if (response is null)
                        {
                            throw new ScenarioAssertionException($"Received unexpected null response.");
                        }

                        if (response.Type == "welcome_response")
                        {
                            if (awaitStep.Id != "welcome")
                            {
                                throw new ScenarioAssertionException($"Expected welcome_response, but await step ID is '{awaitStep.Id}'.");
                            }
                        }
                        else if (response.Type == "prompt_response")
                        {
                            if (response.Id != awaitStep.Id)
                            {
                                throw new ScenarioAssertionException($"Prompt ID mismatch. Expected '{awaitStep.Id}', got '{response.Id}'.");
                            }
                        }
                        else if (response.Type == "cancel_request")
                        {
                            if (awaitStep.Id != "cancel")
                            {
                                throw new ScenarioAssertionException($"Expected cancel_request, but await step ID is '{awaitStep.Id}'.");
                            }
                        }
                        else
                        {
                            throw new ScenarioAssertionException($"Received unexpected message shape. Expected prompt_response, welcome_response, or cancel_request, got: {responseLine}");
                        }

                        _responses.Add(response);

                        if (awaitStep.Expect is not null && !string.Equals(response.Result, awaitStep.Expect, StringComparison.OrdinalIgnoreCase))
                        {
                            var errorMsg = $"Prompt '{awaitStep.Id}' result mismatch. Expected '{awaitStep.Expect}', got '{response.Result}'.";
                            if (strictExpectations)
                            {
                                throw new ScenarioAssertionException(errorMsg);
                            }
                            else
                            {
                                UiTrace.Write("mock_engine_assertion_warning", new { Message = errorMsg });
                            }
                        }
                    }
                }
                catch (Exception ex) when (!strictExpectations && (ex is IOException || ex is ObjectDisposedException))
                {
                    UiTrace.Write("mock_engine_broken_pipe_graceful_exit", new { ex.Message });
                    break;
                }
            }
        }

        try
        {
            // Wait for the server (UI) to close the pipe connection to ensure it has fully processed
            // all sent steps and exited gracefully without encountering a premature broken pipe.
            while (await reader.ReadLineAsync(ct) is not null) { }
        }
        catch
        {
            // Ignore any pipe closure exceptions on shutdown
        }
    }
}

internal sealed class ScenarioAssertionException(string message) : Exception(message);
