using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace Covenant.Setup.Ui;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();

        var mockScenarioName = ReadMockScenario(args);
        Mocking.Scenario? mockScenario = null;
        if (mockScenarioName is not null)
        {
            try
            {
                mockScenario = Mocking.Scenario.LoadFile(mockScenarioName);
            }
            catch (Exception ex)
            {
                UiTrace.Write("mock_scenario_load_error", new { ex.Message });
                NativeDialog.Show(IntPtr.Zero, $"Failed to load mock scenario: {ex.Message}", "covenant-setup", NativeDialogIcon.Error);
                return;
            }
        }

        var pipeName = ReadPipeName(args);
        if (string.IsNullOrWhiteSpace(pipeName))
        {
            if (mockScenario is not null)
            {
                pipeName = $"covenant-setup-mock-{Environment.ProcessId}-{Guid.NewGuid():N}";
            }
            else
            {
                UiTrace.Write("missing_pipe_argument");
                NativeDialog.Show(IntPtr.Zero, "Missing named pipe argument.", "covenant-setup", NativeDialogIcon.Error);
                return;
            }
        }

        UiTrace.Write("process_start", new { ProcessId = Environment.ProcessId, PipeName = pipeName });

        Application.Start(_params =>
        {
            var dispatcherQueue = DispatcherQueue.GetForCurrentThread();
            if (dispatcherQueue is not null)
            {
                SynchronizationContext.SetSynchronizationContext(
                    new DispatcherQueueSynchronizationContext(dispatcherQueue));
            }

            _ = new InstallerUiApp(pipeName, mockScenario);
        });
    }

    internal static string? ReadPipeName(string[] args)
    {
        var value = ReadArgValue(args, "--pipe");
        const string pipePrefix = @"\\.\pipe\";
        if (value is not null && value.StartsWith(pipePrefix, StringComparison.OrdinalIgnoreCase))
        {
            value = value[pipePrefix.Length..];
        }

        return value;
    }

    internal static string? ReadMockScenario(string[] args)
    {
        return ReadArgValue(args, "--mock");
    }

    private static string? ReadArgValue(string[] args, string flag)
    {
        for (var i = 0; i < args.Length - 1; i++)
        {
            if (string.Equals(args[i], flag, StringComparison.OrdinalIgnoreCase))
            {
                return args[i + 1];
            }
        }

        return null;
    }
}

public sealed partial class InstallerUiApp : Application
{
    private readonly string _pipeName;
    private readonly Mocking.Scenario? _mockScenario;
    private Window? _window;

    internal InstallerUiApp(string pipeName, Mocking.Scenario? mockScenario)
    {
        _pipeName = pipeName;
        _mockScenario = mockScenario;
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var window = new InstallerUiWindow(_pipeName);
        _window = window;
        window.Activate();
        window.StartPipeLoop();

        if (_mockScenario is not null)
        {
            _ = Task.Run(() => new Mocking.MockEngineClient(_pipeName, _mockScenario, new Mocking.MockEngineOptions { StrictExpectations = false }).RunAsync());
        }
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
