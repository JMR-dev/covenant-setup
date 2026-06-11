namespace Covenant.Setup.Ui;

internal interface IInstallerView
{
    void ShowInit(string title, string message);

    void ShowProgress(int percent, string? message, int currentStep = 0);

    void AppendLog(string message);

    void ShowFinished(string message);

    void ShowFailure(string failureMessage, string? errorDetails, string errataJson);

    Task<string> ShowPromptAsync(UiMessage message);

    Task<string> ShowWelcomeAsync(string appName, string installDir, string? brandingImage);

    void CloseView();

    void ShowPipeError(string message);
}
