using System.Text.Json;
using Covenant.Setup.Ui;
using Xunit;

namespace Covenant.Setup.Ui.Tests;

public class UiMessageJsonTests
{
    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNameCaseInsensitive = true
    };

    [Fact]
    public void Deserializes_progress_message_with_snake_case_step_fields()
    {
        const string json = """
        {"type":"progress","message":"Copying","current_step":3,"total_steps":10}
        """;

        var msg = JsonSerializer.Deserialize<UiMessage>(json, Options);

        Assert.NotNull(msg);
        Assert.Equal("progress", msg!.Type);
        Assert.Equal("Copying", msg.Message);
        Assert.Equal(3, msg.CurrentStep);
        Assert.Equal(10, msg.TotalSteps);
    }

    [Fact]
    public void Deserializes_fail_message_with_app_name_and_errata()
    {
        const string json = """
        {"type":"fail","app_name":"MyApp","operation":"install","message":"Boom","error":"E_FAIL","errata":{"k":1}}
        """;

        var msg = JsonSerializer.Deserialize<UiMessage>(json, Options);

        Assert.NotNull(msg);
        Assert.Equal("fail", msg!.Type);
        Assert.Equal("MyApp", msg.AppName);
        Assert.Equal("install", msg.Operation);
        Assert.Equal("Boom", msg.Message);
        Assert.Equal("E_FAIL", msg.Error);
        Assert.NotNull(msg.Errata);
        Assert.Equal(JsonValueKind.Object, msg.Errata!.Value.ValueKind);
    }

    [Fact]
    public void Deserializes_prompt_message_with_buttons_and_icon()
    {
        const string json = """
        {"type":"prompt","id":"p1","title":"Confirm","message":"Reboot now?","buttons":"yes_no","icon":"warning"}
        """;

        var msg = JsonSerializer.Deserialize<UiMessage>(json, Options);

        Assert.NotNull(msg);
        Assert.Equal("p1", msg!.Id);
        Assert.Equal("yes_no", msg.Buttons);
        Assert.Equal("warning", msg.Icon);
    }

    [Fact]
    public void Deserializes_message_with_unknown_type_to_arbitrary_string()
    {
        var msg = JsonSerializer.Deserialize<UiMessage>("""{"type":"unknown_type"}""", Options);
        Assert.Equal("unknown_type", msg!.Type);
    }

    [Fact]
    public void Missing_type_round_trips_as_null()
    {
        var msg = JsonSerializer.Deserialize<UiMessage>("{}", Options);
        Assert.NotNull(msg);
        Assert.Null(msg!.Type);
        Assert.Null(msg.CurrentStep);
        Assert.Null(msg.Errata);
    }

    [Fact]
    public void UiResponse_serializes_with_snake_case_property_names()
    {
        var response = new UiResponse { Type = "prompt_response", Id = "p1", Result = "yes" };
        var json = JsonSerializer.Serialize(response, Options);
        Assert.Contains("\"type\":\"prompt_response\"", json);
        Assert.Contains("\"id\":\"p1\"", json);
        Assert.Contains("\"result\":\"yes\"", json);
    }
}
