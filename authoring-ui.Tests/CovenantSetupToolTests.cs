using Covenant.Setup.Authoring;
using Xunit;

namespace Covenant.Setup.Authoring.Tests;

public class CovenantSetupToolTests
{
    [Fact]
    public void Find_returns_first_existing_covenant_setup_exe()
    {
        using var temp = new TempDirectory();
        var missing = Path.Combine(temp.Path, "missing");
        var found = Path.Combine(temp.Path, "found");
        Directory.CreateDirectory(missing);
        Directory.CreateDirectory(found);
        var exe = Path.Combine(found, "covenant-setup.exe");
        File.WriteAllText(exe, string.Empty);

        var tool = CovenantSetupToolLocator.Find([missing, found]);

        Assert.NotNull(tool);
        Assert.Equal(exe, tool.Path);
    }

    [Fact]
    public void Find_returns_null_when_no_candidate_contains_exe()
    {
        using var temp = new TempDirectory();
        var candidate = Path.Combine(temp.Path, "candidate");
        Directory.CreateDirectory(candidate);

        var tool = CovenantSetupToolLocator.Find([candidate]);

        Assert.Null(tool);
    }

    [Fact]
    public void CreateStartInfo_builds_expected_package_command()
    {
        var tool = new CovenantSetupTool(@"C:\tools\covenant-setup.exe");
        var manifest = @"C:\src\sampleapp\SampleApp-install.toml";
        var output = @"C:\out dir";

        var startInfo = CovenantSetupPackager.CreateStartInfo(tool, manifest, output);

        Assert.Equal(tool.Path, startInfo.FileName);
        Assert.Equal(@"C:\src\sampleapp", startInfo.WorkingDirectory);
        Assert.False(startInfo.UseShellExecute);
        Assert.True(startInfo.CreateNoWindow);
        Assert.True(startInfo.RedirectStandardOutput);
        Assert.True(startInfo.RedirectStandardError);
        Assert.Equal(["--json", "package", manifest, "--output", output], startInfo.ArgumentList);
    }

    [Fact]
    public void CreateStartInfo_uses_current_directory_for_relative_manifest()
    {
        var tool = new CovenantSetupTool(@"C:\tools\covenant-setup.exe");

        var startInfo = CovenantSetupPackager.CreateStartInfo(tool, "install.toml", "dist");

        Assert.Equal(Environment.CurrentDirectory, startInfo.WorkingDirectory);
    }
}

internal sealed class TempDirectory : IDisposable
{
    public TempDirectory()
    {
        Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "covenant-authoring-tests-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(Path);
    }

    public string Path { get; }

    public void Dispose()
    {
        try
        {
            Directory.Delete(Path, recursive: true);
        }
        catch
        {
        }
    }
}
