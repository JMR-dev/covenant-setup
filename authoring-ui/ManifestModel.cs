namespace Covenant.Setup.Authoring;

internal static class ManifestTokens
{
    public static readonly string[] KnownPathTokens =
    [
        "{LocalAppData}",
        "{ProgramFilesX64}",
        "{ProgramFilesX86}",
        "{Desktop}"
    ];
}

internal sealed class ManifestDocument
{
    public string AppName { get; init; } = string.Empty;
    public IReadOnlyList<string> Directories { get; init; } = [];
    public IReadOnlyList<FileSpec> Files { get; init; } = [];
    public IReadOnlyList<RegistrySpec> Registry { get; init; } = [];
    public IReadOnlyList<ShortcutSpec> Shortcuts { get; init; } = [];
    public IReadOnlyList<ScriptSpec> Scripts { get; init; } = [];
    public PurgeSpec Purge { get; init; } = new();
}

internal sealed record FileSpec(string Source, string Destination)
{
    public override string ToString() => $"{Source} -> {Destination}";
}

internal sealed record RegistrySpec(string Key, string Name, string Value)
{
    public override string ToString() => $"{Key}\\{Name} = {Value}";
}

internal sealed record ShortcutSpec(
    string Path,
    string Target,
    string? Arguments,
    string? WorkingDirectory,
    string? Description)
{
    public override string ToString() => $"{Path} -> {Target}";
}

internal sealed record ScriptSpec(
    string Command,
    IReadOnlyList<string> Args,
    string? WorkingDirectory)
{
    public override string ToString()
    {
        var args = Args.Count == 0 ? string.Empty : " " + string.Join(" ", Args);
        return Command + args;
    }
}

internal sealed class PurgeSpec
{
    public IReadOnlyList<string> RegistryBranches { get; init; } = [];
    public IReadOnlyList<string> Paths { get; init; } = [];
}

internal sealed record ValidationResult(
    IReadOnlyList<string> Errors,
    IReadOnlyList<string> Warnings)
{
    public bool IsValid => Errors.Count == 0;
}
