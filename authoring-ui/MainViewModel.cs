using System.Collections.ObjectModel;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Text.RegularExpressions;

namespace Covenant.Setup.Authoring;

internal sealed partial class MainViewModel : INotifyPropertyChanged
{
    private string _appName = "Covenant-Setup Sample App";
    private string _installRootToken = "{LocalAppData}";
    private string _applicationFolder = "CovenantSetupSample";
    private string _primaryPayload = @"payload\sample_app.cmd";
    private string _outputDirectory = Path.Combine(Environment.CurrentDirectory, "dist");
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
                PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(CanPackage)));
            }
        }
    }

    public bool HasCovenantSetupTool => CovenantSetupTool is not null;

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

    public ObservableCollection<string> Directories { get; } = [];
    public ObservableCollection<FileSpec> Files { get; } = [];
    public ObservableCollection<RegistrySpec> Registry { get; } = [];
    public ObservableCollection<ShortcutSpec> Shortcuts { get; } = [];
    public ObservableCollection<ScriptSpec> Scripts { get; } = [];
    public ObservableCollection<string> PurgePaths { get; } = [];
    public ObservableCollection<string> PurgeRegistryBranches { get; } = [];

    public void RefreshCovenantSetupTool()
    {
        CovenantSetupTool = _locateCovenantSetupTool();
        CovenantSetupStatus = CovenantSetupTool is null
            ? "covenant-setup.exe was not found. Packaging is disabled."
            : "Packaging enabled: " + CovenantSetupTool.Path;
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
        return new ManifestDocument
        {
            AppName = AppName.Trim(),
            Directories = CombineUnique(template.Directories, Directories),
            Files = CombineUnique(template.Files, Files),
            Registry = CombineUnique(template.Registry, Registry),
            Shortcuts = CombineUnique(template.Shortcuts, Shortcuts),
            Scripts = Scripts.ToArray(),
            Purge = new PurgeSpec
            {
                Paths = CombineUnique(template.PurgePaths, PurgePaths),
                RegistryBranches = CombineUnique(template.PurgeRegistryBranches, PurgeRegistryBranches)
            }
        };
    }

    public ValidationResult Validate()
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

        var document = BuildDocument();

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
        TomlPreview = ManifestTomlWriter.Write(BuildDocument());
        var validation = Validate();
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
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
        if (propertyName == nameof(AppName))
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(ExpectedManifestFileName)));
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(ManifestSubtitle)));
        }
        if (propertyName == nameof(OutputDirectory))
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(CanPackage)));
        }
        if (refreshPreview)
        {
            RefreshPreview();
        }
        return true;
    }

    private void CollectionChanged(object? sender, NotifyCollectionChangedEventArgs args)
    {
        RefreshPreview();
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
        var shortcuts = new List<ShortcutSpec>();

        if (!string.IsNullOrWhiteSpace(PrimaryPayload))
        {
            var source = PrimaryPayload.Trim();
            var fileName = Path.GetFileName(source);
            if (!string.IsNullOrWhiteSpace(fileName))
            {
                var destination = root + @"\bin\" + fileName;
                files.Add(new FileSpec(source, destination));
                shortcuts.Add(new ShortcutSpec(
                    @"{Desktop}\" + folder + ".lnk",
                    destination,
                    null,
                    root,
                    "Launch " + appName));
            }
        }

        return new TemplateManifestEntries(
            [root, root + @"\bin"],
            files,
            [new RegistrySpec(registryBranch, "InstallRoot", root)],
            shortcuts,
            [root],
            [registryBranch]);
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
        IReadOnlyList<RegistrySpec> Registry,
        IReadOnlyList<ShortcutSpec> Shortcuts,
        IReadOnlyList<string> PurgePaths,
        IReadOnlyList<string> PurgeRegistryBranches);
}
