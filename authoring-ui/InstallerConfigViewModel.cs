using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows.Input;

namespace Covenant.Setup.Authoring;

internal enum InstallerConfigStatusKind
{
    Neutral,
    Success,
    Error
}

internal sealed class InstallerConfigViewModel : INotifyPropertyChanged
{
    private const string MissingToolStatus = "covenant-setup.exe was not found. Packaging is disabled.";
    private const string UnvalidatedToolStatus = "covenant-setup.exe was not validated. Packaging is disabled.";

    private readonly Func<CovenantSetupTool?> _locateCovenantSetupTool;
    private readonly Func<string, CancellationToken, Task<ToolValidationResult>> _validateCovenantSetupTool;
    private CovenantSetupTool? _covenantSetupTool;
    private string _covenantSetupPath = string.Empty;
    private string _outputDirectory = Path.Combine(Environment.CurrentDirectory, "dist");
    private string _statusText = MissingToolStatus;
    private string _messageText = string.Empty;
    private InstallerConfigStatusKind _statusKind = InstallerConfigStatusKind.Error;
    private InstallerConfigStatusKind _messageKind = InstallerConfigStatusKind.Neutral;
    private bool _isCheckingTool;
    private bool _isBuilding;
    private bool _hasManifestValidationErrors;
    private bool _syncingAcceptedToolPath;

    public InstallerConfigViewModel()
        : this(CovenantSetupToolLocator.Find, CovenantSetupToolValidator.ValidateAsync)
    {
    }

    internal InstallerConfigViewModel(
        Func<CovenantSetupTool?> locateCovenantSetupTool,
        Func<string, CancellationToken, Task<ToolValidationResult>> validateCovenantSetupTool)
    {
        _locateCovenantSetupTool = locateCovenantSetupTool;
        _validateCovenantSetupTool = validateCovenantSetupTool;
        RefreshToolCheckCommand = new RelayCommand(RefreshToolCheck, () => !IsBusy);
        ValidateToolPathCommand = new AsyncRelayCommand(ValidateToolPathAsync, () => CanValidateToolPath);
        SaveConfigCommand = new AsyncRelayCommand(SaveConfigAsync, () => CanSaveConfig);
        RefreshToolCheck();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public ICommand RefreshToolCheckCommand { get; }

    public ICommand ValidateToolPathCommand { get; }

    public ICommand SaveConfigCommand { get; }

    public CovenantSetupTool? CovenantSetupTool
    {
        get => _covenantSetupTool;
        private set
        {
            if (SetProperty(ref _covenantSetupTool, value))
            {
                OnPropertyChanged(nameof(HasCovenantSetupTool));
                OnPropertyChanged(nameof(CanSaveConfig));
                RaiseCommandStatesChanged();
            }
        }
    }

    public bool HasCovenantSetupTool => CovenantSetupTool is not null;

    public string CovenantSetupPath
    {
        get => _covenantSetupPath;
        set
        {
            if (!SetProperty(ref _covenantSetupPath, value))
            {
                return;
            }

            OnPropertyChanged(nameof(CanValidateToolPath));
            RaiseCommandStatesChanged();

            if (!_syncingAcceptedToolPath && !IsAcceptedToolPath(value))
            {
                ClearValidatedTool(UnvalidatedToolStatus);
            }
        }
    }

    public string OutputDirectory
    {
        get => _outputDirectory;
        set
        {
            if (SetProperty(ref _outputDirectory, value))
            {
                OnPropertyChanged(nameof(CanSaveConfig));
                RaiseCommandStatesChanged();
            }
        }
    }

    public string StatusText
    {
        get => _statusText;
        private set => SetProperty(ref _statusText, value);
    }

    public InstallerConfigStatusKind StatusKind
    {
        get => _statusKind;
        private set => SetProperty(ref _statusKind, value);
    }

    public string MessageText
    {
        get => _messageText;
        private set => SetProperty(ref _messageText, value);
    }

    public InstallerConfigStatusKind MessageKind
    {
        get => _messageKind;
        private set => SetProperty(ref _messageKind, value);
    }

    public bool IsCheckingTool
    {
        get => _isCheckingTool;
        private set
        {
            if (SetProperty(ref _isCheckingTool, value))
            {
                NotifyBusyStateChanged();
            }
        }
    }

    public bool IsBuilding
    {
        get => _isBuilding;
        set
        {
            if (SetProperty(ref _isBuilding, value))
            {
                NotifyBusyStateChanged();
            }
        }
    }

    public bool IsBusy => IsCheckingTool || IsBuilding;

    public bool HasManifestValidationErrors
    {
        get => _hasManifestValidationErrors;
        private set
        {
            if (SetProperty(ref _hasManifestValidationErrors, value))
            {
                OnPropertyChanged(nameof(CanSaveConfig));
                RaiseCommandStatesChanged();
            }
        }
    }

    public bool CanValidateToolPath =>
        !IsBusy &&
        !string.IsNullOrWhiteSpace(CovenantSetupPath);

    public bool CanSaveConfig =>
        !IsBusy &&
        !HasManifestValidationErrors &&
        !string.IsNullOrWhiteSpace(CovenantSetupPath) &&
        !string.IsNullOrWhiteSpace(OutputDirectory);

    public void RefreshToolCheck()
    {
        if (IsBusy)
        {
            return;
        }

        var tool = _locateCovenantSetupTool();
        if (tool is null)
        {
            ClearValidatedTool(MissingToolStatus);
            SetMessage(string.Empty, InstallerConfigStatusKind.Neutral);
            return;
        }

        AcceptValidatedTool(tool);
        SetMessage(string.Empty, InstallerConfigStatusKind.Neutral);
    }

    public async Task<bool> ValidateToolPathAsync(CancellationToken cancellationToken = default)
    {
        return await AcceptToolPathAsync(CovenantSetupPath, cancellationToken);
    }

    public async Task<bool> AcceptToolPathAsync(
        string path,
        CancellationToken cancellationToken = default)
    {
        if (IsBusy)
        {
            return false;
        }

        if (string.IsNullOrWhiteSpace(path))
        {
            SyncToolPath(string.Empty);
            ClearValidatedTool(MissingToolStatus);
            SetMessage(string.Empty, InstallerConfigStatusKind.Neutral);
            return false;
        }

        IsCheckingTool = true;
        SetMessage("Checking covenant-setup...", InstallerConfigStatusKind.Neutral);
        try
        {
            SyncToolPath(path.Trim());
            var result = await _validateCovenantSetupTool(CovenantSetupPath, cancellationToken);
            if (result.IsValid && result.Tool is not null)
            {
                AcceptValidatedTool(result.Tool);
                SetMessage(string.Empty, InstallerConfigStatusKind.Neutral);
                return true;
            }

            ClearValidatedTool(result.Message);
            SetMessage(
                result.Message + " Pick a valid covenant-setup executable.",
                InstallerConfigStatusKind.Error);
            return false;
        }
        finally
        {
            IsCheckingTool = false;
        }
    }

    public void SetManifestValidationState(bool hasErrors)
    {
        HasManifestValidationErrors = hasErrors;
    }

    public async Task<bool> SaveConfigAsync(CancellationToken cancellationToken = default)
    {
        if (IsBuilding)
        {
            SetMessage("Installer configuration cannot be saved while a build is running.", InstallerConfigStatusKind.Error);
            return false;
        }

        if (HasManifestValidationErrors)
        {
            SetMessage("Resolve manifest validation errors before saving installer configuration.", InstallerConfigStatusKind.Error);
            return false;
        }

        if (!IsAcceptedToolPath(CovenantSetupPath))
        {
            var accepted = await ValidateToolPathAsync(cancellationToken);
            if (!accepted)
            {
                return false;
            }
        }

        if (string.IsNullOrWhiteSpace(OutputDirectory))
        {
            SetMessage("Choose an output directory before saving installer configuration.", InstallerConfigStatusKind.Error);
            return false;
        }

        SetMessage("Installer configuration saved.", InstallerConfigStatusKind.Success);
        return true;
    }

    public void ClearMessage()
    {
        SetMessage(string.Empty, InstallerConfigStatusKind.Neutral);
    }

    private void AcceptValidatedTool(CovenantSetupTool tool)
    {
        CovenantSetupTool = tool;
        SyncToolPath(tool.Path);
        StatusText = "Packaging enabled: " + tool.Path;
        StatusKind = InstallerConfigStatusKind.Success;
    }

    private void ClearValidatedTool(string statusText)
    {
        CovenantSetupTool = null;
        StatusText = statusText;
        StatusKind = InstallerConfigStatusKind.Error;
    }

    private void SyncToolPath(string path)
    {
        _syncingAcceptedToolPath = true;
        try
        {
            CovenantSetupPath = path;
        }
        finally
        {
            _syncingAcceptedToolPath = false;
        }
    }

    private bool IsAcceptedToolPath(string path) =>
        CovenantSetupTool is not null &&
        string.Equals(path?.Trim(), CovenantSetupTool.Path, StringComparison.OrdinalIgnoreCase);

    private void SetMessage(string message, InstallerConfigStatusKind kind)
    {
        MessageText = message;
        MessageKind = kind;
    }

    private void NotifyBusyStateChanged()
    {
        OnPropertyChanged(nameof(IsBusy));
        OnPropertyChanged(nameof(CanValidateToolPath));
        OnPropertyChanged(nameof(CanSaveConfig));
        RaiseCommandStatesChanged();
    }

    private bool SetProperty<T>(
        ref T field,
        T value,
        [CallerMemberName] string? propertyName = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }

    private void OnPropertyChanged([CallerMemberName] string? propertyName = null)
    {
        if (propertyName is not null)
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
        }
    }

    private void RaiseCommandStatesChanged()
    {
        ((RelayCommand)RefreshToolCheckCommand).RaiseCanExecuteChanged();
        ((AsyncRelayCommand)ValidateToolPathCommand).RaiseCanExecuteChanged();
        ((AsyncRelayCommand)SaveConfigCommand).RaiseCanExecuteChanged();
    }

    private sealed class RelayCommand : ICommand
    {
        private readonly Action _execute;
        private readonly Func<bool>? _canExecute;

        public RelayCommand(Action execute, Func<bool>? canExecute = null)
        {
            _execute = execute;
            _canExecute = canExecute;
        }

        public event EventHandler? CanExecuteChanged;

        public bool CanExecute(object? parameter) => _canExecute?.Invoke() ?? true;

        public void Execute(object? parameter) => _execute();

        public void RaiseCanExecuteChanged()
        {
            CanExecuteChanged?.Invoke(this, EventArgs.Empty);
        }
    }

    private sealed class AsyncRelayCommand : ICommand
    {
        private readonly Func<CancellationToken, Task> _execute;
        private readonly Func<bool>? _canExecute;
        private CancellationTokenSource? _currentExecution;

        public AsyncRelayCommand(
            Func<CancellationToken, Task> execute,
            Func<bool>? canExecute = null)
        {
            _execute = execute;
            _canExecute = canExecute;
        }

        public event EventHandler? CanExecuteChanged;

        public bool CanExecute(object? parameter) =>
            _currentExecution is null &&
            (_canExecute?.Invoke() ?? true);

        public async void Execute(object? parameter)
        {
            if (!CanExecute(parameter))
            {
                return;
            }

            _currentExecution = new CancellationTokenSource();
            RaiseCanExecuteChanged();
            try
            {
                await _execute(_currentExecution.Token);
            }
            finally
            {
                _currentExecution?.Dispose();
                _currentExecution = null;
                RaiseCanExecuteChanged();
            }
        }

        public void RaiseCanExecuteChanged()
        {
            CanExecuteChanged?.Invoke(this, EventArgs.Empty);
        }
    }
}
