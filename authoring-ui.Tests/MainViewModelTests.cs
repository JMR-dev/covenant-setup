using Covenant.Setup.Authoring;
using Xunit;

namespace Covenant.Setup.Authoring.Tests;

public class MainViewModelTests
{
    [Fact]
    public void Constructor_renders_default_template_preview_without_mutating_manual_entries()
    {
        var viewModel = new MainViewModel(() => null);

        Assert.Equal("Covenant-Setup Sample App", viewModel.AppName);
        Assert.Empty(viewModel.Directories);
        Assert.Empty(viewModel.Files);
        Assert.Empty(viewModel.PurgePaths);
        Assert.Equal("Covenant-SetupSampleApp-install.toml", viewModel.ExpectedManifestFileName);
        Assert.Contains("app_name = 'Covenant-Setup Sample App'", viewModel.TomlPreview);
        Assert.Contains(@"{LocalAppData}\CovenantSetupSample", viewModel.TomlPreview);
        Assert.Contains(@"{Desktop}\CovenantSetupSample.lnk", viewModel.TomlPreview);
        Assert.DoesNotContain(@"{Desktop}\Covenant-Setup Sample App.lnk", viewModel.TomlPreview, StringComparison.Ordinal);
        Assert.Contains(@"payload\sample_app.cmd", viewModel.TomlPreview);
    }

    [Fact]
    public void Expected_manifest_file_name_tracks_app_name()
    {
        var viewModel = new MainViewModel(() => null)
        {
            AppName = "Renamed App"
        };

        Assert.Equal("RenamedApp-install.toml", viewModel.ExpectedManifestFileName);
        Assert.True(viewModel.IsExpectedManifestPath(@"C:\manifests\RenamedApp-install.toml"));
        Assert.False(viewModel.IsExpectedManifestPath(@"C:\manifests\install.toml"));
        Assert.False(viewModel.IsExpectedManifestPath(@"C:\manifest folder\RenamedApp-install.toml"));
    }

    [Fact]
    public void Validate_rejects_app_names_that_cannot_be_manifest_file_names()
    {
        var viewModel = new MainViewModel(() => null)
        {
            AppName = "Bad:Name"
        };

        var validation = viewModel.Validate();

        Assert.False(validation.IsValid);
        Assert.Contains(validation.Errors, error => error.Contains("invalid in Windows file names", StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public void Validate_rejects_unsupported_registry_roots()
    {
        var viewModel = new MainViewModel(() => null);
        viewModel.Registry.Clear();
        viewModel.PurgeRegistryBranches.Clear();
        viewModel.AddRegistry(@"HKCR\Software\Bad", "Name", "Value");
        viewModel.AddPurgeRegistryBranch(@"HKU\Software\Bad");

        var validation = viewModel.Validate();

        Assert.False(validation.IsValid);
        Assert.Contains(validation.Errors, error => error.Contains(@"HKCR\Software\Bad", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains(@"HKU\Software\Bad", StringComparison.Ordinal));
    }

    [Fact]
    public void Validate_warns_when_manifest_requires_elevation()
    {
        var viewModel = new MainViewModel(() => null)
        {
            InstallRootToken = "{ProgramFilesX64}"
        };

        var validation = viewModel.Validate();

        Assert.True(validation.IsValid);
        Assert.Contains(validation.Warnings, warning => warning.Contains("require elevation", StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public void Validate_rejects_whitespace_in_manifest_fields_except_app_name_and_description()
    {
        var viewModel = new MainViewModel(() => null)
        {
            AppName = "Display Name With Spaces",
            ApplicationFolder = "Folder With Spaces",
            PrimaryPayload = @"payload\bad file.exe"
        };
        viewModel.AddDirectory(@"{LocalAppData}\Bad Path");
        viewModel.AddFile(@"payload\manual file.dll", @"{LocalAppData}\Manual\manual.dll");
        viewModel.AddRegistry(@"HKCU\Software\Bad Key", "Install Root", @"{LocalAppData}\Bad Value");
        viewModel.AddShortcut(
            @"{Desktop}\Bad Link.lnk",
            @"{LocalAppData}\Manual\manual.exe",
            "--profile default",
            @"{LocalAppData}\Working Directory",
            "Description can have spaces");
        viewModel.AddScript(
            "post install.cmd",
            ["--ok", "two words"],
            @"{LocalAppData}\Script Working");
        viewModel.AddPurgePath(@"{LocalAppData}\Purge Path");
        viewModel.AddPurgeRegistryBranch(@"HKCU\Software\Purge Branch");

        var validation = viewModel.Validate();

        Assert.False(validation.IsValid);
        Assert.Contains(validation.Errors, error => error.Contains("Application target installation folder", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("File source", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Directory path", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Registry key", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Registry name", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Registry value", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Shortcut path", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Shortcut arguments", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Shortcut working directory", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Script command", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Script argument 2", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Script working directory", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Purge path", StringComparison.Ordinal));
        Assert.Contains(validation.Errors, error => error.Contains("Purge registry branch", StringComparison.Ordinal));
        Assert.DoesNotContain(validation.Errors, error => error.Contains("Description", StringComparison.Ordinal));
        Assert.DoesNotContain(validation.Errors, error => error.Contains("App name", StringComparison.Ordinal));
    }

    [Fact]
    public void Preview_replaces_template_values_without_removing_manual_entries()
    {
        var viewModel = new MainViewModel(() => null);
        viewModel.AddDirectory(@"{LocalAppData}\Manual");
        viewModel.AddFile(@"payload\manual.dll", @"{LocalAppData}\Manual\manual.dll");
        viewModel.AddPurgePath(@"{LocalAppData}\Manual");

        viewModel.AppName = "Renamed App";
        viewModel.ApplicationFolder = "RenamedApp";
        viewModel.InstallRootToken = "{ProgramFilesX64}";
        viewModel.PrimaryPayload = @"payload\renamed.exe";

        Assert.DoesNotContain(viewModel.Directories, directory => directory == @"{ProgramFilesX64}\RenamedApp");
        Assert.DoesNotContain(viewModel.Files, file => file.Source == @"payload\renamed.exe");
        Assert.DoesNotContain(viewModel.PurgePaths, path => path == @"{ProgramFilesX64}\RenamedApp");
        Assert.Contains(viewModel.Directories, directory => directory == @"{LocalAppData}\Manual");
        Assert.Contains(viewModel.Files, file => file.Source == @"payload\manual.dll");
        Assert.Contains(viewModel.PurgePaths, path => path == @"{LocalAppData}\Manual");
        Assert.Contains("app_name = 'Renamed App'", viewModel.TomlPreview);
        Assert.Contains(@"{ProgramFilesX64}\RenamedApp", viewModel.TomlPreview);
        Assert.Contains(@"payload\renamed.exe", viewModel.TomlPreview);
        Assert.Contains(@"{LocalAppData}\Manual", viewModel.TomlPreview);
        Assert.DoesNotContain("CovenantSetupSample", viewModel.TomlPreview, StringComparison.Ordinal);
        Assert.DoesNotContain("Covenant-Setup Sample App", viewModel.TomlPreview, StringComparison.Ordinal);
        Assert.DoesNotContain(@"payload\sample_app.cmd", viewModel.TomlPreview, StringComparison.Ordinal);
        Assert.DoesNotContain(@"\\", viewModel.TomlPreview, StringComparison.Ordinal);
    }

    [Fact]
    public void Packaging_is_disabled_when_covenant_setup_is_missing()
    {
        var viewModel = new MainViewModel(() => null);

        Assert.False(viewModel.HasCovenantSetupTool);
        Assert.False(viewModel.CanPackage);
        Assert.Contains("not found", viewModel.CovenantSetupStatus, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void Packaging_is_enabled_only_when_tool_and_valid_manifest_are_present()
    {
        var viewModel = new MainViewModel(() => new CovenantSetupTool(@"C:\tools\covenant-setup.exe"));

        Assert.True(viewModel.HasCovenantSetupTool);
        Assert.True(viewModel.CanPackage);

        viewModel.AppName = string.Empty;

        Assert.True(viewModel.HasValidationErrors);
        Assert.False(viewModel.CanPackage);
    }
}
