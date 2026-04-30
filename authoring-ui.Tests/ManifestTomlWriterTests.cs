using Covenant.Setup.Authoring;
using Xunit;

namespace Covenant.Setup.Authoring.Tests;

public class ManifestTomlWriterTests
{
    [Fact]
    public void Write_emits_current_manifest_schema()
    {
        var document = new ManifestDocument
        {
            AppName = "Sample App",
            Directories =
            [
                new DirectorySpec(@"{LocalAppData}\Sample")
            ],
            Files =
            [
                new FileSpec(@"payload\app.exe", @"{LocalAppData}\Sample\bin\app.exe")
            ],
            Registry =
            [
                new RegistrySpec(@"HKCU\Software\Sample", "InstallRoot", @"{LocalAppData}\Sample")
            ],
            Shortcuts =
            [
                new ShortcutSpec(
                    @"{Desktop}\Sample App.lnk",
                    @"{LocalAppData}\Sample\bin\app.exe",
                    "--profile default",
                    @"{LocalAppData}\Sample",
                    "Launch Sample App")
            ],
            Scripts =
            [
                new ScriptSpec(
                    "powershell",
                    ["-ExecutionPolicy", "Bypass", "-File", @"payload\post_install.ps1"],
                    @"{LocalAppData}\Sample")
            ],
            Purge = new PurgeSpec
            {
                RegistryBranches = [@"HKCU\Software\Sample"],
                Paths = [@"{LocalAppData}\Sample"]
            }
        };

        var toml = Normalize(ManifestTomlWriter.Write(document));

        const string expected = """
app_name = "Sample App"

[[directories]]
path = "{LocalAppData}\\Sample"

[[files]]
source = "payload\\app.exe"
destination = "{LocalAppData}\\Sample\\bin\\app.exe"

[[registry]]
key = "HKCU\\Software\\Sample"
name = "InstallRoot"
value = "{LocalAppData}\\Sample"

[[shortcuts]]
path = "{Desktop}\\Sample App.lnk"
target = "{LocalAppData}\\Sample\\bin\\app.exe"
arguments = "--profile default"
working_directory = "{LocalAppData}\\Sample"
description = "Launch Sample App"

[[scripts]]
command = "powershell"
args = [
  "-ExecutionPolicy",
  "Bypass",
  "-File",
  "payload\\post_install.ps1"
]
working_directory = "{LocalAppData}\\Sample"

[purge]
registry_branches = ["HKCU\\Software\\Sample"]
paths = ["{LocalAppData}\\Sample"]

""";
        Assert.Equal(Normalize(expected), toml);
    }

    [Fact]
    public void Write_escapes_toml_strings()
    {
        var document = new ManifestDocument
        {
            AppName = "Quoted \"App\"\nTabbed\tPath",
            Directories = [new DirectorySpec(@"C:\Apps\Sample")],
            Purge = new PurgeSpec()
        };

        var toml = Normalize(ManifestTomlWriter.Write(document));

        Assert.Contains("app_name = \"Quoted \\\"App\\\"\\nTabbed\\tPath\"", toml);
        Assert.Contains("path = \"C:\\\\Apps\\\\Sample\"", toml);
    }

    private static string Normalize(string value) => value.Replace("\r\n", "\n");
}
