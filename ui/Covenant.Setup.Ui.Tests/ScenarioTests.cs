using System;
using System.IO;
using System.Linq;
using System.Text.Json;
using Covenant.Setup.Ui.Mocking;
using Xunit;

namespace Covenant.Setup.Ui.Tests;

public class ScenarioTests
{
    [Fact]
    public void parser_skips_blanks_and_comments()
    {
        var lines = new[]
        {
            "# This is a comment",
            "",
            "   ",
            "{\"type\":\"log\",\"message\":\"hello\"}",
            "# Another comment"
        };

        var scenario = Scenario.Parse("test", lines);
        Assert.Single(scenario.Steps);
        var sendStep = Assert.IsType<SendStep>(scenario.Steps[0]);
        Assert.Contains("\"type\":\"log\"", sendStep.JsonLine);
    }

    [Fact]
    public void parser_dispatches_correct_steps()
    {
        var lines = new[]
        {
            "{\"type\":\"init\",\"title\":\"install\"}",
            "{\"wait_ms\":250}",
            "{\"await_response\":{\"id\":\"p1\",\"expect\":\"ok\"}}"
        };

        var scenario = Scenario.Parse("test", lines);
        Assert.Equal(3, scenario.Steps.Count);

        var send = Assert.IsType<SendStep>(scenario.Steps[0]);
        Assert.Contains("\"type\":\"init\"", send.JsonLine);

        var delay = Assert.IsType<DelayStep>(scenario.Steps[1]);
        Assert.Equal(TimeSpan.FromMilliseconds(250), delay.Delay);

        var awaitResponse = Assert.IsType<AwaitResponseStep>(scenario.Steps[2]);
        Assert.Equal("p1", awaitResponse.Id);
        Assert.Equal("ok", awaitResponse.Expect);
    }

    [Fact]
    public void parser_throws_format_error_with_line_numbers()
    {
        var lines = new[]
        {
            "{\"type\":\"init\"}",
            "not-json",
            "{\"wait_ms\":250}"
        };

        var ex = Assert.Throws<ScenarioFormatException>(() => Scenario.Parse("test", lines));
        Assert.Contains("Line 2", ex.Message);
    }

    [Fact]
    public void parser_throws_for_unknown_shape()
    {
        var lines = new[]
        {
            "{\"unknown_key\":\"value\"}"
        };

        var ex = Assert.Throws<ScenarioFormatException>(() => Scenario.Parse("test", lines));
        Assert.Contains("Unknown step shape", ex.Message);
    }

    [Fact]
    public void resolve_path_handles_bare_names_and_paths()
    {
        var barePath = Scenario.ResolvePath("install-happy");
        Assert.Contains("Scenarios", barePath);
        Assert.EndsWith("install-happy.jsonl", barePath);

        var relativePath = Scenario.ResolvePath($"ui{Path.DirectorySeparatorChar}test.jsonl");
        Assert.EndsWith($"ui{Path.DirectorySeparatorChar}test.jsonl", relativePath);
    }

    [Fact]
    public void builder_roundtrips_through_json()
    {
        var scenario = new ScenarioBuilder("test-builder")
            .Init("Title", "Message")
            .Progress(1, 10, "Working")
            .Log("Hello")
            .Finish("Done")
            .Fail("AppName", "Op", "ErrMsg", "ErrDetails", "{\"custom_key\":123}")
            .Prompt("p1", "PromptTitle", "PromptMessage", "ok_cancel", "warning")
            .AwaitPromptResponse("p1", "ok")
            .Delay(TimeSpan.FromMilliseconds(100))
            .Close()
            .Build();

        Assert.Equal(9, scenario.Steps.Count);

        var initStep = Assert.IsType<SendStep>(scenario.Steps[0]);
        var initMsg = JsonSerializer.Deserialize<UiMessage>(initStep.JsonLine);
        Assert.Equal("init", initMsg!.Type);
        Assert.Equal("Title", initMsg.Title);
        Assert.Equal("Message", initMsg.Message);

        var failStep = Assert.IsType<SendStep>(scenario.Steps[4]);
        var failMsg = JsonSerializer.Deserialize<UiMessage>(failStep.JsonLine);
        Assert.Equal("fail", failMsg!.Type);
        Assert.Equal("AppName", failMsg.AppName);
        Assert.Equal("Op", failMsg.Operation);
        Assert.Equal("ErrMsg", failMsg.Message);
        Assert.Equal("ErrDetails", failMsg.Error);
        Assert.Equal(JsonValueKind.Object, failMsg.Errata!.Value.ValueKind);
        Assert.Equal(123, failMsg.Errata.Value.GetProperty("custom_key").GetInt32());

        var promptStep = Assert.IsType<SendStep>(scenario.Steps[5]);
        var promptMsg = JsonSerializer.Deserialize<UiMessage>(promptStep.JsonLine);
        Assert.Equal("prompt", promptMsg!.Type);
        Assert.Equal("p1", promptMsg.Id);
        Assert.Equal("ok_cancel", promptMsg.Buttons);
        Assert.Equal("warning", promptMsg.Icon);

        var awaitStep = Assert.IsType<AwaitResponseStep>(scenario.Steps[6]);
        Assert.Equal("p1", awaitStep.Id);
        Assert.Equal("ok", awaitStep.Expect);

        var delayStep = Assert.IsType<DelayStep>(scenario.Steps[7]);
        Assert.Equal(TimeSpan.FromMilliseconds(100), delayStep.Delay);

        var closeStep = Assert.IsType<SendStep>(scenario.Steps[8]);
        var closeMsg = JsonSerializer.Deserialize<UiMessage>(closeStep.JsonLine);
        Assert.Equal("close", closeMsg!.Type);
    }

    [Fact]
    public void builder_await_cancel_request_emits_cancel_await_step()
    {
        var scenario = new ScenarioBuilder("cancel-builder")
            .Progress(1, 8, "Copying files")
            .AwaitCancelRequest()
            .Finish("Cancelled")
            .Build();

        var awaitStep = Assert.IsType<AwaitResponseStep>(scenario.Steps[1]);
        Assert.Equal("cancel", awaitStep.Id);
        Assert.Null(awaitStep.Expect);
    }

    [Theory]
    [InlineData("install-happy")]
    [InlineData("install-prompt")]
    [InlineData("install-fail-errata")]
    [InlineData("install-cancel-rollback")]
    [InlineData("uninstall-happy")]
    [InlineData("uninstall-reboot-prompt")]
    [InlineData("install-slow")]
    public void canned_scenarios_load_and_contain_matching_awaits(string name)
    {
        var scenario = Scenario.LoadFile(name);
        Assert.NotNull(scenario);
        Assert.NotEmpty(scenario.Steps);

        for (int i = 0; i < scenario.Steps.Count; i++)
        {
            if (scenario.Steps[i] is SendStep sendStep)
            {
                using var doc = JsonDocument.Parse(sendStep.JsonLine);
                var root = doc.RootElement;
                if (root.TryGetProperty("type", out var typeProp) && typeProp.GetString() == "prompt")
                {
                    var id = root.GetProperty("id").GetString();
                    Assert.True(i + 1 < scenario.Steps.Count, $"Prompt '{id}' is the last step in the scenario and has no await_response.");
                    var nextStep = scenario.Steps[i + 1];
                    var awaitStep = Assert.IsType<AwaitResponseStep>(nextStep);
                    Assert.Equal(id, awaitStep.Id);
                }
            }
        }
    }
}
