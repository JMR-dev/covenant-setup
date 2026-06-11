using System.Text.Json;
using Covenant.Setup.Ui;
using Xunit;

namespace Covenant.Setup.Ui.Tests;

public class InstallerUiWindowHelperTests
{
    [Fact]
    public void BuildErrataJson_uses_provided_errata_when_present()
    {
        using var doc = JsonDocument.Parse("""{"counter":42,"label":"alpha"}""");
        var msg = new UiMessage
        {
            AppName = "MyApp",
            Operation = "install",
            Message = "Failed",
            Error = "E_FAIL",
            Errata = doc.RootElement.Clone()
        };

        var json = InstallerUiWindow.BuildErrataJson(msg);

        using var parsed = JsonDocument.Parse(json);
        Assert.Equal(JsonValueKind.Object, parsed.RootElement.ValueKind);
        Assert.Equal(42, parsed.RootElement.GetProperty("counter").GetInt32());
        Assert.Equal("alpha", parsed.RootElement.GetProperty("label").GetString());
        Assert.False(parsed.RootElement.TryGetProperty("app_name", out _));
    }

    [Fact]
    public void BuildErrataJson_falls_back_to_synthesized_payload_when_errata_null()
    {
        var msg = new UiMessage
        {
            AppName = "MyApp",
            Operation = "install",
            Message = "Failed",
            Error = "E_FAIL",
            Errata = null
        };

        var json = InstallerUiWindow.BuildErrataJson(msg);

        using var parsed = JsonDocument.Parse(json);
        Assert.Equal("MyApp", parsed.RootElement.GetProperty("app_name").GetString());
        Assert.Equal("install", parsed.RootElement.GetProperty("operation").GetString());
        Assert.Equal("Failed", parsed.RootElement.GetProperty("message").GetString());
        Assert.Equal("E_FAIL", parsed.RootElement.GetProperty("error").GetString());
    }

    [Fact]
    public void BuildErrataJson_falls_back_when_errata_is_null_jsonelement()
    {
        using var doc = JsonDocument.Parse("null");
        var msg = new UiMessage
        {
            AppName = "MyApp",
            Operation = "uninstall",
            Errata = doc.RootElement.Clone()
        };

        var json = InstallerUiWindow.BuildErrataJson(msg);

        using var parsed = JsonDocument.Parse(json);
        Assert.Equal("MyApp", parsed.RootElement.GetProperty("app_name").GetString());
        Assert.Equal("uninstall", parsed.RootElement.GetProperty("operation").GetString());
    }

    [Fact]
    public void SafeMessageSummary_extracts_known_fields_from_valid_json()
    {
        const string line = """{"type":"progress","id":"x1","message":"Step 1","extra":"ignored"}""";

        var summary = InstallerUiWindow.SafeMessageSummary(line);
        var json = JsonSerializer.Serialize(summary);

        using var parsed = JsonDocument.Parse(json);
        Assert.Equal("progress", parsed.RootElement.GetProperty("Type").GetString());
        Assert.Equal("x1", parsed.RootElement.GetProperty("Id").GetString());
        Assert.Equal("Step 1", parsed.RootElement.GetProperty("Message").GetString());
    }

    [Fact]
    public void SafeMessageSummary_returns_raw_length_for_invalid_json()
    {
        var summary = InstallerUiWindow.SafeMessageSummary("not-json-at-all");
        var json = JsonSerializer.Serialize(summary);

        using var parsed = JsonDocument.Parse(json);
        Assert.Equal("not-json-at-all".Length, parsed.RootElement.GetProperty("RawLength").GetInt32());
        Assert.False(parsed.RootElement.TryGetProperty("Type", out _));
    }

    [Fact]
    public void SafeMessageSummary_returns_null_fields_when_known_keys_absent()
    {
        var summary = InstallerUiWindow.SafeMessageSummary("{}");
        var json = JsonSerializer.Serialize(summary);

        using var parsed = JsonDocument.Parse(json);
        Assert.Equal(JsonValueKind.Null, parsed.RootElement.GetProperty("Type").ValueKind);
        Assert.Equal(JsonValueKind.Null, parsed.RootElement.GetProperty("Id").ValueKind);
        Assert.Equal(JsonValueKind.Null, parsed.RootElement.GetProperty("Message").ValueKind);
    }

    [Theory]
    [InlineData("ok_cancel", 1)]
    [InlineData("yes_no", 2)]
    [InlineData("ok", 0)]
    [InlineData(null, 0)]
    [InlineData("unknown", 0)]
    public void MapButtons_handles_known_and_default_values(string? input, int expected)
    {
        Assert.Equal((PromptButtonSet)expected, InstallerUiWindow.MapButtons(input));
    }

    [Theory]
    [InlineData("error", 2)]
    [InlineData("warning", 1)]
    [InlineData("information", 0)]
    [InlineData(null, 0)]
    [InlineData("anything-else", 0)]
    public void MapIcon_handles_known_and_default_values(string? input, int expected)
    {
        Assert.Equal((PromptIconKind)expected, InstallerUiWindow.MapIcon(input));
    }

    [Theory]
    [InlineData(0, 0, "ok")]
    [InlineData(2, 0, "ok")]
    [InlineData(0, 1, "ok")]
    [InlineData(2, 1, "cancel")]
    [InlineData(0, 2, "yes")]
    [InlineData(1, 2, "no")]
    [InlineData(2, 2, "none")]
    public void MapDialogResult_maps_to_lowercase_token(
        int input,
        int buttons,
        string expected)
    {
        Assert.Equal(
            expected,
            InstallerUiWindow.MapDialogResult((PromptDialogResult)input, (PromptButtonSet)buttons));
    }
}
