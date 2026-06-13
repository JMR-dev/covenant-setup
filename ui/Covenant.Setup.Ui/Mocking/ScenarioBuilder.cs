using System;
using System.Collections.Generic;
using System.Text.Json;

namespace Covenant.Setup.Ui.Mocking;

internal sealed class ScenarioBuilder
{
    private readonly string _name;
    private readonly List<ScenarioStep> _steps = new();

    public ScenarioBuilder(string name)
    {
        _name = name;
    }

    public ScenarioBuilder Init(string title, string message)
    {
        return Init(title, message, appName: null, installDir: null);
    }

    public ScenarioBuilder Init(string title, string message, string? appName, string? installDir, string? brandingImage = null, bool showWelcome = false)
    {
        var msg = new UiMessage
        {
            Type = "init",
            Title = title,
            Message = message,
            AppName = appName,
            InstallDir = installDir,
            BrandingImage = brandingImage,
            ShowWelcome = showWelcome ? true : null
        };
        return Raw(msg);
    }

    public ScenarioBuilder Progress(int currentStep, int totalSteps, string? message = null)
    {
        var msg = new UiMessage
        {
            Type = "progress",
            CurrentStep = currentStep,
            TotalSteps = totalSteps,
            Message = message
        };
        return Raw(msg);
    }

    public ScenarioBuilder Log(string message)
    {
        var msg = new UiMessage
        {
            Type = "log",
            Message = message
        };
        return Raw(msg);
    }

    public ScenarioBuilder Finish(string message)
    {
        var msg = new UiMessage
        {
            Type = "finish",
            Message = message
        };
        return Raw(msg);
    }

    public ScenarioBuilder Fail(string appName, string operation, string message, string error, string? errataJson = null)
    {
        JsonElement? errata = null;
        if (errataJson != null)
        {
            errata = JsonSerializer.Deserialize<JsonElement>(errataJson);
        }

        var msg = new UiMessage
        {
            Type = "fail",
            AppName = appName,
            Operation = operation,
            Message = message,
            Error = error,
            Errata = errata
        };
        return Raw(msg);
    }

    public ScenarioBuilder Prompt(string id, string title, string message, string buttons, string icon)
    {
        var msg = new UiMessage
        {
            Type = "prompt",
            Id = id,
            Title = title,
            Message = message,
            Buttons = buttons,
            Icon = icon
        };
        return Raw(msg);
    }

    public ScenarioBuilder AwaitPromptResponse(string id, string? expect)
    {
        _steps.Add(new AwaitResponseStep(id, expect));
        return this;
    }

    public ScenarioBuilder AwaitWelcomeResponse(string? expect = "install")
    {
        _steps.Add(new AwaitResponseStep("welcome", expect));
        return this;
    }

    public ScenarioBuilder AwaitCancelRequest()
    {
        _steps.Add(new AwaitResponseStep("cancel", null));
        return this;
    }

    public ScenarioBuilder Delay(TimeSpan delay)
    {
        _steps.Add(new DelayStep(delay));
        return this;
    }

    public ScenarioBuilder Delay(int milliseconds)
    {
        return Delay(TimeSpan.FromMilliseconds(milliseconds));
    }

    public ScenarioBuilder Close()
    {
        var msg = new UiMessage
        {
            Type = "close"
        };
        return Raw(msg);
    }

    public ScenarioBuilder Raw(UiMessage message)
    {
        var json = JsonSerializer.Serialize(message, UiJson.Options);
        _steps.Add(new SendStep(json));
        return this;
    }

    public ScenarioBuilder Raw(string rawJsonLine)
    {
        _steps.Add(new SendStep(rawJsonLine));
        return this;
    }

    public Scenario Build()
    {
        return new Scenario(_name, _steps);
    }
}
