using System;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using Covenant.Setup.Ui;
using Covenant.Setup.Ui.Mocking;
using Xunit;

namespace Covenant.Setup.Ui.Tests;

public class MockEngineEndToEndTests
{
    [Fact]
    public async Task e2e_happy_path_ordering_and_execution()
    {
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var pipeName = $"covenant-setup-test-e2e-{Guid.NewGuid():N}";
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController(pipeName, view);

        var controllerTask = Task.Run(() => controller.Run());

        var scenario = new ScenarioBuilder("happy-builder")
            .Init("Title", "Message")
            .Progress(1, 2, "Step 1")
            .Finish("Complete")
            .Close()
            .Build();

        var client = new MockEngineClient(pipeName, scenario, new MockEngineOptions
        {
            SkipDelays = true,
            StrictExpectations = true
        });

        await client.RunAsync(cts.Token);
        await controllerTask;

        Assert.Equal(5, view.Calls.Count);
        Assert.Equal("ShowInit: title=Title, message=Message", view.Calls[0]);
        Assert.Equal("AppendLog: message=Step 1", view.Calls[1]);
        Assert.Equal("ShowProgress: percent=50, message=Step 1", view.Calls[2]);
        Assert.Equal("ShowFinished: message=Complete", view.Calls[3]);
        Assert.Equal("CloseView", view.Calls[4]);
    }

    [Fact]
    public async Task e2e_prompt_flow_recorded_and_checked()
    {
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var pipeName = $"covenant-setup-test-prompt-{Guid.NewGuid():N}";
        var view = new FakeInstallerView { PromptResponder = msg => "yes" };
        var controller = new InstallerSessionController(pipeName, view);

        var controllerTask = Task.Run(() => controller.Run());

        var scenario = new ScenarioBuilder("prompt-builder")
            .Prompt("p1", "Prompt", "Msg", "yes_no", "info")
            .AwaitPromptResponse("p1", "yes")
            .Close()
            .Build();

        var client = new MockEngineClient(pipeName, scenario, new MockEngineOptions
        {
            SkipDelays = true,
            StrictExpectations = true
        });

        await client.RunAsync(cts.Token);
        await controllerTask;

        Assert.Single(client.Responses);
        Assert.Equal("p1", client.Responses[0].Id);
        Assert.Equal("yes", client.Responses[0].Result);
    }

    [Fact]
    public async Task e2e_strict_expectations_throws_on_mismatch()
    {
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var pipeName = $"covenant-setup-test-mismatch-{Guid.NewGuid():N}";
        var view = new FakeInstallerView { PromptResponder = msg => "no" };
        var controller = new InstallerSessionController(pipeName, view);

        var controllerTask = Task.Run(() => controller.Run());

        var scenario = new ScenarioBuilder("prompt-builder")
            .Prompt("p1", "Prompt", "Msg", "yes_no", "info")
            .AwaitPromptResponse("p1", "yes")
            .Close()
            .Build();

        var client = new MockEngineClient(pipeName, scenario, new MockEngineOptions
        {
            SkipDelays = true,
            StrictExpectations = true
        });

        var ex = await Assert.ThrowsAsync<ScenarioAssertionException>(() => client.RunAsync(cts.Token));
        Assert.Contains("Prompt 'p1' result mismatch", ex.Message);
    }

    [Fact]
    public async Task e2e_non_strict_expectations_warns_and_continues()
    {
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var pipeName = $"covenant-setup-test-warn-{Guid.NewGuid():N}";
        var view = new FakeInstallerView { PromptResponder = msg => "no" };
        var controller = new InstallerSessionController(pipeName, view);

        var controllerTask = Task.Run(() => controller.Run());

        var scenario = new ScenarioBuilder("prompt-builder")
            .Prompt("p1", "Prompt", "Msg", "yes_no", "info")
            .AwaitPromptResponse("p1", "yes")
            .Close()
            .Build();

        var client = new MockEngineClient(pipeName, scenario, new MockEngineOptions
        {
            SkipDelays = true,
            StrictExpectations = false
        });

        await client.RunAsync(cts.Token);
        await controllerTask;

        Assert.Single(client.Responses);
        Assert.Equal("no", client.Responses[0].Result);
    }

    [Fact]
    public async Task e2e_cancel_request_round_trip_shows_rollback_then_finish()
    {
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var pipeName = $"covenant-setup-test-cancel-{Guid.NewGuid():N}";
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController(pipeName, view);

        var controllerTask = Task.Run(() => controller.Run());

        var scenario = new ScenarioBuilder("cancel-builder")
            .Init("Installing SampleApp", "Preparing...")
            .Progress(1, 8, "Copying files")
            .AwaitCancelRequest()
            .Log("Cancel requested - reverting changes...")
            .Progress(1, 2, "Removing file")
            .Progress(2, 2, "Removing directory")
            .Finish("SampleApp installation cancelled. All changes were reverted.")
            .Build();

        var client = new MockEngineClient(pipeName, scenario, new MockEngineOptions
        {
            SkipDelays = true,
            StrictExpectations = true
        });

        var clientTask = client.RunAsync(cts.Token);

        // Simulate the user clicking Cancel; RequestCancel succeeds once the
        // pipe is connected, and the client picks it up at AwaitCancelRequest.
        while (!controller.RequestCancel())
        {
            await Task.Delay(10, cts.Token);
        }

        await clientTask;
        await controllerTask;

        Assert.Contains(client.Responses, r => r.Type == "cancel_request");
        Assert.Contains("AppendLog: message=Cancel requested - reverting changes...", view.Calls);
        Assert.Contains("ShowProgress: percent=100, message=Removing directory", view.Calls);
        Assert.Contains("ShowFinished: message=SampleApp installation cancelled. All changes were reverted.", view.Calls);
        Assert.DoesNotContain("CloseView", view.Calls);
    }

    [Theory]
    [InlineData("install-happy")]
    [InlineData("install-fail-errata")]
    [InlineData("uninstall-reboot-prompt")]
    public async Task e2e_play_canned_scenarios(string name)
    {
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var pipeName = $"covenant-setup-test-canned-{name}-{Guid.NewGuid():N}";

        var view = new FakeInstallerView
        {
            PromptResponder = msg => msg.Buttons == "yes_no" ? "yes" : "ok"
        };
        var controller = new InstallerSessionController(pipeName, view);

        var controllerTask = Task.Run(() => controller.Run());

        var scenario = Scenario.LoadFile(name);
        var client = new MockEngineClient(pipeName, scenario, new MockEngineOptions
        {
            SkipDelays = true,
            StrictExpectations = true
        });

        await client.RunAsync(cts.Token);
        await controllerTask;

        Assert.NotEmpty(view.Calls);
    }
}
