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
        ApplyDefaults(resetCollections: true);
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string AppName
    {
        get => _appName;
        set => SetProperty(ref _appName, value);
    }

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

    public ObservableCollection<DirectorySpec> Directories { get; } = [];
    public ObservableCollection<FileSpec> Files { get; } = [];
    public ObservableCollection<RegistrySpec> Registry { get; } = [];
    public ObservableCollection<ShortcutSpec> Shortcuts { get; } = [];
    public ObservableCollection<ScriptSpec> Scripts { get; } = [];
    public ObservableCollection<string> PurgePaths { get; } = [];
    public ObservableCollection<string> PurgeRegistryBranches { get; } = [];

    public void ApplyDefaults(bool resetCollections)
    {
        if (resetCollections)
        {
            Directories.Clear();
            Files.Clear();
            Registry.Clear();
            Shortcuts.Clear();
            Scripts.Clear();
            PurgePaths.Clear();
            PurgeRegistryBranches.Clear();
        }

        var folder = string.IsNullOrWhiteSpace(ApplicationFolder)
            ? SanitizeIdentifier(AppName)
            : SanitizeIdentifier(ApplicationFolder);
        ApplicationFolder = folder;

        var root = InstallRootToken.TrimEnd('\\') + "\\" + folder;
        AddUnique(Directories, new DirectorySpec(root));
        AddUnique(Directories, new DirectorySpec(root + @"\bin"));
        AddUnique(PurgePaths, root);

        var registryBranch = @"HKCU\Software\" + folder;
        AddUnique(PurgeRegistryBranches, registryBranch);
        AddUnique(Registry, new RegistrySpec(registryBranch, "InstallRoot", root));

        if (!string.IsNullOrWhiteSpace(PrimaryPayload))
        {
            var fileName = Path.GetFileName(PrimaryPayload.Trim());
            if (!string.IsNullOrWhiteSpace(fileName))
            {
                var destination = root + @"\bin\" + fileName;
                AddUnique(Files, new FileSpec(PrimaryPayload.Trim(), destination));
                AddUnique(Shortcuts, new ShortcutSpec(
                    @"{Desktop}\" + AppName.Trim() + ".lnk",
                    destination,
                    null,
                    root,
                    "Launch " + AppName.Trim()));
            }
        }

        RefreshPreview();
    }

    public void RefreshCovenantSetupTool()
    {
        CovenantSetupTool = _locateCovenantSetupTool();
        CovenantSetupStatus = CovenantSetupTool is null
            ? "covenant-setup.exe was not found. Packaging is disabled."
            : "Packaging enabled: " + CovenantSetupTool.Path;
    }

    public void AddDirectory(string path)
    {
        AddTrimmed(Directories, path, value => new DirectorySpec(value));
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

    public ManifestDocument BuildDocument() => new()
    {
        AppName = AppName.Trim(),
        Directories = Directories.ToArray(),
        Files = Files.ToArray(),
        Registry = Registry.ToArray(),
        Shortcuts = Shortcuts.ToArray(),
        Scripts = Scripts.ToArray(),
        Purge = new PurgeSpec
        {
            Paths = PurgePaths.ToArray(),
            RegistryBranches = PurgeRegistryBranches.ToArray()
        }
    };

    public ValidationResult Validate()
    {
        var errors = new List<string>();
        var warnings = new List<string>();

        if (string.IsNullOrWhiteSpace(AppName))
        {
            errors.Add("App name is required.");
        }

        if (Directories.Count + Files.Count + Registry.Count + Shortcuts.Count + Scripts.Count == 0)
        {
            errors.Add("Add at least one install action.");
        }

        foreach (var file in Files)
        {
            if (Path.IsPathRooted(file.Source))
            {
                warnings.Add($"File source '{file.Source}' is rooted; package sources are usually relative to the manifest folder.");
            }
        }

        foreach (var registry in Registry)
        {
            if (!IsSupportedRegistryRoot(registry.Key))
            {
                errors.Add($"Registry key '{registry.Key}' must start with HKCU\\ or HKLM\\.");
            }
        }

        foreach (var branch in PurgeRegistryBranches)
        {
            if (!IsSupportedRegistryRoot(branch))
            {
                errors.Add($"Purge registry branch '{branch}' must start with HKCU\\ or HKLM\\.");
            }
        }

        if (Registry.Any(entry => entry.Key.StartsWith(@"HKLM\", StringComparison.OrdinalIgnoreCase)) ||
            PurgeRegistryBranches.Any(branch => branch.StartsWith(@"HKLM\", StringComparison.OrdinalIgnoreCase)) ||
            Directories.Any(directory => RequiresAdminPath(directory.Path)) ||
            Files.Any(file => RequiresAdminPath(file.Destination)) ||
            Shortcuts.Any(shortcut => RequiresAdminPath(shortcut.Path)) ||
            PurgePaths.Any(RequiresAdminPath))
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

    private static string SanitizeIdentifier(string value)
    {
        var sanitized = IdentifierRegex().Replace(value.Trim(), "_").Trim('_');
        return string.IsNullOrWhiteSpace(sanitized) ? "covenant_setup" : sanitized;
    }

    [GeneratedRegex(@"[^A-Za-z0-9_-]+")]
    private static partial Regex IdentifierRegex();
}
