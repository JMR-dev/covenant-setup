using System;
using System.IO;
using System.Text.Json;
using Covenant.Setup.Ui;
using Xunit;

namespace Covenant.Setup.Ui.Tests;

public class InstallerSessionControllerTests
{
    [Fact]
    public void HandleMessage_init_calls_ShowInit()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"init\",\"title\":\"My Title\",\"message\":\"My Message\"}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.Equal("ShowInit: title=My Title, message=My Message", view.Calls[0]);
    }

    [Fact]
    public void HandleMessage_init_falls_back_to_defaults()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"init\"}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.Equal("ShowInit: title=covenant-setup, message=covenant-setup", view.Calls[0]);
    }

    [Theory]
    [InlineData(1, 10, 10)]
    [InlineData(5, 10, 50)]
    [InlineData(10, 10, 100)]
    [InlineData(12, 10, 100)]
    [InlineData(-1, 10, 0)]
    [InlineData(5, 0, 100)]
    [InlineData(5, null, 100)]
    [InlineData(null, null, 0)]
    public void ComputePercent_clamps_and_calculates_correctly(int? current, int? total, int expected)
    {
        var percent = InstallerSessionController.ComputePercent(current, total);
        Assert.Equal(expected, percent);
    }

    [Fact]
    public void HandleMessage_progress_calls_ShowProgress_and_AppendLog()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"progress\",\"current_step\":2,\"total_steps\":5,\"message\":\"Step msg\"}");

        Assert.True(ok);
        Assert.Equal(2, view.Calls.Count);
        Assert.Equal("AppendLog: message=Step msg", view.Calls[0]);
        Assert.Equal("ShowProgress: percent=40, message=Step msg", view.Calls[1]);
    }

    [Fact]
    public void HandleMessage_progress_omits_log_when_message_empty()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"progress\",\"current_step\":2,\"total_steps\":5}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.Equal("ShowProgress: percent=40, message=", view.Calls[0]);
    }

    [Fact]
    public void HandleMessage_log_calls_AppendLog()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"log\",\"message\":\"test log message\"}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.Equal("AppendLog: message=test log message", view.Calls[0]);
    }

    [Fact]
    public void HandleMessage_finish_calls_ShowFinished()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"finish\",\"message\":\"Successful installation!\"}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.Equal("ShowFinished: message=Successful installation!", view.Calls[0]);
    }

    [Fact]
    public void HandleMessage_fail_calls_ShowFailure_with_errata()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"fail\",\"app_name\":\"TestApp\",\"operation\":\"copy\",\"message\":\"Failed copy\",\"error\":\"E_OUTOFMEMORY\"}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.StartsWith("ShowFailure: failureMessage=Failed copy, errorDetails=E_OUTOFMEMORY", view.Calls[0]);
        Assert.Contains("\"app_name\": \"TestApp\"", view.Calls[0]);
        Assert.Contains("\"operation\": \"copy\"", view.Calls[0]);
    }

    [Fact]
    public void HandleMessage_fail_passes_support_contact_through_errata()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"fail\",\"app_name\":\"TestApp\",\"operation\":\"install\",\"message\":\"Failed install\",\"error\":\"E_FAIL\",\"errata\":{\"app_name\":\"TestApp\",\"support_contact\":\"support@example.com\"}}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.StartsWith("ShowFailure: failureMessage=Failed install, errorDetails=E_FAIL", view.Calls[0]);
        Assert.Contains("\"support_contact\": \"support@example.com\"", view.Calls[0]);
    }

    [Theory]
    [InlineData("{\"support_contact\":\"support@example.com\"}", "support@example.com")]
    [InlineData("{\"app_name\":\"TestApp\",\"support_contact\":\"1-800-555-0100\"}", "1-800-555-0100")]
    [InlineData("{\"support_contact\":null}", null)]
    [InlineData("{\"support_contact\":42}", null)]
    [InlineData("{\"app_name\":\"TestApp\"}", null)]
    [InlineData("[1,2,3]", null)]
    [InlineData("not-json", null)]
    public void ParseSupportContact_extracts_string_value_only(string errataJson, string? expected)
    {
        Assert.Equal(expected, InstallerUiWindow.ParseSupportContact(errataJson));
    }

    [Fact]
    public void FormatFailureMessage_appends_support_contact_when_present()
    {
        var formatted = InstallerUiWindow.FormatFailureMessage("Error: program TestApp failed to install completely!", "support@example.com");

        Assert.Equal(
            "Error: program TestApp failed to install completely!\n\nFor support, please contact: support@example.com",
            formatted);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    public void FormatFailureMessage_returns_message_unchanged_without_contact(string? supportContact)
    {
        Assert.Equal("Failed install", InstallerUiWindow.FormatFailureMessage("Failed install", supportContact));
    }

    [Fact]
    public void HandleMessage_prompt_shows_prompt_and_writes_response()
    {
        var view = new FakeInstallerView { PromptResponder = msg => "yes" };
        var controller = new InstallerSessionController("dummy", view);
        using var sw = new StringWriter();
        controller.ResponseWriter = sw;

        var ok = controller.HandleMessage("{\"type\":\"prompt\",\"id\":\"p1\",\"title\":\"Ask\",\"buttons\":\"yes_no\",\"icon\":\"question\"}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.Equal("ShowPromptAsync: id=p1, title=Ask, buttons=yes_no, icon=question", view.Calls[0]);

        var written = sw.ToString().Trim();
        using var responseDoc = JsonDocument.Parse(written);
        var root = responseDoc.RootElement;
        Assert.Equal("prompt_response", root.GetProperty("type").GetString());
        Assert.Equal("p1", root.GetProperty("id").GetString());
        Assert.Equal("yes", root.GetProperty("result").GetString());
    }

    [Fact]
    public void HandleMessage_close_calls_CloseView_and_returns_false()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"close\"}");

        Assert.False(ok);
        Assert.Single(view.Calls);
        Assert.Equal("CloseView", view.Calls[0]);
    }

    [Fact]
    public void HandleMessage_unknown_type_returns_true_no_view_calls()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        var ok = controller.HandleMessage("{\"type\":\"unknown_message_type\"}");

        Assert.True(ok);
        Assert.Empty(view.Calls);
    }

    [Fact]
    public void RequestCancel_writes_cancel_request_and_keeps_writer_open()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);
        using var sw = new StringWriter();
        controller.ResponseWriter = sw;

        var sent = controller.RequestCancel();

        Assert.True(sent);
        Assert.NotNull(controller.ResponseWriter);
        var written = sw.ToString().Trim();
        using var responseDoc = JsonDocument.Parse(written);
        Assert.Equal("cancel_request", responseDoc.RootElement.GetProperty("type").GetString());
    }

    [Fact]
    public void RequestCancel_returns_false_without_writer()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);

        Assert.False(controller.RequestCancel());
    }

    [Fact]
    public void HandleMessage_init_with_show_welcome_shows_welcome_and_proceeds_on_install()
    {
        var view = new FakeInstallerView { WelcomeResponder = (app, dir, img) => "install" };
        var controller = new InstallerSessionController("dummy", view);
        using var sw = new StringWriter();
        controller.ResponseWriter = sw;

        var ok = controller.HandleMessage("{\"type\":\"init\",\"show_welcome\":true,\"title\":\"My Title\",\"app_name\":\"TestApp\",\"install_dir\":\"C:\\\\Test\",\"branding_image\":\"img.png\"}");

        Assert.True(ok);
        Assert.Equal(2, view.Calls.Count);
        Assert.Equal("ShowWelcomeAsync: appName=TestApp, installDir=C:\\Test, brandingImage=img.png", view.Calls[0]);
        Assert.Equal("ShowInit: title=My Title, message=My Title", view.Calls[1]);

        var written = sw.ToString().Trim();
        using var responseDoc = JsonDocument.Parse(written);
        var root = responseDoc.RootElement;
        Assert.Equal("welcome_response", root.GetProperty("type").GetString());
        Assert.Equal("install", root.GetProperty("result").GetString());
    }

    [Fact]
    public void HandleMessage_init_with_show_welcome_shows_welcome_and_cancels()
    {
        var view = new FakeInstallerView { WelcomeResponder = (app, dir, img) => "cancel" };
        var controller = new InstallerSessionController("dummy", view);
        using var sw = new StringWriter();
        controller.ResponseWriter = sw;

        var ok = controller.HandleMessage("{\"type\":\"init\",\"show_welcome\":true,\"title\":\"My Title\",\"app_name\":\"TestApp\",\"install_dir\":\"C:\\\\Test\",\"branding_image\":\"img.png\"}");

        Assert.False(ok);
        Assert.Equal(2, view.Calls.Count);
        Assert.Equal("ShowWelcomeAsync: appName=TestApp, installDir=C:\\Test, brandingImage=img.png", view.Calls[0]);
        Assert.Equal("CloseView", view.Calls[1]);

        var written = sw.ToString().Trim();
        using var responseDoc = JsonDocument.Parse(written);
        var root = responseDoc.RootElement;
        Assert.Equal("welcome_response", root.GetProperty("type").GetString());
        Assert.Equal("cancel", root.GetProperty("result").GetString());
    }

    [Fact]
    public void HandleMessage_init_without_show_welcome_skips_welcome()
    {
        var view = new FakeInstallerView();
        var controller = new InstallerSessionController("dummy", view);
        using var sw = new StringWriter();
        controller.ResponseWriter = sw;

        var ok = controller.HandleMessage("{\"type\":\"init\",\"title\":\"My Title\",\"app_name\":\"TestApp\",\"install_dir\":\"C:\\\\Test\"}");

        Assert.True(ok);
        Assert.Single(view.Calls);
        Assert.Equal("ShowInit: title=My Title, message=My Title", view.Calls[0]);
        Assert.Equal(string.Empty, sw.ToString());
    }

    [Fact]
    public void HandleMessage_init_with_show_welcome_but_no_install_dir_still_prompts()
    {
        var view = new FakeInstallerView { WelcomeResponder = (app, dir, img) => "install" };
        var controller = new InstallerSessionController("dummy", view);
        using var sw = new StringWriter();
        controller.ResponseWriter = sw;

        var ok = controller.HandleMessage("{\"type\":\"init\",\"show_welcome\":true,\"title\":\"My Title\",\"app_name\":\"TestApp\"}");

        Assert.True(ok);
        Assert.Equal(2, view.Calls.Count);
        Assert.Equal("ShowWelcomeAsync: appName=TestApp, installDir=, brandingImage=", view.Calls[0]);

        var written = sw.ToString().Trim();
        using var responseDoc = JsonDocument.Parse(written);
        Assert.Equal("welcome_response", responseDoc.RootElement.GetProperty("type").GetString());
    }
}
