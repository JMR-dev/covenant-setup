using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Covenant.Setup.Ui;

internal enum PromptButtonSet
{
    Ok,
    OkCancel,
    YesNo
}

internal enum PromptIconKind
{
    Information,
    Warning,
    Error
}

internal enum PromptDialogResult
{
    Primary,
    Secondary,
    Close
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
