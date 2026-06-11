using System.Collections.ObjectModel;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Text.RegularExpressions;

namespace Covenant.Setup.Authoring;

internal sealed partial class MainViewModel : INotifyPropertyChanged
{
    private string _appName = "Covenant-Setup Sample App";
    private string _installRootToken = "{ProgramFilesX64}";
    private string _applicationFolder = "CovenantSetupSample";
    private string _primaryPayload = @"payload\sample_app.cmd";
    private string _outputDirectory = Path.Combine(Environment.CurrentDirectory, "dist");
    private string? _shortcutDescriptionOverride;
    private CovenantSetupTool? _covenantSetupTool;
    private string _covenantSetupStatus = "covenant-setup.exe was not found. Packaging is disabled.";
    private string _tomlPreview = string.Empty;
    private string _validationSummary = string.Empty;
    private bool _hasValidationErrors;
    private readonly Func<CovenantSetupTool?> _locateCovenantSetupTool;

    public MainViewModel()
        : this(CovenantSetupToolLocator.Find)
    {
    }

    internal MainViewModel(Func<CovenantSetupTool?> locateCovenantSetupTool)
    {
        _locateCovenantSetupTool = locateCovenantSetupTool;
        Directories.CollectionChanged += CollectionChanged;
        Files.CollectionChanged += CollectionChanged;
        Registry.CollectionChanged += CollectionChanged;
        Shortcuts.CollectionChanged += CollectionChanged;
        Scripts.CollectionChanged += CollectionChanged;
        PurgePaths.CollectionChanged += CollectionChanged;
        PurgeRegistryBranches.CollectionChanged += CollectionChanged;
        RefreshCovenantSetupTool();
        RefreshPreview();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string AppName
    {
        get => _appName;
        set => SetProperty(ref _appName, value);
    }

    public string ExpectedManifestFileName => ManifestFileName(AppName);

    public string ManifestSubtitle => "Create " + ExpectedManifestFileName + " for covenant-setup";

    public string InstallRootToken
    {
        get => _installRootToken;
        set => SetProperty(ref _installRootToken, value);
    }

    public string DirectoryPlaceholder => $"{InstallRootToken}\\Vendor\\App";

    public string FileDestinationPlaceholder => $"{InstallRootToken}\\Vendor\\App\\bin\\app.exe";

    public string ApplicationFolder
    {
        get => _applicationFolder;
        set => SetProperty(ref _applicationFolder, value);
    }

    public string PrimaryPayload
    {
        get => _primaryPayload;
        set => SetProperty(ref _primaryPayload, value);
    }

    public string ShortcutDescription
    {
        get => EffectiveShortcutDescription();
        set
        {
            var normalized = string.IsNullOrWhiteSpace(value) ? null : value;
            var defaultDescription = DefaultShortcutDescription(AppName.Trim());
            var overrideValue = string.Equals(normalized, defaultDescription, StringComparison.Ordinal)
                ? null
                : normalized;

            if (string.Equals(_shortcutDescriptionOverride, overrideValue, StringComparison.Ordinal))
            {
                return;
            }

            _shortcutDescriptionOverride = overrideValue;
            OnPropertyChanged();
            NotifyGeneratedManifestFieldsChanged();
            RefreshPreview();
        }
    }

    public string OutputDirectory
    {
        get => _outputDirectory;
        set => SetProperty(ref _outputDirectory, value, refreshPreview: false);
    }

    public CovenantSetupTool? CovenantSetupTool
    {
        get => _covenantSetupTool;
        private set
        {
            if (SetProperty(ref _covenantSetupTool, value, refreshPreview: false))
            {
                PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(HasCovenantSetupTool)));
                PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(CovenantSetupPath)));
                PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(CanPackage)));
            }
        }
    }

    public bool HasCovenantSetupTool => CovenantSetupTool is not null;

    public string CovenantSetupPath => CovenantSetupTool?.Path ?? string.Empty;

    public string CovenantSetupStatus
    {
        get => _covenantSetupStatus;
        private set => SetProperty(ref _covenantSetupStatus, value, refreshPreview: false);
    }

    public bool CanPackage =>
        HasCovenantSetupTool &&
        !HasValidationErrors &&
        !string.IsNullOrWhiteSpace(OutputDirectory);

    public string TomlPreview
    {
        get => _tomlPreview;
        private set => SetProperty(ref _tomlPreview, value, refreshPreview: false);
    }

    public string ValidationSummary
    {
        get => _validationSummary;
        private set => SetProperty(ref _validationSummary, value, refreshPreview: false);
    }

    public bool HasValidationErrors
    {
        get => _hasValidationErrors;
        private set
        {
            if (SetProperty(ref _hasValidationErrors, value, refreshPreview: false))
            {
                PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(CanPackage)));
            }
        }
    }

    public string ShortcutPath => GeneratedShortcut()?.Path ?? string.Empty;

    public string ShortcutTarget => GeneratedShortcut()?.Target ?? string.Empty;

    public string ShortcutWorkingDirectory => GeneratedShortcut()?.WorkingDirectory ?? string.Empty;

    public string PurgePathsPreview => JoinLines(BuildDocument().Purge.Paths);

    public string PurgeRegistryBranchesPreview => JoinLines(BuildDocument().Purge.RegistryBranches);

    public ObservableCollection<string> Directories { get; } = [];
    public ObservableCollection<FileSpec> Files { get; } = [];
    public ObservableCollection<RegistrySpec> Registry { get; } = [];
    public ObservableCollection<ShortcutSpec> Shortcuts { get; } = [];
    public ObservableCollection<ScriptSpec> Scripts { get; } = [];
    public ObservableCollection<string> PurgePaths { get; } = [];
    public ObservableCollection<string> PurgeRegistryBranches { get; } = [];

    public void RefreshCovenantSetupTool()
    {
        SetCovenantSetupTool(_locateCovenantSetupTool());
    }

    public void SetCovenantSetupTool(CovenantSetupTool? tool)
    {
        CovenantSetupTool = tool;
        CovenantSetupStatus = CovenantSetupTool is null
            ? "covenant-setup.exe was not found. Packaging is disabled."
            : "Packaging enabled: " + CovenantSetupTool.Path;
    }

    public void RejectCovenantSetupTool(string message)
    {
        CovenantSetupTool = null;
        CovenantSetupStatus = message;
    }

    public void AddDirectory(string path)
    {
        AddTrimmed(Directories, path, value => value);
    }

    public void AddFile(string source, string destination)
    {
        if (!string.IsNullOrWhiteSpace(source) && !string.IsNullOrWhiteSpace(destination))
        {
            AddUnique(Files, new FileSpec(source.Trim(), destination.Trim()));
        }
    }

    public void AddRegistry(string key, string name, string value)
    {
        if (!string.IsNullOrWhiteSpace(key) &&
            !string.IsNullOrWhiteSpace(name) &&
            !string.IsNullOrWhiteSpace(value))
        {
            AddUnique(Registry, new RegistrySpec(key.Trim(), name.Trim(), value.Trim()));
        }
    }

    public void AddShortcut(
        string path,
        string target,
        string? arguments,
        string? workingDirectory,
        string? description)
    {
        if (!string.IsNullOrWhiteSpace(path) && !string.IsNullOrWhiteSpace(target))
        {
            AddUnique(Shortcuts, new ShortcutSpec(
                path.Trim(),
                target.Trim(),
                NullIfWhiteSpace(arguments),
                NullIfWhiteSpace(workingDirectory),
                NullIfWhiteSpace(description)));
        }
    }

    public void AddScript(string command, IReadOnlyList<string> args, string? workingDirectory)
    {
        if (!string.IsNullOrWhiteSpace(command))
        {
            AddUnique(Scripts, new ScriptSpec(
                command.Trim(),
                args.Where(arg => !string.IsNullOrWhiteSpace(arg)).Select(arg => arg.Trim()).ToArray(),
                NullIfWhiteSpace(workingDirectory)));
        }
    }

    public void AddPurgePath(string path)
    {
        AddTrimmed(PurgePaths, path, value => value);
    }

    public void AddPurgeRegistryBranch(string branch)
    {
        AddTrimmed(PurgeRegistryBranches, branch, value => value);
    }

    public ManifestDocument BuildDocument()
    {
        var template = BuildTemplateEntries();
        var directories = CombineUnique(template.Directories, Directories);
        var files = CombineUnique(template.Files, Files);
        var registry = CombineUnique(template.Registry, Registry);
        var shortcuts = CombineUnique(BuildShortcutEntries(files), Shortcuts);
        var purgePaths = CombineUnique(BuildPurgePaths(files, registry), PurgePaths);
        return new ManifestDocument
        {
            AppName = AppName.Trim(),
            Directories = directories,
            Files = files,
            Registry = registry,
            Shortcuts = shortcuts,
            Scripts = Scripts.ToArray(),
            Purge = new PurgeSpec
            {
                Paths = purgePaths,
                RegistryBranches = CombineUnique(registry.Select(entry => entry.Key), PurgeRegistryBranches)
            }
        };
    }

    public ValidationResult Validate() => Validate(BuildDocument());

    private ValidationResult Validate(ManifestDocument document)
    {
        var errors = new List<string>();
        var warnings = new List<string>();

        if (string.IsNullOrWhiteSpace(AppName))
        {
            errors.Add("App name is required.");
        }
        else if (AppName.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0)
        {
            errors.Add("App name cannot contain characters that are invalid in Windows file names.");
        }

        if (document.Directories.Count + document.Files.Count + document.Registry.Count + document.Shortcuts.Count + document.Scripts.Count == 0)
        {
            errors.Add("Add at least one install action.");
        }

        ValidateManifestSpacing(document, errors);
        ValidateNoWhitespace(errors, "Application target installation folder", ApplicationFolder);

        foreach (var file in document.Files)
        {
            if (Path.IsPathRooted(file.Source))
            {
                warnings.Add($"File source '{file.Source}' is rooted; package sources are usually relative to the manifest folder.");
            }
        }

        foreach (var registry in document.Registry)
        {
            if (!IsSupportedRegistryRoot(registry.Key))
            {
                errors.Add($"Registry key '{registry.Key}' must start with HKCU\\ or HKLM\\.");
            }
        }

        foreach (var branch in document.Purge.RegistryBranches)
        {
            if (!IsSupportedRegistryRoot(branch))
            {
                errors.Add($"Purge registry branch '{branch}' must start with HKCU\\ or HKLM\\.");
            }
        }

        if (document.Registry.Any(entry => entry.Key.StartsWith(@"HKLM\", StringComparison.OrdinalIgnoreCase)) ||
            document.Purge.RegistryBranches.Any(branch => branch.StartsWith(@"HKLM\", StringComparison.OrdinalIgnoreCase)) ||
            document.Directories.Any(RequiresAdminPath) ||
            document.Files.Any(file => RequiresAdminPath(file.Destination)) ||
            document.Shortcuts.Any(shortcut => RequiresAdminPath(shortcut.Path)) ||
            document.Purge.Paths.Any(RequiresAdminPath))
        {
            warnings.Add("This manifest appears to require elevation because it targets Program Files or HKLM.");
        }

        return new ValidationResult(errors, warnings);
    }

    private void RefreshPreview()
    {
        var document = BuildDocument();
        TomlPreview = ManifestTomlWriter.Write(document);
        var validation = Validate(document);
        HasValidationErrors = !validation.IsValid;
        ValidationSummary = validation.Errors.Count switch
        {
            > 0 => string.Join(Environment.NewLine, validation.Errors),
            _ when validation.Warnings.Count > 0 => string.Join(Environment.NewLine, validation.Warnings),
            _ => "Manifest is ready to save."
        };
    }

    private bool SetProperty<T>(
        ref T field,
        T value,
        [CallerMemberName] string? propertyName = null,
        bool refreshPreview = true)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        OnPropertyChanged(propertyName);
        if (propertyName == nameof(AppName))
        {
            OnPropertyChanged(nameof(ExpectedManifestFileName));
            OnPropertyChanged(nameof(ManifestSubtitle));
        }
        if (propertyName == nameof(OutputDirectory))
        {
            OnPropertyChanged(nameof(CanPackage));
        }
        if (propertyName is nameof(AppName)
            or nameof(InstallRootToken)
            or nameof(ApplicationFolder)
            or nameof(PrimaryPayload))
        {
            NotifyGeneratedManifestFieldsChanged();
        }
        if (refreshPreview)
        {
            RefreshPreview();
        }
        return true;
    }

    private void CollectionChanged(object? sender, NotifyCollectionChangedEventArgs args)
    {
        if (ReferenceEquals(sender, Files))
        {
            OnPropertyChanged(nameof(ShortcutPath));
            OnPropertyChanged(nameof(ShortcutTarget));
            OnPropertyChanged(nameof(ShortcutWorkingDirectory));
            OnPropertyChanged(nameof(PurgePathsPreview));
        }
        if (ReferenceEquals(sender, Registry) || ReferenceEquals(sender, PurgePaths))
        {
            OnPropertyChanged(nameof(PurgePathsPreview));
        }
        if (ReferenceEquals(sender, Registry) || ReferenceEquals(sender, PurgeRegistryBranches))
        {
            OnPropertyChanged(nameof(PurgeRegistryBranchesPreview));
        }

        RefreshPreview();
    }

    private void NotifyGeneratedManifestFieldsChanged()
    {
        OnPropertyChanged(nameof(ShortcutDescription));
        OnPropertyChanged(nameof(ShortcutPath));
        OnPropertyChanged(nameof(ShortcutTarget));
        OnPropertyChanged(nameof(ShortcutWorkingDirectory));
        OnPropertyChanged(nameof(PurgePathsPreview));
        OnPropertyChanged(nameof(PurgeRegistryBranchesPreview));
        OnPropertyChanged(nameof(DirectoryPlaceholder));
        OnPropertyChanged(nameof(FileDestinationPlaceholder));
    }

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null)
    {
        if (propertyName is not null)
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
        }
    }

    private static void AddTrimmed<T>(
        ObservableCollection<T> collection,
        string value,
        Func<string, T> factory)
    {
        if (!string.IsNullOrWhiteSpace(value))
        {
            AddUnique(collection, factory(value.Trim()));
        }
    }

    private static void AddUnique<T>(ObservableCollection<T> collection, T item)
    {
        if (!collection.Contains(item))
        {
            collection.Add(item);
        }
    }

    private static string? NullIfWhiteSpace(string? value) =>
        string.IsNullOrWhiteSpace(value) ? null : value.Trim();

    private string EffectiveShortcutDescription() =>
        _shortcutDescriptionOverride ?? DefaultShortcutDescription(AppName.Trim());

    private string ManifestShortcutDescription() =>
        NullIfWhiteSpace(EffectiveShortcutDescription()) ?? DefaultShortcutDescription(AppName.Trim());

    private static string DefaultShortcutDescription(string appName) =>
        string.IsNullOrWhiteSpace(appName) ? "Launch application" : "Launch " + appName;

    private ShortcutSpec? GeneratedShortcut()
    {
        var template = BuildTemplateEntries();
        var files = CombineUnique(template.Files, Files);
        return BuildShortcutEntries(files).FirstOrDefault();
    }

    private static string JoinLines(IEnumerable<string> values) =>
        string.Join(Environment.NewLine, values);

    private IReadOnlyList<ShortcutSpec> BuildShortcutEntries(IReadOnlyList<FileSpec> files)
    {
        var target = files
            .Select(file => file.Destination.Trim())
            .FirstOrDefault(destination => !string.IsNullOrWhiteSpace(destination));

        if (string.IsNullOrWhiteSpace(target))
        {
            return [];
        }

        return
        [
            new ShortcutSpec(
                @"{Desktop}\" + ShortcutFileName(target) + ".lnk",
                target,
                null,
                ParentPath(target),
                ManifestShortcutDescription())
        ];
    }

    private static IReadOnlyList<string> BuildPurgePaths(
        IReadOnlyList<FileSpec> files,
        IReadOnlyList<RegistrySpec> registry)
    {
        var candidates = new List<string>();
        foreach (var entry in registry)
        {
            var value = entry.Value.Trim();
            if (IsManifestPath(value) && !IsUnsafePurgeRoot(value))
            {
                candidates.Add(value);
            }
        }

        foreach (var file in files)
        {
            var parent = ParentPath(file.Destination);
            if (!string.IsNullOrWhiteSpace(parent) && !IsUnsafePurgeRoot(parent))
            {
                candidates.Add(parent);
            }
        }

        return RemoveCoveredChildPaths(CombineUnique(candidates, Array.Empty<string>()));
    }

    private string ShortcutFileName(string target)
    {
        var fileName = FileNameWithoutExtension(target);
        var sanitized = SanitizeIdentifier(fileName);
        if (!string.IsNullOrWhiteSpace(sanitized))
        {
            return sanitized;
        }

        var fallbackName = string.IsNullOrWhiteSpace(ApplicationFolder)
            ? AppName
            : ApplicationFolder;
        return SanitizeIdentifier(fallbackName);
    }

    private static string FileNameWithoutExtension(string path)
    {
        var fileName = path.Trim().TrimEnd('\\');
        var separatorIndex = fileName.LastIndexOf('\\');
        if (separatorIndex >= 0 && separatorIndex < fileName.Length - 1)
        {
            fileName = fileName[(separatorIndex + 1)..];
        }

        var extensionIndex = fileName.LastIndexOf('.');
        return extensionIndex > 0 ? fileName[..extensionIndex] : fileName;
    }

    private static string? ParentPath(string path)
    {
        var trimmed = path.Trim().TrimEnd('\\');
        var separatorIndex = trimmed.LastIndexOf('\\');
        if (separatorIndex < 0)
        {
            return null;
        }

        if (separatorIndex == 2 && trimmed[1] == ':')
        {
            return trimmed[..(separatorIndex + 1)];
        }

        return separatorIndex == 0 ? null : trimmed[..separatorIndex];
    }

    private static bool IsManifestPath(string value) =>
        ManifestTokens.KnownPathTokens.Any(token =>
            value.StartsWith(token + @"\", StringComparison.OrdinalIgnoreCase)) ||
        Path.IsPathRooted(value);

    private static bool IsUnsafePurgeRoot(string value)
    {
        var normalized = value.Trim().TrimEnd('\\');
        if (ManifestTokens.KnownPathTokens.Any(token =>
            string.Equals(normalized, token, StringComparison.OrdinalIgnoreCase)))
        {
            return true;
        }

        return normalized.Length == 2 &&
            normalized[1] == ':' &&
            IsAsciiLetter(normalized[0]);
    }

    private static bool IsAsciiLetter(char value) =>
        value is >= 'A' and <= 'Z' or >= 'a' and <= 'z';

    private static IReadOnlyList<string> RemoveCoveredChildPaths(IReadOnlyList<string> paths) =>
        paths
            .Where(path => !paths.Any(parent =>
                !string.Equals(parent, path, StringComparison.OrdinalIgnoreCase) &&
                IsParentPath(parent, path)))
            .ToArray();

    private static bool IsParentPath(string parent, string child)
    {
        var normalizedParent = parent.TrimEnd('\\');
        return child.Length > normalizedParent.Length &&
            child.StartsWith(normalizedParent + "\\", StringComparison.OrdinalIgnoreCase);
    }

    private static bool IsSupportedRegistryRoot(string value) =>
        value.StartsWith(@"HKCU\", StringComparison.OrdinalIgnoreCase) ||
        value.StartsWith(@"HKLM\", StringComparison.OrdinalIgnoreCase);

    private static bool RequiresAdminPath(string value) =>
        value.StartsWith(@"{ProgramFilesX64}\", StringComparison.OrdinalIgnoreCase) ||
        value.StartsWith(@"{ProgramFilesX86}\", StringComparison.OrdinalIgnoreCase) ||
        value.StartsWith(@"C:\Program Files\", StringComparison.OrdinalIgnoreCase) ||
        value.StartsWith(@"C:\Program Files (x86)\", StringComparison.OrdinalIgnoreCase);

    private static void ValidateManifestSpacing(ManifestDocument document, List<string> errors)
    {
        foreach (var directory in document.Directories)
        {
            ValidateNoWhitespace(errors, "Directory path", directory);
        }

        foreach (var file in document.Files)
        {
            ValidateNoWhitespace(errors, "File source", file.Source);
            ValidateNoWhitespace(errors, "File destination", file.Destination);
        }

        foreach (var registry in document.Registry)
        {
            ValidateNoWhitespace(errors, "Registry key", registry.Key);
            ValidateNoWhitespace(errors, "Registry name", registry.Name);
            ValidateNoWhitespace(errors, "Registry value", registry.Value);
        }

        foreach (var shortcut in document.Shortcuts)
        {
            ValidateNoWhitespace(errors, "Shortcut path", shortcut.Path);
            ValidateNoWhitespace(errors, "Shortcut target", shortcut.Target);
            ValidateOptionalNoWhitespace(errors, "Shortcut arguments", shortcut.Arguments);
            ValidateOptionalNoWhitespace(errors, "Shortcut working directory", shortcut.WorkingDirectory);
        }

        foreach (var script in document.Scripts)
        {
            ValidateNoWhitespace(errors, "Script command", script.Command);
            for (var index = 0; index < script.Args.Count; index++)
            {
                ValidateNoWhitespace(errors, $"Script argument {index + 1}", script.Args[index]);
            }
            ValidateOptionalNoWhitespace(errors, "Script working directory", script.WorkingDirectory);
        }

        foreach (var branch in document.Purge.RegistryBranches)
        {
            ValidateNoWhitespace(errors, "Purge registry branch", branch);
        }

        foreach (var path in document.Purge.Paths)
        {
            ValidateNoWhitespace(errors, "Purge path", path);
        }
    }

    private static void ValidateOptionalNoWhitespace(List<string> errors, string fieldName, string? value)
    {
        if (!string.IsNullOrWhiteSpace(value))
        {
            ValidateNoWhitespace(errors, fieldName, value);
        }
    }

    private static void ValidateNoWhitespace(List<string> errors, string fieldName, string value)
    {
        if (ContainsWhitespace(value))
        {
            errors.Add($"{fieldName} cannot contain spaces or other whitespace.");
        }
    }

    private static string SanitizeIdentifier(string value)
    {
        var sanitized = IdentifierRegex().Replace(value.Trim(), "_").Trim('_');
        return string.IsNullOrWhiteSpace(sanitized) ? "covenant_setup" : sanitized;
    }

    internal static string ManifestFileName(string appName)
    {
        var name = new string(appName.Trim().Where(ch => !char.IsWhiteSpace(ch)).ToArray());
        return string.IsNullOrWhiteSpace(name) ? "install.toml" : name + "-install.toml";
    }

    internal bool IsExpectedManifestPath(string path) =>
        string.Equals(
            Path.GetFileName(path),
            ExpectedManifestFileName,
            StringComparison.OrdinalIgnoreCase) &&
        !ContainsWhitespace(path);

    internal static bool ContainsWhitespace(string value) =>
        value.Any(char.IsWhiteSpace);

    [GeneratedRegex(@"[^A-Za-z0-9_-]+")]
    private static partial Regex IdentifierRegex();

    private TemplateManifestEntries BuildTemplateEntries()
    {
        var appName = AppName.Trim();
        var folder = string.IsNullOrWhiteSpace(ApplicationFolder)
            ? SanitizeIdentifier(appName)
            : SanitizeIdentifier(ApplicationFolder);
        var root = InstallRootToken.TrimEnd('\\') + "\\" + folder;
        var registryBranch = @"HKCU\Software\" + folder;
        var files = new List<FileSpec>();

        if (!string.IsNullOrWhiteSpace(PrimaryPayload))
        {
            var source = PrimaryPayload.Trim();
            var fileName = Path.GetFileName(source);
            if (!string.IsNullOrWhiteSpace(fileName))
            {
                var destination = root + @"\bin\" + fileName;
                files.Add(new FileSpec(source, destination));
            }
        }

        return new TemplateManifestEntries(
            [root, root + @"\bin"],
            files,
            [new RegistrySpec(registryBranch, "InstallRoot", root)]);
    }

    private static IReadOnlyList<T> CombineUnique<T>(IEnumerable<T> first, IEnumerable<T> second)
    {
        var values = new List<T>();
        foreach (var item in first.Concat(second))
        {
            if (!values.Contains(item))
            {
                values.Add(item);
            }
        }

        return values;
    }

    private sealed record TemplateManifestEntries(
        IReadOnlyList<string> Directories,
        IReadOnlyList<FileSpec> Files,
        IReadOnlyList<RegistrySpec> Registry);
}
