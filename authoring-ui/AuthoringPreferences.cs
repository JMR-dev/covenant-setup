using Microsoft.UI.Xaml;

namespace Covenant.Setup.Authoring;

internal static class AuthoringPreferences
{
    private const string ThemeKey = "theme";

    public static string PreferencesPath { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "Covenant Setup Authoring",
        "preferences.ini");

    public static ElementTheme? LoadTheme() => LoadTheme(PreferencesPath);

    internal static ElementTheme? LoadTheme(string path)
    {
        if (!File.Exists(path))
        {
            return null;
        }

        foreach (var line in File.ReadLines(path))
        {
            var trimmed = line.Trim();
            if (trimmed.Length == 0 || trimmed.StartsWith('#') || trimmed.StartsWith(';') || trimmed.StartsWith('['))
            {
                continue;
            }

            var separator = trimmed.IndexOf('=');
            if (separator < 0)
            {
                continue;
            }

            var key = trimmed[..separator].Trim();
            var value = trimmed[(separator + 1)..].Trim();
            if (string.Equals(key, ThemeKey, StringComparison.OrdinalIgnoreCase))
            {
                return ParseTheme(value);
            }
        }

        return null;
    }

    public static void SaveTheme(ElementTheme theme) => SaveTheme(theme, PreferencesPath);

    internal static void SaveTheme(ElementTheme theme, string path)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, $"[preferences]{Environment.NewLine}{ThemeKey}={FormatTheme(theme)}{Environment.NewLine}");
    }

    internal static ElementTheme? ParseTheme(string value) =>
        value.Equals("dark", StringComparison.OrdinalIgnoreCase)
            ? ElementTheme.Dark
            : value.Equals("light", StringComparison.OrdinalIgnoreCase)
                ? ElementTheme.Light
                : null;

    private static string FormatTheme(ElementTheme theme) =>
        theme == ElementTheme.Dark ? "dark" : "light";
}
