using System.Collections.ObjectModel;
using System.Runtime.InteropServices;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Text;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.UI;
using Windows.UI.ViewManagement;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics;
using Windows.Storage;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace Covenant.Setup.Authoring;

internal sealed class MainWindow : Window
{
    private readonly MainViewModel _viewModel = new();
    private readonly InstallerConfigViewModel _installerConfigViewModel = new();
    private readonly List<Border> _sectionBorders = [];
    private readonly List<Border> _rowBorders = [];
    private readonly List<TextBlock> _secondaryTextBlocks = [];
    private TextBlock _statusText = null!;
    private TextBox _previewBox = null!;
    private Button _copyPreviewButton = null!;
    private Button _generateInstallerButton = null!;
    private Grid _rootGrid = null!;
    private ScrollView _editorScroll = null!;
    private ColumnSplitter _paneSplitter = null!;
    private ToggleSwitch _themeToggle = null!;

    // Keep the subclass delegate alive for the window's lifetime so the GC
    // cannot collect the thunk the OS window procedure chain is calling into.
    private SubclassProc? _wheelSubclassProc;
    private IntPtr _hwnd;
    private long _lastXamlWheelTick;

    private const double EditorMinWidth = 360;
    private const double PreviewMinWidth = 320;
    private string? _lastSavedManifestPath;
    private bool _isPackaging;
    private int _copyPreviewFeedbackVersion;
    private bool _syncingThemeToggle;
    private bool _usingSystemTheme;

    public MainWindow()
    {
        Title = "Covenant Setup Manifest Authoring";

        // Parse command line arguments for testing (to avoid interactive file pickers)
        var args = Environment.GetCommandLineArgs();
        for (int i = 1; i < args.Length - 1; i++)
        {
            if (args[i] == "--manifest-path")
            {
                _lastSavedManifestPath = args[i + 1];
            }
            else if (args[i] == "--tool-path")
            {
                _viewModel.SetCovenantSetupTool(new CovenantSetupTool(args[i + 1]));
            }
        }

        Content = BuildContent();
        ApplyInitialTheme();
        ConfigureWindow();
        _viewModel.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(MainViewModel.HasValidationErrors))
            {
                RefreshStatusBrush();
            }
            if (args.PropertyName is nameof(MainViewModel.HasCovenantSetupTool)
                or nameof(MainViewModel.CanPackage))
            {
                RefreshPackageControls();
            }
        };
        RefreshStatusBrush();
        RefreshPackageControls();
    }

    private Grid BuildContent()
    {
        var root = new Grid
        {
            Padding = new Thickness(8),
            RowSpacing = 8,
            ColumnSpacing = 8,
            DataContext = _viewModel
        };
        _rootGrid = root;
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var editorColumn = new ColumnDefinition { Width = new GridLength(480), MinWidth = EditorMinWidth };
        var previewColumn = new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star), MinWidth = PreviewMinWidth };
        root.ColumnDefinitions.Add(editorColumn);
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        root.ColumnDefinitions.Add(previewColumn);

        var header = BuildHeader();
        Grid.SetRow(header, 0);
        Grid.SetColumnSpan(header, 3);
        root.Children.Add(header);

        var editor = BuildEditor();
        Grid.SetRow(editor, 1);
        Grid.SetColumn(editor, 0);
        root.Children.Add(editor);

        _paneSplitter = new ColumnSplitter(editorColumn, previewColumn, EditorMinWidth, PreviewMinWidth);
        Grid.SetRow(_paneSplitter, 1);
        Grid.SetColumn(_paneSplitter, 1);
        root.Children.Add(_paneSplitter);

        var preview = BuildPreview();
        Grid.SetRow(preview, 1);
        Grid.SetColumn(preview, 2);
        root.Children.Add(preview);

        _statusText = new TextBlock
        {
            TextWrapping = TextWrapping.Wrap,
            MaxLines = 3
        };
        AutomationProperties.SetAutomationId(_statusText, "ValidationSummaryText");
        _statusText.SetBinding(
            TextBlock.TextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.ValidationSummary)),
                Mode = BindingMode.OneWay
            });

        Grid.SetRow(_statusText, 2);
        Grid.SetColumnSpan(_statusText, 3);
        root.Children.Add(_statusText);

        return root;
    }

    private Grid BuildHeader()
    {
        var header = new Grid { ColumnSpacing = 12 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var titleStack = new StackPanel { Spacing = 2 };
        titleStack.Children.Add(new TextBlock
        {
            Text = "Manifest Authoring",
            FontSize = 22,
            FontWeight = FontWeights.SemiBold
        });
        var subtitle = new TextBlock();
        _secondaryTextBlocks.Add(subtitle);
        subtitle.SetBinding(
            TextBlock.TextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.ManifestSubtitle)),
                Mode = BindingMode.OneWay
            });
        titleStack.Children.Add(subtitle);
        Grid.SetColumn(titleStack, 0);
        header.Children.Add(titleStack);

        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Spacing = 8
        };

        var themeControl = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            VerticalAlignment = VerticalAlignment.Center,
            Spacing = 6
        };
        themeControl.Children.Add(new TextBlock
        {
            Text = "Light",
            VerticalAlignment = VerticalAlignment.Center
        });

        _themeToggle = new ToggleSwitch
        {
            OnContent = "Dark",
            OffContent = string.Empty,
            MinWidth = 72
        };
        AutomationProperties.SetAutomationId(_themeToggle, "ThemeToggle");
        _themeToggle.Toggled += (_, _) => ToggleTheme();
        themeControl.Children.Add(_themeToggle);
        actions.Children.Add(themeControl);

        var validateButton = new Button { Content = "Validate", MinWidth = 92 };
        AutomationProperties.SetAutomationId(validateButton, "ValidateButton");
        validateButton.Click += async (_, _) => await ShowValidationAsync();
        actions.Children.Add(validateButton);

        var installerConfigButton = new Button { Content = "Installer Config", MinWidth = 124 };
        AutomationProperties.SetAutomationId(installerConfigButton, "InstallerConfigButton");
        installerConfigButton.Click += async (_, _) => await ShowInstallerConfigAsync();
        actions.Children.Add(installerConfigButton);

        _generateInstallerButton = new Button { Content = "Save and Build", MinWidth = 124 };
        AutomationProperties.SetAutomationId(_generateInstallerButton, "SaveAndBuildButton");
        _generateInstallerButton.Click += async (_, _) => await GenerateInstallerAsync();
        actions.Children.Add(_generateInstallerButton);

        var saveButton = new Button { Content = "Save TOML", MinWidth = 104 };
        AutomationProperties.SetAutomationId(saveButton, "SaveButton");
        saveButton.Click += async (_, _) => await SaveManifestAsync();
        actions.Children.Add(saveButton);

        Grid.SetColumn(actions, 1);
        header.Children.Add(actions);
        return header;
    }

    private ScrollView BuildEditor()
    {
        var stack = new StackPanel { Spacing = 12 };
        stack.Children.Add(BuildAppSection());
        stack.Children.Add(BuildDirectoriesSection());
        stack.Children.Add(BuildFilesSection());
        stack.Children.Add(BuildRegistrySection());
        stack.Children.Add(BuildShortcutsSection());
        stack.Children.Add(BuildScriptsSection());

        // ScrollView (not the legacy ScrollViewer) on purpose: the legacy control
        // scrolls through the OS DirectManipulation service, which consumes wheel
        // input below the XAML layer and then drops it for WinUI 3 windows
        // (microsoft-ui-xaml #8764 / #10091 / #10480 — wheel dead, scrollbar drag
        // fine). ScrollView drives InteractionTracker from XAML pointer events, so
        // DirectManipulation never claims the wheel over the editor.
        _editorScroll = new ScrollView
        {
            VerticalScrollBarVisibility = ScrollingScrollBarVisibility.Auto,
            Content = stack
        };
        return _editorScroll;
    }

    private Grid BuildPreview()
    {
        var preview = new Grid { RowSpacing = 8 };
        preview.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        preview.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });

        var header = new Grid { ColumnSpacing = 8 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        header.Children.Add(new TextBlock
        {
            Text = "TOML Preview",
            FontSize = 16,
            FontWeight = FontWeights.SemiBold,
            VerticalAlignment = VerticalAlignment.Center
        });

        _copyPreviewButton = new Button { Content = "Copy", MinWidth = 80 };
        AutomationProperties.SetAutomationId(_copyPreviewButton, "CopyPreviewButton");
        AutomationProperties.SetName(_copyPreviewButton, "Copy TOML preview");
        _copyPreviewButton.Click += async (_, _) => await CopyPreviewAsync();
        Grid.SetColumn(_copyPreviewButton, 1);
        header.Children.Add(_copyPreviewButton);

        Grid.SetRow(header, 0);
        preview.Children.Add(header);

        _previewBox = new TextBox
        {
            AcceptsReturn = true,
            FontFamily = new FontFamily("Consolas"),
            FontSize = 13,
            IsReadOnly = true,
            TextWrapping = TextWrapping.NoWrap
        };
        AutomationProperties.SetAutomationId(_previewBox, "PreviewBox");
        _previewBox.SetBinding(
            TextBox.TextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.TomlPreview)),
                Mode = BindingMode.OneWay
            });
        ScrollViewer.SetHorizontalScrollBarVisibility(_previewBox, ScrollBarVisibility.Auto);
        ScrollViewer.SetVerticalScrollBarVisibility(_previewBox, ScrollBarVisibility.Auto);

        Grid.SetRow(_previewBox, 1);
        preview.Children.Add(_previewBox);
        return preview;
    }

    private FrameworkElement BuildAppSection()
    {
        var appNameBox = BoundTextBox(nameof(MainViewModel.AppName), "App name");
        var folderBox = BoundTextBox(nameof(MainViewModel.ApplicationFolder), "Application target installation folder");
        var payloadBox = BoundTextBox(nameof(MainViewModel.PrimaryPayload), @"payload\app.exe");

        var rootCombo = new ComboBox
        {
            ItemsSource = ManifestTokens.KnownPathTokens,
            SelectedItem = _viewModel.InstallRootToken,
            MinWidth = 180
        };
        AutomationProperties.SetAutomationId(rootCombo, "InstallRootTokenCombo");
        rootCombo.SelectionChanged += (_, _) =>
        {
            if (rootCombo.SelectedItem is string token)
            {
                _viewModel.InstallRootToken = token;
            }
        };

        return Section(
            "App",
            Labeled("Name", appNameBox),
            TwoColumn(
                Labeled("Payload App Install Root", rootCombo),
                Labeled("Application Target Installation Folder", folderBox)),
            Labeled("Primary Payload", payloadBox));
    }

    private FrameworkElement BuildDirectoriesSection()
    {
        var pathBox = new TextBox();
        AutomationProperties.SetAutomationId(pathBox, "DirectoryPathBox");
        pathBox.SetBinding(
            TextBox.PlaceholderTextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.DirectoryPlaceholder)),
                Mode = BindingMode.OneWay
            });
        var rows = RemovableRows(_viewModel.Directories);
        var addButton = new Button { Content = "Add Path", MinWidth = 88 };
        AutomationProperties.SetAutomationId(addButton, "AddDirectoryButton");
        addButton.Click += (_, _) =>
        {
            _viewModel.AddDirectory(pathBox.Text);
            pathBox.Text = string.Empty;
        };

        return Section(
            "Directory Paths",
            InputRow(pathBox, addButton),
            rows);
    }

    private FrameworkElement BuildFilesSection()
    {
        var sourceBox = new TextBox { PlaceholderText = @"payload\app.exe" };
        AutomationProperties.SetAutomationId(sourceBox, "FileSourceBox");
        var destinationBox = new TextBox();
        AutomationProperties.SetAutomationId(destinationBox, "FileDestinationBox");
        destinationBox.SetBinding(
            TextBox.PlaceholderTextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.FileDestinationPlaceholder)),
                Mode = BindingMode.OneWay
            });
        var rows = RemovableRows(_viewModel.Files);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
        AutomationProperties.SetAutomationId(addButton, "AddFileButton");
        addButton.Click += async (_, _) =>
        {
            if (string.IsNullOrWhiteSpace(sourceBox.Text) || string.IsNullOrWhiteSpace(destinationBox.Text))
            {
                await ShowNoticeAsync("File Entry", "Source and destination are required.");
                return;
            }

            _viewModel.AddFile(sourceBox.Text, destinationBox.Text);
            sourceBox.Text = string.Empty;
            destinationBox.Text = string.Empty;
        };

        return Section(
            "Files",
            TwoColumn(Labeled("Source", sourceBox), Labeled("Destination", destinationBox)),
            AlignRight(addButton),
            rows);
    }

    private FrameworkElement BuildRegistrySection()
    {
        var keyBox = new TextBox { PlaceholderText = @"HKCU\Software\VendorApp" };
        AutomationProperties.SetAutomationId(keyBox, "RegistryKeyBox");
        var nameBox = new TextBox { PlaceholderText = "InstallRoot" };
        AutomationProperties.SetAutomationId(nameBox, "RegistryNameBox");
        var valueBox = new TextBox();
        AutomationProperties.SetAutomationId(valueBox, "RegistryValueBox");
        valueBox.SetBinding(
            TextBox.PlaceholderTextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.DirectoryPlaceholder)),
                Mode = BindingMode.OneWay
            });
        var rows = RemovableRows(_viewModel.Registry);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
        AutomationProperties.SetAutomationId(addButton, "AddRegistryButton");
        addButton.Click += async (_, _) =>
        {
            if (string.IsNullOrWhiteSpace(keyBox.Text) ||
                string.IsNullOrWhiteSpace(nameBox.Text) ||
                string.IsNullOrWhiteSpace(valueBox.Text))
            {
                await ShowNoticeAsync("Registry Entry", "Key, name, and value are required.");
                return;
            }

            _viewModel.AddRegistry(keyBox.Text, nameBox.Text, valueBox.Text);
            keyBox.Text = string.Empty;
            nameBox.Text = string.Empty;
            valueBox.Text = string.Empty;
        };

        return Section(
            "Registry",
            Labeled("Key", keyBox),
            TwoColumn(Labeled("Name", nameBox), Labeled("Value", valueBox)),
            AlignRight(addButton),
            rows);
    }

    private FrameworkElement BuildShortcutsSection()
    {
        var descriptionBox = BoundTextBox(nameof(MainViewModel.ShortcutDescription), "Launch application");

        return Section(
            "Shortcuts",
            Labeled("Description", descriptionBox));
    }

    private FrameworkElement BuildScriptsSection()
    {
        var commandBox = new TextBox { PlaceholderText = "powershell" };
        AutomationProperties.SetAutomationId(commandBox, "ScriptCommandBox");
        var argsBox = new TextBox
        {
            AcceptsReturn = true,
            Height = 76,
            PlaceholderText = "-ExecutionPolicy"
        };
        AutomationProperties.SetAutomationId(argsBox, "ScriptArgsBox");
        var workingDirectoryBox = new TextBox();
        AutomationProperties.SetAutomationId(workingDirectoryBox, "ScriptWorkingDirBox");
        workingDirectoryBox.SetBinding(
            TextBox.PlaceholderTextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.DirectoryPlaceholder)),
                Mode = BindingMode.OneWay
            });
        var rows = RemovableRows(_viewModel.Scripts);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
        AutomationProperties.SetAutomationId(addButton, "AddScriptButton");
        addButton.Click += async (_, _) =>
        {
            if (string.IsNullOrWhiteSpace(commandBox.Text))
            {
                await ShowNoticeAsync("Script Entry", "Command is required.");
                return;
            }

            _viewModel.AddScript(
                commandBox.Text,
                SplitLines(argsBox.Text),
                workingDirectoryBox.Text);
            commandBox.Text = string.Empty;
            argsBox.Text = string.Empty;
            workingDirectoryBox.Text = string.Empty;
        };

        return Section(
            "Scripts",
            Labeled("Command", commandBox),
            Labeled("Arguments", argsBox),
            Labeled("Working Directory", workingDirectoryBox),
            AlignRight(addButton),
            rows);
    }

    private async Task ShowInstallerConfigAsync()
    {
        _installerConfigViewModel.SetManifestValidationState(_viewModel.HasValidationErrors);
        _installerConfigViewModel.IsBuilding = _isPackaging;
        // The main view model owns the committed tool/output state; the dialog
        // is a transient editor seeded on open so Close discards edits.
        _installerConfigViewModel.SyncFrom(_viewModel.CovenantSetupTool, _viewModel.OutputDirectory);

        var dialog = new InstallerConfigDialog(
            _installerConfigViewModel,
            WindowNative.GetWindowHandle(this));
        if (Content is FrameworkElement root)
        {
            dialog.XamlRoot = root.XamlRoot;
        }

        var result = await dialog.ShowAsync();
        if (result == ContentDialogResult.Primary)
        {
            _viewModel.SetCovenantSetupTool(_installerConfigViewModel.CovenantSetupTool);
            _viewModel.OutputDirectory = _installerConfigViewModel.OutputDirectory;
            RefreshPackageControls();
        }
    }

    private async Task SaveManifestAsync()
    {
        var manifestPath = await WriteManifestWithPickerAsync(showNotice: true);
        if (manifestPath is not null)
        {
            _lastSavedManifestPath = manifestPath;
        }
    }

    private async Task<string?> WriteManifestWithPickerAsync(bool showNotice)
    {
        return await WriteManifestWithPickerAsync(showNotice, ShowNoticeAsync);
    }

    private async Task<string?> WriteManifestWithPickerAsync(
        bool showNotice,
        Func<string, string, Task> showMessageAsync)
    {
        var validation = _viewModel.Validate();
        if (!validation.IsValid)
        {
            await showMessageAsync("Validation", string.Join(Environment.NewLine, validation.Errors));
            return null;
        }

        if (!string.IsNullOrWhiteSpace(_lastSavedManifestPath) &&
            !_viewModel.IsExpectedManifestPath(_lastSavedManifestPath))
        {
            _lastSavedManifestPath = null;
        }

        if (!string.IsNullOrWhiteSpace(_lastSavedManifestPath))
        {
            await File.WriteAllTextAsync(_lastSavedManifestPath, _viewModel.TomlPreview);
            if (showNotice)
            {
                await showMessageAsync("Manifest Saved", _lastSavedManifestPath);
            }
            return _lastSavedManifestPath;
        }

        var picker = new FileSavePicker();
        InitializeWithWindow.Initialize(picker, WindowNative.GetWindowHandle(this));
        picker.SuggestedStartLocation = PickerLocationId.DocumentsLibrary;
        picker.FileTypeChoices.Add("TOML manifest", [".toml"]);
        picker.SuggestedFileName = _viewModel.ExpectedManifestFileName;

        var file = await picker.PickSaveFileAsync();
        if (file is null)
        {
            return null;
        }

        if (!_viewModel.IsExpectedManifestPath(file.Path))
        {
            await showMessageAsync(
                "Manifest File Name",
                MainViewModel.ContainsWhitespace(file.Path)
                    ? "The manifest path cannot contain spaces. Save the manifest as " + _viewModel.ExpectedManifestFileName + " in a folder path without spaces."
                    : "Save the manifest as " + _viewModel.ExpectedManifestFileName + ".");
            return null;
        }

        await FileIO.WriteTextAsync(file, _viewModel.TomlPreview);
        if (showNotice)
        {
            await showMessageAsync("Manifest Saved", file.Path);
        }
        return file.Path;
    }

    private async Task GenerateInstallerAsync(Func<string, string, Task>? showMessageAsync = null)
    {
        showMessageAsync ??= ShowNoticeAsync;
        // The packaging flag must be set before the first await so a second
        // click cannot start a concurrent run against the same output EXE.
        if (_isPackaging)
        {
            return;
        }
        _isPackaging = true;
        _installerConfigViewModel.IsBuilding = true;
        RefreshPackageControls();
        try
        {
            var tool = _viewModel.CovenantSetupTool;
            if (tool is null)
            {
                await showMessageAsync("Installer EXE", "covenant-setup.exe was not found. Packaging is disabled.");
                return;
            }

            var validation = _viewModel.Validate();
            if (!validation.IsValid)
            {
                await showMessageAsync("Validation", string.Join(Environment.NewLine, validation.Errors));
                return;
            }

            if (string.IsNullOrWhiteSpace(_viewModel.OutputDirectory))
            {
                await showMessageAsync("Installer EXE", "Choose an output directory before packaging.");
                return;
            }

            var manifestPath = await WriteManifestWithPickerAsync(showNotice: false, showMessageAsync);
            if (manifestPath is null)
            {
                return;
            }
            _lastSavedManifestPath = manifestPath;

            var result = await CovenantSetupPackager.PackageAsync(
                tool,
                manifestPath,
                _viewModel.OutputDirectory,
                CancellationToken.None);

            var message = result.Succeeded
                ? "Generated installer in " + _viewModel.OutputDirectory
                : $"Packaging failed with exit code {result.ExitCode}.";
            var detail = string.Join(
                Environment.NewLine + Environment.NewLine,
                new[] { message, result.Output, result.Error }.Where(part => !string.IsNullOrWhiteSpace(part)));
            await showMessageAsync("Installer EXE", detail);
        }
        catch (Exception ex)
        {
            await showMessageAsync("Installer EXE", "Packaging failed: " + ex.Message);
        }
        finally
        {
            _isPackaging = false;
            _installerConfigViewModel.IsBuilding = false;
            RefreshPackageControls();
        }
    }

    private async Task ShowValidationAsync()
    {
        var validation = _viewModel.Validate();
        var message = validation.Errors.Count > 0
            ? string.Join(Environment.NewLine, validation.Errors)
            : validation.Warnings.Count > 0
                ? string.Join(Environment.NewLine, validation.Warnings)
                : "Manifest is ready to save.";
        await ShowNoticeAsync("Validation", message);
    }

    private async Task CopyPreviewAsync()
    {
        var data = new DataPackage();
        data.SetText(_viewModel.TomlPreview);
        Clipboard.SetContent(data);
        await ShowCopyPreviewCopiedAsync();
    }

    private async Task ShowCopyPreviewCopiedAsync()
    {
        var version = ++_copyPreviewFeedbackVersion;
        var brushes = CreateThemeBrushes();

        _copyPreviewButton.Content = new FontIcon
        {
            FontFamily = new FontFamily("Segoe MDL2 Assets"),
            FontSize = 16,
            Glyph = "\uE8FB",
            Foreground = brushes.SuccessText
        };
        AutomationProperties.SetName(_copyPreviewButton, "Copied");

        await Task.Delay(TimeSpan.FromSeconds(1));

        if (version != _copyPreviewFeedbackVersion)
        {
            return;
        }

        _copyPreviewButton.Content = "Copy";
        AutomationProperties.SetName(_copyPreviewButton, "Copy TOML preview");
    }

    private async Task ShowNoticeAsync(string title, string message)
    {
        var dialog = new ContentDialog
        {
            Title = title,
            CloseButtonText = "OK",
            Content = new TextBlock
            {
                Text = message,
                TextWrapping = TextWrapping.Wrap,
                MaxWidth = 560
            }
        };

        if (Content is FrameworkElement root)
        {
            dialog.XamlRoot = root.XamlRoot;
        }

        _ = await dialog.ShowAsync();
    }

    private void ConfigureWindow()
    {
        var hwnd = WindowNative.GetWindowHandle(this);
        var windowId = Win32Interop.GetWindowIdFromWindow(hwnd);
        var appWindow = AppWindow.GetFromWindowId(windowId);
        appWindow.Title = Title;

        const int width = 1240;
        const int height = 820;
        var displayArea = DisplayArea.GetFromWindowId(windowId, DisplayAreaFallback.Primary);
        var workArea = displayArea.WorkArea;
        var x = workArea.X + Math.Max(0, (workArea.Width - width) / 2);
        var y = workArea.Y + Math.Max(0, (workArea.Height - height) / 2);
        appWindow.MoveAndResize(new RectInt32(x, y, width, height));

        _hwnd = hwnd;
        InstallRawWheelFallback();
        Closed += (_, _) => RemoveRawWheelFallback();
    }

    // On some machines WinUI 3 never delivers bare mouse-wheel input to the app:
    // the OS DirectManipulation service (kept active on the window's input HWND
    // by the legacy ScrollViewers inside every TextBox) claims the wheel below
    // the message layer and then drops it because it mis-judges the window's
    // activation state (microsoft-ui-xaml #8764 / #10091 / #10480). No window
    // message, XAML pointer event, or InteractionTracker input ever fires.
    // Raw Input is a parallel delivery channel DirectManipulation cannot
    // intercept, so we register for mouse raw input and scroll the pane under
    // the cursor ourselves. When the normal pipeline IS alive (healthy
    // machines), the XAML wheel event recorded below makes the fallback bow out
    // so nothing scrolls twice.
    private void InstallRawWheelFallback()
    {
        if (_wheelSubclassProc is not null)
        {
            return;
        }

        _wheelSubclassProc = WheelInputSubclassProc;
        if (!SetWindowSubclass(_hwnd, _wheelSubclassProc, 1, IntPtr.Zero))
        {
            _wheelSubclassProc = null;
            return;
        }

        var device = new RAWINPUTDEVICE
        {
            UsagePage = 0x01, // HID_USAGE_PAGE_GENERIC
            Usage = 0x02,     // HID_USAGE_GENERIC_MOUSE
            Flags = 0,        // deliver while this window's thread is foreground
            Target = _hwnd
        };
        RegisterRawInputDevices([device], 1, (uint)Marshal.SizeOf<RAWINPUTDEVICE>());

        _rootGrid.AddHandler(
            UIElement.PointerWheelChangedEvent,
            new PointerEventHandler((_, _) => _lastXamlWheelTick = Environment.TickCount64),
            handledEventsToo: true);
    }

    private void RemoveRawWheelFallback()
    {
        if (_wheelSubclassProc is not null)
        {
            RemoveWindowSubclass(_hwnd, _wheelSubclassProc, 1);
            _wheelSubclassProc = null;
        }
    }

    private IntPtr WheelInputSubclassProc(
        IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr idSubclass, IntPtr refData)
    {
        if (msg == WM_INPUT)
        {
            try
            {
                HandleRawWheelInput(lParam);
            }
            catch
            {
                // Never let an input-path failure take down the window procedure.
            }
        }

        return DefSubclassProc(hWnd, msg, wParam, lParam);
    }

    private void HandleRawWheelInput(IntPtr rawInputHandle)
    {
        var size = (uint)Marshal.SizeOf<RAWINPUT>();
        if (GetRawInputData(rawInputHandle, RID_INPUT, out var raw, ref size, (uint)Marshal.SizeOf<RAWINPUTHEADER>())
            == unchecked((uint)-1))
        {
            return;
        }

        if (raw.Header.Type != RIM_TYPEMOUSE || (raw.Mouse.ButtonFlags & RI_MOUSE_WHEEL) == 0)
        {
            return;
        }

        var delta = (short)raw.Mouse.ButtonData;
        if (delta == 0 || (GetKeyState(VK_CONTROL) & 0x8000) != 0)
        {
            return; // Ctrl+wheel is zoom and does reach the XAML pipeline.
        }

        // Give a healthy XAML wheel event (already ahead of us in the queue) a
        // chance to run first; if it does, the normal pipeline owns scrolling.
        var stamp = Environment.TickCount64;
        DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            var xamlSawThisNotch = _lastXamlWheelTick >= stamp - 32;
            var xamlRecentlyAlive = Environment.TickCount64 - _lastXamlWheelTick <= 250;
            if (!xamlSawThisNotch && !xamlRecentlyAlive)
            {
                ScrollPaneUnderCursor(delta);
            }
        });
    }

    private void ScrollPaneUnderCursor(short delta)
    {
        if (_rootGrid?.XamlRoot is null || !GetCursorPos(out var screen))
        {
            return;
        }

        var hit = WindowFromPoint(screen);
        if (hit != _hwnd && GetAncestor(hit, GA_ROOT) != _hwnd)
        {
            return;
        }

        // A ContentDialog overlays the panes inside the same HWND; don't scroll
        // what's underneath it.
        if (VisualTreeHelper.GetOpenPopupsForXamlRoot(_rootGrid.XamlRoot).Count > 0)
        {
            return;
        }

        var client = screen;
        if (!ScreenToClient(_hwnd, ref client))
        {
            return;
        }

        var scale = _rootGrid.XamlRoot.RasterizationScale;
        if (scale <= 0)
        {
            scale = 1;
        }

        var cursor = new Windows.Foundation.Point(client.X / scale, client.Y / scale);
        if (IsOver(_editorScroll, cursor))
        {
            _editorScroll.ScrollBy(0, -delta, new ScrollingScrollOptions(ScrollingAnimationMode.Disabled));
        }
        else if (IsOver(_previewBox, cursor))
        {
            var inner = FindDescendant<ScrollViewer>(_previewBox);
            inner?.ChangeView(null, inner.VerticalOffset - delta, null, disableAnimation: true);
        }
    }

    private static bool IsOver(FrameworkElement? element, Windows.Foundation.Point cursor)
    {
        if (element?.XamlRoot is null)
        {
            return false;
        }

        var origin = element.TransformToVisual(null).TransformPoint(new Windows.Foundation.Point(0, 0));
        return cursor.X >= origin.X && cursor.X <= origin.X + element.ActualWidth
            && cursor.Y >= origin.Y && cursor.Y <= origin.Y + element.ActualHeight;
    }

    private static T? FindDescendant<T>(DependencyObject parent) where T : class
    {
        var count = VisualTreeHelper.GetChildrenCount(parent);
        for (var i = 0; i < count; i++)
        {
            var child = VisualTreeHelper.GetChild(parent, i);
            if (child is T match)
            {
                return match;
            }

            if (FindDescendant<T>(child) is { } nested)
            {
                return nested;
            }
        }

        return null;
    }

    private const uint WM_INPUT = 0x00FF;
    private const uint RID_INPUT = 0x10000003;
    private const uint RIM_TYPEMOUSE = 0;
    private const ushort RI_MOUSE_WHEEL = 0x0400;
    private const int VK_CONTROL = 0x11;
    private const uint GA_ROOT = 2;

    private delegate IntPtr SubclassProc(
        IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr idSubclass, IntPtr refData);

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT
    {
        public int X;
        public int Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct RAWINPUTDEVICE
    {
        public ushort UsagePage;
        public ushort Usage;
        public uint Flags;
        public IntPtr Target;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct RAWINPUTHEADER
    {
        public uint Type;
        public uint Size;
        public IntPtr Device;
        public IntPtr WParam;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct RAWMOUSE
    {
        [FieldOffset(0)] public ushort Flags;
        [FieldOffset(4)] public ushort ButtonFlags;
        [FieldOffset(6)] public ushort ButtonData;
        [FieldOffset(8)] public uint RawButtons;
        [FieldOffset(12)] public int LastX;
        [FieldOffset(16)] public int LastY;
        [FieldOffset(20)] public uint ExtraInformation;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct RAWINPUT
    {
        public RAWINPUTHEADER Header;
        public RAWMOUSE Mouse;
    }

    [DllImport("comctl32.dll")]
    private static extern bool SetWindowSubclass(IntPtr hWnd, SubclassProc pfnSubclass, UIntPtr uIdSubclass, IntPtr dwRefData);

    [DllImport("comctl32.dll")]
    private static extern bool RemoveWindowSubclass(IntPtr hWnd, SubclassProc pfnSubclass, UIntPtr uIdSubclass);

    [DllImport("comctl32.dll")]
    private static extern IntPtr DefSubclassProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool RegisterRawInputDevices(RAWINPUTDEVICE[] devices, uint count, uint size);

    [DllImport("user32.dll")]
    private static extern uint GetRawInputData(IntPtr hRawInput, uint uiCommand, out RAWINPUT pData, ref uint pcbSize, uint cbSizeHeader);

    [DllImport("user32.dll")]
    private static extern bool GetCursorPos(out POINT point);

    [DllImport("user32.dll")]
    private static extern IntPtr WindowFromPoint(POINT point);

    [DllImport("user32.dll")]
    private static extern IntPtr GetAncestor(IntPtr hwnd, uint gaFlags);

    [DllImport("user32.dll")]
    private static extern bool ScreenToClient(IntPtr hWnd, ref POINT point);

    [DllImport("user32.dll")]
    private static extern short GetKeyState(int virtualKey);

    private void ApplyInitialTheme()
    {
        var savedTheme = AuthoringPreferences.LoadTheme();
        var osTheme = savedTheme is null ? DetectOsTheme() : null;
        var initialTheme = savedTheme ?? osTheme ?? ElementTheme.Dark;
        _usingSystemTheme = savedTheme is null && osTheme is not null;
        _rootGrid.RequestedTheme = _usingSystemTheme ? ElementTheme.Default : initialTheme;
        SetThemeToggle(initialTheme == ElementTheme.Dark);

        _rootGrid.Loaded += (_, _) =>
        {
            if (_usingSystemTheme)
            {
                SetThemeToggle(EffectiveTheme() == ElementTheme.Dark);
            }

            ApplyThemeBrushes();
        };
        _rootGrid.ActualThemeChanged += (_, _) =>
        {
            if (_usingSystemTheme)
            {
                SetThemeToggle(EffectiveTheme() == ElementTheme.Dark);
            }

            ApplyThemeBrushes();
        };
        ApplyThemeBrushes();
    }

    private void ToggleTheme()
    {
        if (_syncingThemeToggle)
        {
            return;
        }

        var theme = _themeToggle.IsOn ? ElementTheme.Dark : ElementTheme.Light;
        _usingSystemTheme = false;
        _rootGrid.RequestedTheme = theme;
        AuthoringPreferences.SaveTheme(theme);
        ApplyThemeBrushes();
    }

    private void SetThemeToggle(bool isDark)
    {
        _syncingThemeToggle = true;
        try
        {
            _themeToggle.IsOn = isDark;
        }
        finally
        {
            _syncingThemeToggle = false;
        }
    }

    private void RefreshStatusBrush()
    {
        if (_statusText is null)
        {
            return;
        }

        var brushes = CreateThemeBrushes();
        _statusText.Foreground = _viewModel.HasValidationErrors
            ? brushes.ErrorText
            : brushes.SuccessText;
    }

    private void RefreshPackageControls()
    {
        if (_generateInstallerButton is not null)
        {
            _generateInstallerButton.IsEnabled = _viewModel.HasCovenantSetupTool && !_isPackaging;
            _generateInstallerButton.Content = _isPackaging ? "Building..." : "Save and Build";
        }
    }

    private void ApplyThemeBrushes()
    {
        if (_rootGrid is null)
        {
            return;
        }

        var brushes = CreateThemeBrushes();
        _rootGrid.Background = brushes.PageBackground;

        foreach (var border in _sectionBorders)
        {
            border.Background = brushes.SurfaceBackground;
            border.BorderBrush = brushes.Border;
        }

        foreach (var border in _rowBorders)
        {
            border.Background = brushes.RowBackground;
            border.BorderBrush = brushes.Border;
        }

        foreach (var textBlock in _secondaryTextBlocks)
        {
            textBlock.Foreground = brushes.SecondaryText;
        }

        if (_previewBox is not null)
        {
            _previewBox.Background = brushes.PreviewBackground;
            _previewBox.Foreground = brushes.PreviewText;
            _previewBox.BorderBrush = brushes.Border;
        }

        if (_paneSplitter is not null)
        {
            _paneSplitter.Background = brushes.Border;
        }

        RefreshStatusBrush();
    }

    private ElementTheme EffectiveTheme()
    {
        if (_rootGrid.RequestedTheme is ElementTheme.Dark or ElementTheme.Light)
        {
            return _rootGrid.RequestedTheme;
        }

        if (_rootGrid.ActualTheme is ElementTheme.Dark or ElementTheme.Light)
        {
            return _rootGrid.ActualTheme;
        }

        return DetectOsTheme() ?? ElementTheme.Dark;
    }

    private static ElementTheme? DetectOsTheme()
    {
        try
        {
            var background = new UISettings().GetColorValue(UIColorType.Background);
            var luminance = (0.2126 * background.R) + (0.7152 * background.G) + (0.0722 * background.B);
            return luminance < 128 ? ElementTheme.Dark : ElementTheme.Light;
        }
        catch
        {
            return null;
        }
    }

    private ThemeBrushes CreateThemeBrushes() =>
        EffectiveTheme() == ElementTheme.Dark
            ? new ThemeBrushes(
                Brush(27, 26, 25),
                Brush(37, 36, 35),
                Brush(46, 45, 43),
                Brush(90, 88, 86),
                Brush(200, 198, 196),
                Brush(17, 17, 17),
                Brush(243, 242, 241),
                Brush(108, 203, 127),
                Brush(255, 138, 138))
            : new ThemeBrushes(
                Brush(246, 246, 244),
                Brush(255, 255, 255),
                Brush(250, 250, 248),
                Brush(208, 208, 208),
                Brush(98, 98, 98),
                Brush(255, 255, 255),
                Brush(26, 26, 26),
                Brush(16, 124, 16),
                Brush(196, 43, 28));

    private static SolidColorBrush Brush(byte red, byte green, byte blue) =>
        new(Color.FromArgb(255, red, green, blue));

    private static TextBox BoundTextBox(string propertyName, string placeholder)
    {
        var box = new TextBox { PlaceholderText = placeholder };
        AutomationProperties.SetAutomationId(box, propertyName);
        box.SetBinding(
            TextBox.TextProperty,
            new Binding
            {
                Path = new PropertyPath(propertyName),
                Mode = BindingMode.TwoWay,
                UpdateSourceTrigger = UpdateSourceTrigger.PropertyChanged
            });
        return box;
    }

    private FrameworkElement Section(string title, params UIElement[] children)
    {
        var stack = new StackPanel { Spacing = 6 };
        stack.Children.Add(new TextBlock
        {
            Text = title,
            FontSize = 16,
            FontWeight = FontWeights.SemiBold
        });

        foreach (var child in children)
        {
            stack.Children.Add(child);
        }

        var border = new Border
        {
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(6),
            Padding = new Thickness(8),
            Child = stack
        };
        _sectionBorders.Add(border);
        var brushes = CreateThemeBrushes();
        border.Background = brushes.SurfaceBackground;
        border.BorderBrush = brushes.Border;
        return border;
    }

    private static FrameworkElement Labeled(string label, FrameworkElement control)
    {
        var stack = new StackPanel
        {
            Spacing = 2,
            HorizontalAlignment = HorizontalAlignment.Stretch
        };
        stack.Children.Add(new TextBlock
        {
            Text = label,
            FontWeight = FontWeights.SemiBold,
            FontSize = 12
        });
        stack.Children.Add(control);
        return stack;
    }

    private static Grid TwoColumn(FrameworkElement left, FrameworkElement right)
    {
        var grid = new Grid { ColumnSpacing = 6 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        Grid.SetColumn(left, 0);
        Grid.SetColumn(right, 1);
        grid.Children.Add(left);
        grid.Children.Add(right);
        return grid;
    }

    private static Grid InputRow(TextBox textBox, Button button)
    {
        var grid = new Grid { ColumnSpacing = 6 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(9, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(7, GridUnitType.Star) });
        grid.HorizontalAlignment = HorizontalAlignment.Stretch;
        textBox.HorizontalAlignment = HorizontalAlignment.Stretch;
        button.HorizontalAlignment = HorizontalAlignment.Right;
        Grid.SetColumn(textBox, 0);
        Grid.SetColumn(button, 1);
        grid.Children.Add(textBox);
        grid.Children.Add(button);
        return grid;
    }

    private static FrameworkElement AlignRight(Button button)
    {
        button.HorizontalAlignment = HorizontalAlignment.Right;
        return button;
    }

    private FrameworkElement RemovableRows<T>(ObservableCollection<T> collection)
    {
        var rows = new StackPanel { Spacing = 4 };

        void RenderRows()
        {
            foreach (var border in rows.Children.OfType<Border>())
            {
                _rowBorders.Remove(border);
            }

            rows.Children.Clear();
            foreach (var item in collection.ToArray())
            {
                var value = item;
                rows.Children.Add(RemovableRow(Convert.ToString(value) ?? string.Empty, () => collection.Remove(value)));
            }

            rows.Visibility = collection.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
        }

        collection.CollectionChanged += (_, _) => RenderRows();
        RenderRows();
        return rows;
    }

    private FrameworkElement RemovableRow(string text, Action remove)
    {
        var grid = new Grid { ColumnSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var label = new TextBlock
        {
            Text = text,
            TextWrapping = TextWrapping.Wrap,
            VerticalAlignment = VerticalAlignment.Center
        };
        Grid.SetColumn(label, 0);
        grid.Children.Add(label);

        var removeButton = new Button
        {
            Content = "x",
            MinWidth = 28,
            Width = 28,
            Height = 28,
            Padding = new Thickness(0),
            VerticalAlignment = VerticalAlignment.Center
        };
        removeButton.Click += (_, _) => remove();
        Grid.SetColumn(removeButton, 1);
        grid.Children.Add(removeButton);

        var border = new Border
        {
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(6),
            Child = grid
        };
        _rowBorders.Add(border);
        var brushes = CreateThemeBrushes();
        border.Background = brushes.RowBackground;
        border.BorderBrush = brushes.Border;
        return border;
    }

    // Thin draggable divider between the editor and preview panes. Dragging
    // adjusts the editor column's pixel width; the preview column is star-sized
    // and absorbs the remainder. Both panes are clamped to sensible minimums.
    private sealed class ColumnSplitter : Grid
    {
        private readonly ColumnDefinition _primary;
        private readonly ColumnDefinition _secondary;
        private readonly double _minPrimary;
        private readonly double _minSecondary;
        private bool _dragging;
        private double _startX;
        private double _startWidth;
        private double _available;

        public ColumnSplitter(
            ColumnDefinition primary,
            ColumnDefinition secondary,
            double minPrimary,
            double minSecondary)
        {
            _primary = primary;
            _secondary = secondary;
            _minPrimary = minPrimary;
            _minSecondary = minSecondary;

            Width = 6;
            HorizontalAlignment = HorizontalAlignment.Stretch;
            VerticalAlignment = VerticalAlignment.Stretch;
            ProtectedCursor = InputSystemCursor.Create(InputSystemCursorShape.SizeWestEast);

            // Pointer capture (rather than ManipulationMode) is used deliberately:
            // enabling the manipulation pipeline on this element intercepts the
            // mouse-wheel routing the adjacent editor pane depends on.
            PointerPressed += OnPointerPressed;
            PointerMoved += OnPointerMoved;
            PointerReleased += OnPointerReleased;
            PointerCaptureLost += OnPointerCaptureLost;
        }

        private void OnPointerPressed(object sender, PointerRoutedEventArgs e)
        {
            _dragging = CapturePointer(e.Pointer);
            _startX = e.GetCurrentPoint(null).Position.X;
            _startWidth = _primary.ActualWidth;
            _available = _primary.ActualWidth + _secondary.ActualWidth;
            e.Handled = true;
        }

        private void OnPointerMoved(object sender, PointerRoutedEventArgs e)
        {
            if (!_dragging)
            {
                return;
            }

            var delta = e.GetCurrentPoint(null).Position.X - _startX;
            var max = Math.Max(_minPrimary, _available - _minSecondary);
            var desired = Math.Clamp(_startWidth + delta, _minPrimary, max);
            _primary.Width = new GridLength(desired);
            e.Handled = true;
        }

        private void OnPointerReleased(object sender, PointerRoutedEventArgs e)
        {
            if (_dragging)
            {
                ReleasePointerCapture(e.Pointer);
                _dragging = false;
                e.Handled = true;
            }
        }

        private void OnPointerCaptureLost(object sender, PointerRoutedEventArgs e) => _dragging = false;
    }

    private static IReadOnlyList<string> SplitLines(string value) =>
        value.Split(
            [Environment.NewLine, "\n", "\r"],
            StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);

    private sealed record ThemeBrushes(
        SolidColorBrush PageBackground,
        SolidColorBrush SurfaceBackground,
        SolidColorBrush RowBackground,
        SolidColorBrush Border,
        SolidColorBrush SecondaryText,
        SolidColorBrush PreviewBackground,
        SolidColorBrush PreviewText,
        SolidColorBrush SuccessText,
        SolidColorBrush ErrorText);
}
