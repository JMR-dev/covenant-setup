using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using FlaUI.Core;
using FlaUI.Core.AutomationElements;
using FlaUI.Core.Definitions;
using FlaUI.Core.Input;
using FlaUI.UIA3;
using Xunit;

namespace Covenant.Setup.Ui.Tests;

public class InstallerUiAutomationTests
{
    private string GetExePath()
    {
        var dir = AppContext.BaseDirectory;
        while (!string.IsNullOrEmpty(dir))
        {
            if (Directory.Exists(Path.Combine(dir, "Covenant.Setup.Ui")) && 
                Directory.Exists(Path.Combine(dir, "Covenant.Setup.Ui.Tests")))
            {
                break;
            }
            dir = Path.GetDirectoryName(dir);
        }
        if (string.IsNullOrEmpty(dir))
        {
            throw new DirectoryNotFoundException("Could not find ui root directory");
        }
        var uiDir = dir;
        var exePath = Path.Combine(uiDir, "Covenant.Setup.Ui", "bin", "x64", "Debug", "net10.0-windows10.0.19041.0", "win-x64", "Covenant.Setup.Ui.exe");
        if (!File.Exists(exePath))
        {
            exePath = Path.Combine(uiDir, "Covenant.Setup.Ui", "bin", "x64", "Release", "net10.0-windows10.0.19041.0", "win-x64", "Covenant.Setup.Ui.exe");
        }
        return exePath;
    }

    private static string DumpDescendants(AutomationElement root)
    {
        var sb = new StringBuilder();
        DumpDescendantsInternal(root, sb, 0);
        return sb.ToString();
    }

    private static void DumpDescendantsInternal(AutomationElement element, StringBuilder sb, int indent)
    {
        if (indent > 8) return;
        var indentStr = new string(' ', indent * 2);
        try
        {
            sb.AppendLine($"{indentStr}- Name: '{element.Name}', Id: '{element.AutomationId}', Class: '{element.ClassName}'");
            foreach (var child in element.FindAllChildren())
            {
                DumpDescendantsInternal(child, sb, indent + 1);
            }
        }
        catch (System.Exception ex)
        {
            sb.AppendLine($"{indentStr}- [Error reading element: {ex.Message}]");
        }
    }

    private static AutomationElement FindElement(Window window, string automationId, int timeoutMs = 10000)
    {
        var stopwatch = Stopwatch.StartNew();
        while (stopwatch.ElapsedMilliseconds < timeoutMs)
        {
            var el = window.FindFirstDescendant(cf => cf.ByAutomationId(automationId));
            if (el != null)
            {
                return el;
            }
            Thread.Sleep(100);
        }
        throw new Exception($"Failed to find element with AutomationId '{automationId}' after {timeoutMs}ms.\nVisual Tree:\n{DumpDescendants(window)}");
    }

    private static AutomationElement FindElementByName(Window window, string name, int timeoutMs = 10000)
    {
        var stopwatch = Stopwatch.StartNew();
        while (stopwatch.ElapsedMilliseconds < timeoutMs)
        {
            var el = window.FindFirstDescendant(cf => cf.ByName(name));
            if (el != null)
            {
                return el;
            }
            Thread.Sleep(100);
        }
        throw new Exception($"Failed to find element with Name '{name}' after {timeoutMs}ms.\nVisual Tree:\n{DumpDescendants(window)}");
    }

    private static Window GetMainWindow(Application app, AutomationBase automation, string titlePart, int timeoutMs = 15000)
    {
        var stopwatch = Stopwatch.StartNew();
        while (stopwatch.ElapsedMilliseconds < timeoutMs)
        {
            try
            {
                var windows = app.GetAllTopLevelWindows(automation);
                var target = windows.FirstOrDefault(w => 
                    w.Title.Contains(titlePart, System.StringComparison.OrdinalIgnoreCase) ||
                    w.Title.Contains("covenant-setup", System.StringComparison.OrdinalIgnoreCase) ||
                    w.Title.Contains("SampleApp", System.StringComparison.OrdinalIgnoreCase));
                if (target != null)
                {
                    return target;
                }
            }
            catch
            {
                // Ignore COM errors
            }

            try
            {
                var desktopChildren = automation.GetDesktop().FindAllChildren();
                foreach (var child in desktopChildren)
                {
                    if (child.ControlType == FlaUI.Core.Definitions.ControlType.Window)
                    {
                        var w = child.AsWindow();
                        if (w.Title.Contains(titlePart, System.StringComparison.OrdinalIgnoreCase) ||
                            w.Title.Contains("covenant-setup", System.StringComparison.OrdinalIgnoreCase) ||
                            w.Title.Contains("SampleApp", System.StringComparison.OrdinalIgnoreCase) ||
                            w.Title.Contains("Uninstalling", System.StringComparison.OrdinalIgnoreCase))
                        {
                            return w;
                        }
                    }
                }
            }
            catch
            {
                // Ignore COM errors
            }

            Thread.Sleep(250);
        }

        var allWindowsStr = "None";
        try
        {
            allWindowsStr = string.Join(", ", automation.GetDesktop().FindAllChildren()
                .Where(c => c.ControlType == FlaUI.Core.Definitions.ControlType.Window)
                .Select(c => $"'{c.Name}' ({c.ClassName})"));
        }
        catch {}
        throw new Exception($"Failed to find window with title containing '{titlePart}' after {timeoutMs}ms. All desktop windows found: {allWindowsStr}");
    }

    private static void ClickElement(AutomationElement element)
    {
        if (element is Button button)
        {
            button.Invoke();
        }
        else if (element.Patterns.Invoke.IsSupported)
        {
            element.Patterns.Invoke.Pattern.Invoke();
        }
        else if (element.Patterns.Toggle.IsSupported)
        {
            element.Patterns.Toggle.Pattern.Toggle();
        }
        else if (element.Patterns.SelectionItem.IsSupported)
        {
            element.Patterns.SelectionItem.Pattern.Select();
        }
        else
        {
            element.Click(false);
        }
    }

    [Fact]
    public void TestMockInstallHappy()
    {
        var exePath = GetExePath();
        Assert.True(File.Exists(exePath), $"Executable not found at: {exePath}");

        using (var app = Application.Launch(exePath, "--mock install-happy"))
        {
            using (var automation = new UIA3Automation())
            {
                var window = GetMainWindow(app, automation, "covenant-setup");
                Assert.NotNull(window);

                // Click the Welcome Install button
                var installBtn = FindElement(window, "WelcomeInstallButton").AsButton();
                ClickElement(installBtn);

                // Wait for the install process to finish or for the application to exit
                var stopwatch = Stopwatch.StartNew();
                bool exitedCleanly = false;
                while (stopwatch.ElapsedMilliseconds < 15000)
                {
                    if (app.HasExited)
                    {
                        exitedCleanly = true;
                        break;
                    }
                    try
                    {
                        var btn = window.FindFirstDescendant(cf => cf.ByAutomationId("CancelButton"))?.AsButton();
                        if (btn != null && btn.Name == "Close")
                        {
                            ClickElement(btn);
                            break;
                        }
                    }
                    catch
                    {
                        // Ignore UIA/COM exceptions while UI transitions
                    }
                    Thread.Sleep(250);
                }
                
                if (!exitedCleanly)
                {
                    stopwatch = Stopwatch.StartNew();
                    while (stopwatch.ElapsedMilliseconds < 5000)
                    {
                        if (app.HasExited)
                        {
                            exitedCleanly = true;
                            break;
                        }
                        Thread.Sleep(250);
                    }
                }
                Assert.True(exitedCleanly || app.HasExited, "Application should have exited.");
            }
        }
    }

    [Fact]
    public void TestMockInstallPrompt()
    {
        var exePath = GetExePath();
        Assert.True(File.Exists(exePath), $"Executable not found at: {exePath}");

        using (var app = Application.Launch(exePath, "--mock install-prompt"))
        {
            using (var automation = new UIA3Automation())
            {
                var window = GetMainWindow(app, automation, "covenant-setup");
                Assert.NotNull(window);

                // An OK/Cancel content dialog should prompt FIRST. Accept it.
                var okBtn = FindElementByName(window, "OK").AsButton();
                ClickElement(okBtn);

                // Click the Welcome Install button which appears after confirming the prompt
                var installBtn = FindElement(window, "WelcomeInstallButton").AsButton();
                ClickElement(installBtn);

                // Wait for the install process to finish or for the application to exit
                var stopwatch = Stopwatch.StartNew();
                bool exitedCleanly = false;
                while (stopwatch.ElapsedMilliseconds < 15000)
                {
                    if (app.HasExited)
                    {
                        exitedCleanly = true;
                        break;
                    }
                    try
                    {
                        var btn = window.FindFirstDescendant(cf => cf.ByAutomationId("CancelButton"))?.AsButton();
                        if (btn != null && btn.Name == "Close")
                        {
                            ClickElement(btn);
                            break;
                        }
                    }
                    catch
                    {
                        // Ignore UIA/COM exceptions while UI transitions
                    }
                    Thread.Sleep(250);
                }
                
                if (!exitedCleanly)
                {
                    stopwatch = Stopwatch.StartNew();
                    while (stopwatch.ElapsedMilliseconds < 5000)
                    {
                        if (app.HasExited)
                        {
                            exitedCleanly = true;
                            break;
                        }
                        Thread.Sleep(250);
                    }
                }
                Assert.True(exitedCleanly || app.HasExited, "Application should have exited.");
            }
        }
    }

    [Fact]
    public void TestMockInstallFailErrata()
    {
        var exePath = GetExePath();
        Assert.True(File.Exists(exePath), $"Executable not found at: {exePath}");

        using (var app = Application.Launch(exePath, "--mock install-fail-errata"))
        {
            using (var automation = new UIA3Automation())
            {
                var window = GetMainWindow(app, automation, "covenant-setup");
                Assert.NotNull(window);

                // Click the Welcome Install button
                var installBtn = FindElement(window, "WelcomeInstallButton").AsButton();
                ClickElement(installBtn);

                // Wait for failure. SaveErrataButton and CopyErrorButton should be visible
                Button? saveErrataBtn = null;
                Button? copyErrorBtn = null;
                var stopwatch = Stopwatch.StartNew();
                while (stopwatch.ElapsedMilliseconds < 10000)
                {
                    try
                    {
                        var saveBtn = window.FindFirstDescendant(cf => cf.ByAutomationId("SaveErrataButton"))?.AsButton();
                        var copyBtn = window.FindFirstDescendant(cf => cf.ByAutomationId("CopyErrorButton"))?.AsButton();
                        if (saveBtn != null && copyBtn != null && !saveBtn.IsOffscreen && !copyBtn.IsOffscreen)
                        {
                            saveErrataBtn = saveBtn;
                            copyErrorBtn = copyBtn;
                            break;
                        }
                    }
                    catch
                    {
                        // Ignore COM exceptions during transition
                    }
                    Thread.Sleep(250);
                }

                Assert.NotNull(saveErrataBtn);
                Assert.NotNull(copyErrorBtn);
                Assert.True(saveErrataBtn.IsEnabled, "Save Errata button should be enabled on failure.");
                Assert.True(copyErrorBtn.IsEnabled, "Copy Error button should be enabled on failure.");

                // Close the installer manually since it does not auto-exit on failure
                var closeBtn = FindElement(window, "CancelButton").AsButton();
                ClickElement(closeBtn);
            }
        }
    }

    [Fact]
    public void TestMockUninstallRebootPrompt()
    {
        var exePath = GetExePath();
        Assert.True(File.Exists(exePath), $"Executable not found at: {exePath}");

        using (var app = Application.Launch(exePath, "--mock uninstall-reboot-prompt"))
        {
            using (var automation = new UIA3Automation())
            {
                var window = GetMainWindow(app, automation, "Uninstalling");
                Assert.NotNull(window);

                // At the end, a Yes/No restart prompt will show up. Choose No.
                var noBtn = FindElementByName(window, "No").AsButton();
                ClickElement(noBtn);

                // Wait for the uninstall process to finish or for the application to exit
                var stopwatch = Stopwatch.StartNew();
                bool exitedCleanly = false;
                while (stopwatch.ElapsedMilliseconds < 15000)
                {
                    if (app.HasExited)
                    {
                        exitedCleanly = true;
                        break;
                    }
                    try
                    {
                        var btn = window.FindFirstDescendant(cf => cf.ByAutomationId("CancelButton"))?.AsButton();
                        if (btn != null && btn.Name == "Close")
                        {
                            ClickElement(btn);
                            break;
                        }
                    }
                    catch
                    {
                        // Ignore UIA/COM exceptions during transition
                    }
                    Thread.Sleep(250);
                }
                
                if (!exitedCleanly)
                {
                    stopwatch = Stopwatch.StartNew();
                    while (stopwatch.ElapsedMilliseconds < 5000)
                    {
                        if (app.HasExited)
                        {
                            exitedCleanly = true;
                            break;
                        }
                        Thread.Sleep(250);
                    }
                }
                Assert.True(exitedCleanly || app.HasExited, "Application should have exited.");
            }
        }
    }

    [Fact]
    public void TestRealInstallUninstallFlow()
    {
        // This test runs only when configured via environment variables
        var realInstallerPath = System.Environment.GetEnvironmentVariable("COVENANT_REAL_INSTALLER_PATH");
        if (string.IsNullOrEmpty(realInstallerPath) || !File.Exists(realInstallerPath))
        {
            return; // Skip if not executing in the VM real context
        }

        var realInstallerArgs = System.Environment.GetEnvironmentVariable("COVENANT_REAL_INSTALLER_ARGS") ?? "--headed";
        
        // Filter out --automation to ensure the installer/uninstaller runs interactively
        var argsList = realInstallerArgs.Split(' ', System.StringSplitOptions.RemoveEmptyEntries).ToList();
        bool isUninstall = argsList.Any(a => a.Equals("uninstall", System.StringComparison.OrdinalIgnoreCase)) || 
                           realInstallerPath.Contains("uninstall", System.StringComparison.OrdinalIgnoreCase);
        argsList.Remove("--automation");
        realInstallerArgs = string.Join(" ", argsList);

        using (var app = Application.Launch(realInstallerPath, realInstallerArgs))
        {
            using (var automation = new UIA3Automation())
            {
                if (!isUninstall)
                {
                    // 1. Consent/Welcome Page: Click Install
                    var window = GetMainWindow(app, automation, "covenant-setup");
                    Assert.NotNull(window);

                    var installBtn = FindElement(window, "WelcomeInstallButton").AsButton();
                    ClickElement(installBtn);

                    // 2. Wait for progress to finish and close button to appear
                    Button? closeBtn = null;
                    var stopwatch = Stopwatch.StartNew();
                    while (stopwatch.ElapsedMilliseconds < 30000)
                    {
                        try
                        {
                            var btn = window.FindFirstDescendant(cf => cf.ByAutomationId("CancelButton"))?.AsButton();
                            if (btn != null && btn.Name == "Close")
                            {
                                closeBtn = btn;
                                break;
                            }
                        }
                        catch
                        {
                            // Ignore COM exceptions during transition
                        }
                        Thread.Sleep(250);
                    }
                    Assert.NotNull(closeBtn);
                    Assert.Equal("Close", closeBtn.Name);
                    ClickElement(closeBtn);
                }
                else
                {
                    // For uninstall: progress runs immediately.
                    // When finished, the uninstall success prompt "covenant-setup" shows up.
                    // Let's wait for that prompt window to appear.
                    var stopwatch = Stopwatch.StartNew();
                    Window? promptWindow = null;
                    while (stopwatch.ElapsedMilliseconds < 30000)
                    {
                        try
                        {
                            // Search all desktop windows for one named "covenant-setup"
                            var desktopChildren = automation.GetDesktop().FindAllChildren();
                            foreach (var child in desktopChildren)
                            {
                                if (child.ControlType == FlaUI.Core.Definitions.ControlType.Window)
                                {
                                    var w = child.AsWindow();
                                    if (w.Title.Equals("covenant-setup", System.StringComparison.OrdinalIgnoreCase))
                                    {
                                        promptWindow = w;
                                        break;
                                    }
                                }
                            }
                            if (promptWindow != null)
                            {
                                break;
                            }
                        }
                        catch
                        {
                            // Ignore COM exceptions
                        }
                        Thread.Sleep(250);
                    }

                    Assert.NotNull(promptWindow);

                    // The dialog has an "OK" button. Find it and click it.
                    var okBtn = FindElementByName(promptWindow, "OK").AsButton();
                    ClickElement(okBtn);
                }

                // Verify the application exits cleanly
                var exitStopwatch = Stopwatch.StartNew();
                bool exited = false;
                while (exitStopwatch.ElapsedMilliseconds < 10000)
                {
                    if (app.HasExited)
                    {
                        exited = true;
                        break;
                    }
                    Thread.Sleep(250);
                }
                Assert.True(exited || app.HasExited, "Application should have exited after closing final dialog/window.");
            }
        }
    }
}
