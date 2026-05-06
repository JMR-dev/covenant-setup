using System.Collections.ObjectModel;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Data;
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
    private ToggleSwitch _themeToggle = null!;
    private string? _lastSavedManifestPath;
    private bool _isPackaging;
    private int _copyPreviewFeedbackVersion;
    private bool _syncingThemeToggle;
    private bool _usingSystemTheme;

    public MainWindow()
    {
        Title = "Covenant Setup Manifest Authoring";
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
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(480) });
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        var header = BuildHeader();
        Grid.SetRow(header, 0);
        Grid.SetColumnSpan(header, 2);
        root.Children.Add(header);

        var editor = BuildEditor();
        Grid.SetRow(editor, 1);
        Grid.SetColumn(editor, 0);
        root.Children.Add(editor);

        var preview = BuildPreview();
        Grid.SetRow(preview, 1);
        Grid.SetColumn(preview, 1);
        root.Children.Add(preview);

        _statusText = new TextBlock
        {
            TextWrapping = TextWrapping.Wrap,
            MaxLines = 3
        };
        _statusText.SetBinding(
            TextBlock.TextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.ValidationSummary)),
                Mode = BindingMode.OneWay
            });

        Grid.SetRow(_statusText, 2);
        Grid.SetColumnSpan(_statusText, 2);
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
        _themeToggle.Toggled += (_, _) => ToggleTheme();
        themeControl.Children.Add(_themeToggle);
        actions.Children.Add(themeControl);

        var validateButton = new Button { Content = "Validate", MinWidth = 92 };
        validateButton.Click += async (_, _) => await ShowValidationAsync();
        actions.Children.Add(validateButton);

        var installerConfigButton = new Button { Content = "Installer Config", MinWidth = 124 };
        installerConfigButton.Click += async (_, _) => await ShowInstallerConfigAsync();
        actions.Children.Add(installerConfigButton);

        _generateInstallerButton = new Button { Content = "Save and Build", MinWidth = 124 };
        _generateInstallerButton.Click += async (_, _) => await GenerateInstallerAsync();
        actions.Children.Add(_generateInstallerButton);

        var saveButton = new Button { Content = "Save TOML", MinWidth = 104 };
        saveButton.Click += async (_, _) => await SaveManifestAsync();
        actions.Children.Add(saveButton);

        Grid.SetColumn(actions, 1);
        header.Children.Add(actions);
        return header;
    }

    private ScrollViewer BuildEditor()
    {
        var stack = new StackPanel { Spacing = 12 };
        stack.Children.Add(BuildAppSection());
        stack.Children.Add(BuildDirectoriesSection());
        stack.Children.Add(BuildFilesSection());
        stack.Children.Add(BuildRegistrySection());
        stack.Children.Add(BuildShortcutsSection());
        stack.Children.Add(BuildScriptsSection());

        return new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Content = stack
        };
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
                Labeled("Install Root", rootCombo),
                Labeled("Application Target Installation Folder", folderBox)),
            Labeled("Primary Payload", payloadBox));
    }

    private FrameworkElement BuildDirectoriesSection()
    {
        var pathBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App" };
        var rows = RemovableRows(_viewModel.Directories);
        var addButton = new Button { Content = "Add Path", MinWidth = 88 };
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
        var destinationBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App\bin\app.exe" };
        var rows = RemovableRows(_viewModel.Files);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
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
        var nameBox = new TextBox { PlaceholderText = "InstallRoot" };
        var valueBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App" };
        var rows = RemovableRows(_viewModel.Registry);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
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
        var argsBox = new TextBox
        {
            AcceptsReturn = true,
            Height = 76,
            PlaceholderText = "-ExecutionPolicy"
        };
        var workingDirectoryBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App" };
        var rows = RemovableRows(_viewModel.Scripts);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
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

        _isPackaging = true;
        _installerConfigViewModel.IsBuilding = true;
        RefreshPackageControls();
        try
        {
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
    }

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
        ApplyThemeBrushes();
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
        ApplyThemeBrushes();
        return border;
    }

    private static IReadOnlyList<string> SplitLines(string value) =>
        value.Split(
            [Environment.NewLine, "\n"],
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
