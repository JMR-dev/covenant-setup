using Covenant.Setup.Authoring;
using Xunit;

namespace Covenant.Setup.Authoring.Tests;

public class MainViewModelTests
{
    [Fact]
    public void Constructor_populates_default_manifest_entries()
    {
        var viewModel = new MainViewModel(() => null);

        Assert.Equal("Covenant-Setup Sample App", viewModel.AppName);
        Assert.Contains(viewModel.Directories, directory => directory.Path == @"{LocalAppData}\CovenantSetupSample");
        Assert.Contains(viewModel.Files, file => file.Source == @"payload\sample_app.cmd");
        Assert.Contains(viewModel.PurgePaths, path => path == @"{LocalAppData}\CovenantSetupSample");
        Assert.Contains("app_name = \"Covenant-Setup Sample App\"", viewModel.TomlPreview);
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
        viewModel.ApplyDefaults(resetCollections: true);

        var validation = viewModel.Validate();

        Assert.True(validation.IsValid);
        Assert.Contains(validation.Warnings, warning => warning.Contains("require elevation", StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public void ApplyDefaults_replaces_previous_suggested_entries_without_removing_manual_entries()
    {
        var viewModel = new MainViewModel(() => null);
        viewModel.AddDirectory(@"{LocalAppData}\Manual");
        viewModel.AddFile(@"payload\manual.dll", @"{LocalAppData}\Manual\manual.dll");
        viewModel.AddPurgePath(@"{LocalAppData}\Manual");

        viewModel.AppName = "Renamed App";
        viewModel.ApplicationFolder = "RenamedApp";
        viewModel.PrimaryPayload = @"payload\renamed.exe";
        viewModel.ApplyDefaults(resetCollections: false);

        Assert.DoesNotContain(viewModel.Directories, directory => directory.Path == @"{LocalAppData}\CovenantSetupSample");
        Assert.DoesNotContain(viewModel.Files, file => file.Source == @"payload\sample_app.cmd");
        Assert.DoesNotContain(viewModel.PurgePaths, path => path == @"{LocalAppData}\CovenantSetupSample");
        Assert.Contains(viewModel.Directories, directory => directory.Path == @"{LocalAppData}\RenamedApp");
        Assert.Contains(viewModel.Files, file => file.Source == @"payload\renamed.exe");
        Assert.Contains(viewModel.PurgePaths, path => path == @"{LocalAppData}\RenamedApp");
        Assert.Contains(viewModel.Directories, directory => directory.Path == @"{LocalAppData}\Manual");
        Assert.Contains(viewModel.Files, file => file.Source == @"payload\manual.dll");
        Assert.Contains(viewModel.PurgePaths, path => path == @"{LocalAppData}\Manual");
        Assert.DoesNotContain("CovenantSetupSample", viewModel.TomlPreview, StringComparison.Ordinal);
        Assert.DoesNotContain("Covenant-Setup Sample App", viewModel.TomlPreview, StringComparison.Ordinal);
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
