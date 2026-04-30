using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace Covenant.Setup.Authoring;

internal static class Program
{
    [STAThread]
    private static void Main()
    {
        Application.Start(_params =>
        {
            var dispatcherQueue = DispatcherQueue.GetForCurrentThread();
            if (dispatcherQueue is not null)
            {
                SynchronizationContext.SetSynchronizationContext(
                    new DispatcherQueueSynchronizationContext(dispatcherQueue));
            }

            _ = new AuthoringApp();
        });
    }
}

internal sealed class AuthoringApp : Application
{
    private Window? _window;

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var window = new MainWindow();
        _window = window;
        window.Activate();
    }
}
