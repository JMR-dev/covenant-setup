using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace Covenant.Setup.Ui;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        var pipeName = ReadPipeName(args);
        UiTrace.Write("process_start", new { ProcessId = Environment.ProcessId, PipeName = pipeName });
        if (string.IsNullOrWhiteSpace(pipeName))
        {
            UiTrace.Write("missing_pipe_argument");
            NativeDialog.Show(IntPtr.Zero, "Missing named pipe argument.", "covenant-setup", NativeDialogIcon.Error);
            return;
        }

        Application.Start(_params =>
        {
            var dispatcherQueue = DispatcherQueue.GetForCurrentThread();
            if (dispatcherQueue is not null)
            {
                SynchronizationContext.SetSynchronizationContext(
                    new DispatcherQueueSynchronizationContext(dispatcherQueue));
            }

            _ = new InstallerUiApp(pipeName);
        });
    }

    internal static string? ReadPipeName(string[] args)
    {
        for (var i = 0; i < args.Length - 1; i++)
        {
            if (string.Equals(args[i], "--pipe", StringComparison.OrdinalIgnoreCase))
            {
                var value = args[i + 1];
                const string pipePrefix = @"\\.\pipe\";
                if (value.StartsWith(pipePrefix, StringComparison.OrdinalIgnoreCase))
                {
                    value = value[pipePrefix.Length..];
                }
                return value;
            }
        }

        return null;
    }
}

internal sealed class InstallerUiApp(string pipeName) : Application
{
    private Window? _window;

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var window = new InstallerUiWindow(pipeName);
        _window = window;
        window.Activate();
        window.StartPipeLoop();
    }
}

internal enum NativeDialogIcon : uint
{
    Error = 0x00000010,
    Information = 0x00000040
}

internal static class NativeDialog
{
    public static void Show(IntPtr owner, string message, string title, NativeDialogIcon icon)
    {
        _ = MessageBoxW(owner, message, title, (uint)icon);
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int MessageBoxW(IntPtr hWnd, string text, string caption, uint type);
}
