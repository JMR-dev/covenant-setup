using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Covenant.Setup.Ui;

namespace Covenant.Setup.Ui.Tests;

internal sealed class FakeInstallerView : IInstallerView
{
    public List<string> Calls { get; } = new();

    public Func<UiMessage, string> PromptResponder { get; set; } = _ => "ok";

    public void ShowInit(string title, string message)
    {
        Calls.Add($"ShowInit: title={title}, message={message}");
    }

    public void ShowProgress(int percent, string? message, int currentStep = 0)
    {
        Calls.Add($"ShowProgress: percent={percent}, message={message}");
    }

    public void AppendLog(string message)
    {
        Calls.Add($"AppendLog: message={message}");
    }

    public void ShowFinished(string message)
    {
        Calls.Add($"ShowFinished: message={message}");
    }

    public void ShowFailure(string failureMessage, string? errorDetails, string errataJson)
    {
        Calls.Add($"ShowFailure: failureMessage={failureMessage}, errorDetails={errorDetails}, errataJson={errataJson}");
    }

    public Task<string> ShowPromptAsync(UiMessage message)
    {
        Calls.Add($"ShowPromptAsync: id={message.Id}, title={message.Title}, buttons={message.Buttons}, icon={message.Icon}");
        return Task.FromResult(PromptResponder(message));
    }

    public Func<string, string, string?, string> WelcomeResponder { get; set; } = (_, _, _) => "install";

    public Task<string> ShowWelcomeAsync(string appName, string installDir, string? brandingImage)
    {
        Calls.Add($"ShowWelcomeAsync: appName={appName}, installDir={installDir}, brandingImage={brandingImage}");
        return Task.FromResult(WelcomeResponder(appName, installDir, brandingImage));
    }

    public void CloseView()
    {
        Calls.Add("CloseView");
    }

    public void ShowPipeError(string message)
    {
        Calls.Add($"ShowPipeError: message={message}");
    }
}
