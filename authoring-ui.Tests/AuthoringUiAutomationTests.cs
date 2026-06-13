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

namespace Covenant.Setup.Authoring.Tests;

public class AuthoringUiAutomationTests
{
    private string GetExePath()
    {
        var dir = AppContext.BaseDirectory;
        while (!string.IsNullOrEmpty(dir))
        {
            if (File.Exists(Path.Combine(dir, "Covenant.Setup.slnx")))
            {
                break;
            }
            dir = Path.GetDirectoryName(dir);
        }
        if (string.IsNullOrEmpty(dir))
        {
            throw new DirectoryNotFoundException("Could not find repository root containing Covenant.Setup.slnx");
        }
        var rootDir = dir;
        var exePath = Path.Combine(rootDir, "authoring-ui", "bin", "Debug", "net10.0-windows10.0.19041.0", "win-x64", "Covenant.Setup.Authoring.exe");
        if (!File.Exists(exePath))
        {
            exePath = Path.Combine(rootDir, "dist", "authoring-ui", "Covenant.Setup.Authoring.exe");
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
        if (indent > 12) return;
        var indentStr = new string(' ', indent * 2);
        
        string name = "[Unknown]";
        string id = "[Unknown]";
        string className = "[Unknown]";
        
        try { name = element.Name; } catch {}
        try { id = element.AutomationId; } catch {}
        try { className = element.ClassName; } catch {}

        sb.AppendLine($"{indentStr}- Name: '{name}', Id: '{id}', Class: '{className}'");

        try
        {
            var children = element.FindAllChildren();
            if (children != null)
            {
                foreach (var child in children)
                {
                    if (child != null)
                    {
                        DumpDescendantsInternal(child, sb, indent + 1);
                    }
                }
            }
        }
        catch (System.Exception ex)
        {
            sb.AppendLine($"{indentStr}  [Error listing children: {ex.Message}]");
        }
    }

    private static AutomationElement FindElement(Window window, string automationId, int timeoutMs = 5000)
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

    private static AutomationElement FindElementAcrossWindows(Application app, AutomationBase automation, string automationId, int timeoutMs = 5000)
    {
        var stopwatch = Stopwatch.StartNew();
        while (stopwatch.ElapsedMilliseconds < timeoutMs)
        {
            var windows = app.GetAllTopLevelWindows(automation);
            foreach (var w in windows)
            {
                var el = w.FindFirstDescendant(cf => cf.ByAutomationId(automationId));
                if (el != null)
                {
                    return el;
                }
            }
            Thread.Sleep(100);
        }
        throw new Exception($"Failed to find element with AutomationId '{automationId}' across all windows after {timeoutMs}ms.");
    }

    private static Window GetMainWindow(Application app, AutomationBase automation, string titlePart, int timeoutMs = 15000)
    {
        var stopwatch = Stopwatch.StartNew();
        while (stopwatch.ElapsedMilliseconds < timeoutMs)
        {
            var windows = app.GetAllTopLevelWindows(automation);
            var target = windows.FirstOrDefault(w => w.Title.Contains(titlePart, StringComparison.OrdinalIgnoreCase));
            if (target != null)
            {
                return target;
            }
            Thread.Sleep(250);
        }
        throw new Exception($"Failed to find window with title containing '{titlePart}' after {timeoutMs}ms. Windows found: {string.Join(", ", app.GetAllTopLevelWindows(automation).Select(w => $"'{w.Title}' ({w.ClassName})"))}");
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
    public void TestAllInteractiveContent()
    {
        var exePath = GetExePath();
        Assert.True(File.Exists(exePath), $"Executable not found at: {exePath}. Build the project first.");

        // The expected filename for AppName "FlaUI Test App" is "FlaUITestApp-install.toml"
        var tempManifestPath = Path.Combine(AppContext.BaseDirectory, "FlaUITestApp-install.toml");
        if (File.Exists(tempManifestPath))
        {
            File.Delete(tempManifestPath);
        }

        // We run the app passing the --manifest-path and --tool-path arguments to bypass file dialogs
        var dummyToolPath = Path.Combine(AppContext.BaseDirectory, "dummy-covenant-setup.exe");
        File.WriteAllText(dummyToolPath, "dummy executable");

        var appArgs = $"--manifest-path \"{tempManifestPath}\" --tool-path \"{dummyToolPath}\"";
        
        using (var app = Application.Launch(exePath, appArgs))
        {
            using (var automation = new UIA3Automation())
            {
                var window = GetMainWindow(app, automation, "Manifest Authoring");
                Assert.NotNull(window);

                // Wait a moment for window initialization
                Thread.Sleep(1000);

                // 1. Edit App Name
                var appNameBox = FindElement(window, "AppName").AsTextBox();
                appNameBox.Text = "FlaUI Test App";

                // 2. Edit Application Target Folder
                var folderBox = FindElement(window, "ApplicationFolder").AsTextBox();
                folderBox.Text = "FlaUITestFolder";

                // 3. Edit Primary Payload
                var payloadBox = FindElement(window, "PrimaryPayload").AsTextBox();
                payloadBox.Text = @"payload\testapp.exe";

                // 4. Select Install Root Token Combo
                var rootCombo = FindElement(window, "InstallRootTokenCombo").AsComboBox();
                rootCombo.Select(0);
                Thread.Sleep(300);

                // 5. Add a Directory Path
                var dirPathBox = FindElement(window, "DirectoryPathBox").AsTextBox();
                dirPathBox.Text = @"{LocalAppData}\FlaUITestFolder\subdir";

                var addDirButton = FindElement(window, "AddDirectoryButton").AsButton();
                ClickElement(addDirButton);
                Thread.Sleep(200);

                // 6. Add a File
                var fileSourceBox = FindElement(window, "FileSourceBox").AsTextBox();
                fileSourceBox.Text = @"payload\extra.dll";

                var fileDestBox = FindElement(window, "FileDestinationBox").AsTextBox();
                fileDestBox.Text = @"{LocalAppData}\FlaUITestFolder\extra.dll";

                var addFileButton = FindElement(window, "AddFileButton").AsButton();
                ClickElement(addFileButton);
                Thread.Sleep(200);

                // 7. Add a Registry entry
                var regKeyBox = FindElement(window, "RegistryKeyBox").AsTextBox();
                regKeyBox.Text = @"HKCU\Software\FlaUITestApp";

                var regNameBox = FindElement(window, "RegistryNameBox").AsTextBox();
                regNameBox.Text = "Version";

                var regValBox = FindElement(window, "RegistryValueBox").AsTextBox();
                regValBox.Text = "1.0.0";

                var addRegButton = FindElement(window, "AddRegistryButton").AsButton();
                ClickElement(addRegButton);
                Thread.Sleep(200);

                // 8. Edit Shortcut Description
                var shortcutDescBox = FindElement(window, "ShortcutDescription").AsTextBox();
                shortcutDescBox.Text = "Launch FlaUI Test App";

                // 9. Add a Script
                var scriptCmdBox = FindElement(window, "ScriptCommandBox").AsTextBox();
                scriptCmdBox.Text = "cmd.exe";

                var scriptArgsBox = FindElement(window, "ScriptArgsBox").AsTextBox();
                scriptArgsBox.Text = $"/c{System.Environment.NewLine}echo";

                var scriptWorkDirBox = FindElement(window, "ScriptWorkingDirBox").AsTextBox();
                scriptWorkDirBox.Text = @"{LocalAppData}\FlaUITestFolder";

                var addScriptButton = FindElement(window, "AddScriptButton").AsButton();
                ClickElement(addScriptButton);
                Thread.Sleep(200);

                // 10. Toggle Theme
                var themeToggle = FindElement(window, "ThemeToggle");
                ClickElement(themeToggle);
                Thread.Sleep(200);

                // 11. Verify TOML Preview
                var previewBox = FindElement(window, "PreviewBox").AsTextBox();
                var previewText = previewBox.Text;
                Assert.Contains("app_name = 'FlaUI Test App'", previewText);
                Assert.Contains("FlaUITestFolder", previewText);
                Assert.Contains("testapp.exe", previewText);
                Assert.Contains("subdir", previewText);
                Assert.Contains("extra.dll", previewText);
                Assert.Contains("FlaUITestApp", previewText);

                // 12. Validate
                var validateButton = FindElement(window, "ValidateButton").AsButton();
                ClickElement(validateButton);
                Thread.Sleep(500);

                // Find the ContentDialog and close it (should have a CloseButton)
                var okButton = FindElementAcrossWindows(app, automation, "CloseButton").AsButton();
                ClickElement(okButton);
                Thread.Sleep(300);

                // 13. Installer Config Dialog
                var configButton = FindElement(window, "InstallerConfigButton").AsButton();
                ClickElement(configButton);
                Thread.Sleep(500);

                // Find the config inputs inside the ContentDialog
                var configPathBox = FindElementAcrossWindows(app, automation, "CovenantSetupPathBox").AsTextBox();
                configPathBox.Text = dummyToolPath;

                var outputDirBox = FindElementAcrossWindows(app, automation, "OutputDirectoryBox").AsTextBox();
                outputDirBox.Text = AppContext.BaseDirectory;

                var saveAndCloseButton = FindElementAcrossWindows(app, automation, "PrimaryButton").AsButton();
                if (!saveAndCloseButton.IsEnabled)
                {
                    var statusEl = FindElement(window, "ValidationSummaryText").AsLabel();
                    throw new System.Exception($"Save and Close button is disabled! Validation Summary: '{statusEl.Text}'");
                }
                ClickElement(saveAndCloseButton);
                Thread.Sleep(300);

                // 14. Save TOML (this saves manifest to tempManifestPath without dialog, because we configured --manifest-path)
                var saveButton = FindElement(window, "SaveButton").AsButton();
                ClickElement(saveButton);
                Thread.Sleep(500);

                // Confirm save dialog OK click
                var saveOkButton = FindElementAcrossWindows(app, automation, "CloseButton").AsButton();
                ClickElement(saveOkButton);
                Thread.Sleep(300);

                Assert.True(File.Exists(tempManifestPath), "Manifest was not written to output path.");
                var writtenManifest = File.ReadAllText(tempManifestPath);
                Assert.Contains("app_name = 'FlaUI Test App'", writtenManifest);

                // Close application
                app.Close();
            }
        }

        // Clean up
        if (File.Exists(tempManifestPath))
        {
            File.Delete(tempManifestPath);
        }
        if (File.Exists(dummyToolPath))
        {
            File.Delete(dummyToolPath);
        }
    }
}
