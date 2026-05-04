using Covenant.Setup.Authoring;
using Microsoft.UI.Xaml;
using Xunit;

namespace Covenant.Setup.Authoring.Tests;

public class AuthoringPreferencesTests
{
    [Fact]
    public void SaveTheme_writes_preferences_ini_and_LoadTheme_reads_it()
    {
        var path = Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString("N"), "preferences.ini");

        AuthoringPreferences.SaveTheme(ElementTheme.Dark, path);

        Assert.Equal(ElementTheme.Dark, AuthoringPreferences.LoadTheme(path));
        Assert.Contains("theme=dark", File.ReadAllText(path), StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void LoadTheme_returns_null_when_preferences_are_missing()
    {
        var path = Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString("N"), "preferences.ini");

        Assert.Null(AuthoringPreferences.LoadTheme(path));
    }
}
