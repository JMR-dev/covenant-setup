using System.Collections;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics;
using Windows.Storage;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace Covenant.Setup.Authoring;

internal sealed class MainWindow : Window
{
    private readonly MainViewModel _viewModel = new();
    private TextBlock _statusText = null!;
    private TextBlock _toolStatusText = null!;
    private TextBox _previewBox = null!;
    private TextBox _outputDirectoryBox = null!;
    private Button _chooseOutputButton = null!;
    private Button _generateInstallerButton = null!;
    private string? _lastSavedManifestPath;
    private bool _isPackaging;

    public MainWindow()
    {
        Title = "Covenant Setup Manifest Authoring";
        Content = BuildContent();
        ConfigureWindow();
        _viewModel.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(MainViewModel.HasValidationErrors))
            {
                RefreshStatusBrush();
            }
            if (args.PropertyName is nameof(MainViewModel.HasCovenantSetupTool)
                or nameof(MainViewModel.CanPackage)
                or nameof(MainViewModel.CovenantSetupStatus)
                or nameof(MainViewModel.OutputDirectory))
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
            Padding = new Thickness(16),
            RowSpacing = 12,
            ColumnSpacing = 16,
            DataContext = _viewModel
        };
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(520) });
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
        titleStack.Children.Add(new TextBlock
        {
            Text = "Create install.toml for covenant-setup",
            Foreground = new SolidColorBrush(Colors.DimGray)
        });
        Grid.SetColumn(titleStack, 0);
        header.Children.Add(titleStack);

        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Spacing = 8
        };

        var validateButton = new Button { Content = "Validate", MinWidth = 92 };
        validateButton.Click += async (_, _) => await ShowValidationAsync();
        actions.Children.Add(validateButton);

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
        stack.Children.Add(BuildPackageSection());
        stack.Children.Add(BuildDirectoriesSection());
        stack.Children.Add(BuildFilesSection());
        stack.Children.Add(BuildRegistrySection());
        stack.Children.Add(BuildShortcutsSection());
        stack.Children.Add(BuildScriptsSection());
        stack.Children.Add(BuildPurgeSection());

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

        var copyButton = new Button { Content = "Copy", MinWidth = 80 };
        copyButton.Click += async (_, _) => await CopyPreviewAsync();
        Grid.SetColumn(copyButton, 1);
        header.Children.Add(copyButton);

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
        var folderBox = BoundTextBox(nameof(MainViewModel.ApplicationFolder), "Application folder");
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

        var defaultsButton = new Button
        {
            Content = "Apply Suggested Entries",
            HorizontalAlignment = HorizontalAlignment.Left
        };
        defaultsButton.Click += (_, _) => _viewModel.ApplyDefaults(resetCollections: false);

        return Section(
            "App",
            Labeled("Name", appNameBox),
            TwoColumn(
                Labeled("Install Root", rootCombo),
                Labeled("Folder", folderBox)),
            Labeled("Primary Payload", payloadBox),
            defaultsButton);
    }

    private FrameworkElement BuildPackageSection()
    {
        _toolStatusText = new TextBlock
        {
            TextWrapping = TextWrapping.Wrap
        };
        _toolStatusText.SetBinding(
            TextBlock.TextProperty,
            new Binding
            {
                Path = new PropertyPath(nameof(MainViewModel.CovenantSetupStatus)),
                Mode = BindingMode.OneWay
            });

        _outputDirectoryBox = BoundTextBox(nameof(MainViewModel.OutputDirectory), @"C:\path\to\dist");

        _chooseOutputButton = new Button
        {
            Content = "Choose",
            MinWidth = 84
        };
        _chooseOutputButton.Click += async (_, _) => await ChooseOutputDirectoryAsync();

        var refreshButton = new Button
        {
            Content = "Refresh Tool Check",
            MinWidth = 140
        };
        refreshButton.Click += (_, _) => _viewModel.RefreshCovenantSetupTool();

        _generateInstallerButton = new Button
        {
            Content = "Generate Installer EXE",
            MinWidth = 168
        };
        _generateInstallerButton.Click += async (_, _) => await GenerateInstallerAsync();

        var actionRow = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Spacing = 8
        };
        actionRow.Children.Add(refreshButton);
        actionRow.Children.Add(_generateInstallerButton);

        return Section(
            "Installer EXE",
            _toolStatusText,
            Labeled("Output Directory", InputRow(_outputDirectoryBox, _chooseOutputButton)),
            actionRow);
    }

    private FrameworkElement BuildDirectoriesSection()
    {
        var pathBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App" };
        var list = List(_viewModel.Directories, 104);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
        addButton.Click += (_, _) =>
        {
            _viewModel.AddDirectory(pathBox.Text);
            pathBox.Text = string.Empty;
        };

        return Section(
            "Directories",
            InputRow(pathBox, addButton),
            list,
            RemoveButton(list, _viewModel.Directories));
    }

    private FrameworkElement BuildFilesSection()
    {
        var sourceBox = new TextBox { PlaceholderText = @"payload\app.exe" };
        var destinationBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App\bin\app.exe" };
        var list = List(_viewModel.Files, 120);
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
            list,
            RemoveButton(list, _viewModel.Files));
    }

    private FrameworkElement BuildRegistrySection()
    {
        var keyBox = new TextBox { PlaceholderText = @"HKCU\Software\VendorApp" };
        var nameBox = new TextBox { PlaceholderText = "InstallRoot" };
        var valueBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App" };
        var list = List(_viewModel.Registry, 120);
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
            list,
            RemoveButton(list, _viewModel.Registry));
    }

    private FrameworkElement BuildShortcutsSection()
    {
        var pathBox = new TextBox { PlaceholderText = @"{Desktop}\Vendor App.lnk" };
        var targetBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App\bin\app.exe" };
        var argumentsBox = new TextBox { PlaceholderText = "--optional" };
        var workingDirectoryBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App" };
        var descriptionBox = new TextBox { PlaceholderText = "Launch application" };
        var list = List(_viewModel.Shortcuts, 120);
        var addButton = new Button { Content = "Add", MinWidth = 72 };
        addButton.Click += async (_, _) =>
        {
            if (string.IsNullOrWhiteSpace(pathBox.Text) || string.IsNullOrWhiteSpace(targetBox.Text))
            {
                await ShowNoticeAsync("Shortcut Entry", "Path and target are required.");
                return;
            }

            _viewModel.AddShortcut(
                pathBox.Text,
                targetBox.Text,
                argumentsBox.Text,
                workingDirectoryBox.Text,
                descriptionBox.Text);
            pathBox.Text = string.Empty;
            targetBox.Text = string.Empty;
            argumentsBox.Text = string.Empty;
            workingDirectoryBox.Text = string.Empty;
            descriptionBox.Text = string.Empty;
        };

        return Section(
            "Shortcuts",
            TwoColumn(Labeled("Path", pathBox), Labeled("Target", targetBox)),
            TwoColumn(Labeled("Arguments", argumentsBox), Labeled("Working Directory", workingDirectoryBox)),
            Labeled("Description", descriptionBox),
            AlignRight(addButton),
            list,
            RemoveButton(list, _viewModel.Shortcuts));
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
        var list = List(_viewModel.Scripts, 112);
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
            list,
            RemoveButton(list, _viewModel.Scripts));
    }

    private FrameworkElement BuildPurgeSection()
    {
        var pathBox = new TextBox { PlaceholderText = @"{LocalAppData}\Vendor\App" };
        var branchBox = new TextBox { PlaceholderText = @"HKCU\Software\VendorApp" };
        var pathList = List(_viewModel.PurgePaths, 88);
        var branchList = List(_viewModel.PurgeRegistryBranches, 88);

        var addPathButton = new Button { Content = "Add Path", MinWidth = 96 };
        addPathButton.Click += (_, _) =>
        {
            _viewModel.AddPurgePath(pathBox.Text);
            pathBox.Text = string.Empty;
        };

        var addBranchButton = new Button { Content = "Add Branch", MinWidth = 104 };
        addBranchButton.Click += (_, _) =>
        {
            _viewModel.AddPurgeRegistryBranch(branchBox.Text);
            branchBox.Text = string.Empty;
        };

        return Section(
            "Purge",
            Labeled("Path", InputRow(pathBox, addPathButton)),
            pathList,
            RemoveButton(pathList, _viewModel.PurgePaths),
            Labeled("Registry Branch", InputRow(branchBox, addBranchButton)),
            branchList,
            RemoveButton(branchList, _viewModel.PurgeRegistryBranches));
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
        var validation = _viewModel.Validate();
        if (!validation.IsValid)
        {
            await ShowNoticeAsync("Validation", string.Join(Environment.NewLine, validation.Errors));
            return null;
        }

        if (!string.IsNullOrWhiteSpace(_lastSavedManifestPath))
        {
            await File.WriteAllTextAsync(_lastSavedManifestPath, _viewModel.TomlPreview);
            if (showNotice)
            {
                await ShowNoticeAsync("Manifest Saved", _lastSavedManifestPath);
            }
            return _lastSavedManifestPath;
        }

        var picker = new FileSavePicker();
        InitializeWithWindow.Initialize(picker, WindowNative.GetWindowHandle(this));
        picker.SuggestedStartLocation = PickerLocationId.DocumentsLibrary;
        picker.FileTypeChoices.Add("TOML manifest", [".toml"]);
        picker.SuggestedFileName = "install";

        var file = await picker.PickSaveFileAsync();
        if (file is null)
        {
            return null;
        }

        await FileIO.WriteTextAsync(file, _viewModel.TomlPreview);
        if (showNotice)
        {
            await ShowNoticeAsync("Manifest Saved", file.Path);
        }
        return file.Path;
    }

    private async Task ChooseOutputDirectoryAsync()
    {
        var picker = new FolderPicker();
        InitializeWithWindow.Initialize(picker, WindowNative.GetWindowHandle(this));
        picker.SuggestedStartLocation = PickerLocationId.DocumentsLibrary;
        picker.FileTypeFilter.Add("*");

        var folder = await picker.PickSingleFolderAsync();
        if (folder is not null)
        {
            _viewModel.OutputDirectory = folder.Path;
        }
    }

    private async Task GenerateInstallerAsync()
    {
        var tool = _viewModel.CovenantSetupTool;
        if (tool is null)
        {
            await ShowNoticeAsync("Installer EXE", "covenant-setup.exe was not found. Packaging is disabled.");
            return;
        }

        var validation = _viewModel.Validate();
        if (!validation.IsValid)
        {
            await ShowNoticeAsync("Validation", string.Join(Environment.NewLine, validation.Errors));
            return;
        }

        if (string.IsNullOrWhiteSpace(_viewModel.OutputDirectory))
        {
            await ShowNoticeAsync("Installer EXE", "Choose an output directory before packaging.");
            return;
        }

        var manifestPath = await WriteManifestWithPickerAsync(showNotice: false);
        if (manifestPath is null)
        {
            return;
        }
        _lastSavedManifestPath = manifestPath;

        _isPackaging = true;
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
            await ShowNoticeAsync("Installer EXE", detail);
        }
        catch (Exception ex)
        {
            await ShowNoticeAsync("Installer EXE", "Packaging failed: " + ex.Message);
        }
        finally
        {
            _isPackaging = false;
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
        await ShowNoticeAsync("TOML Preview", "Copied to clipboard.");
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

    private void RefreshStatusBrush()
    {
        if (_statusText is null)
        {
            return;
        }

        _statusText.Foreground = new SolidColorBrush(
            _viewModel.HasValidationErrors ? Colors.Firebrick : Colors.DarkGreen);
    }

    private void RefreshPackageControls()
    {
        if (_generateInstallerButton is null)
        {
            return;
        }

        var toolAvailable = _viewModel.HasCovenantSetupTool && !_isPackaging;
        _outputDirectoryBox.IsEnabled = toolAvailable;
        _chooseOutputButton.IsEnabled = toolAvailable;
        _generateInstallerButton.IsEnabled = _viewModel.CanPackage && !_isPackaging;
        _generateInstallerButton.Content = _isPackaging ? "Generating..." : "Generate Installer EXE";
        _toolStatusText.Foreground = new SolidColorBrush(
            _viewModel.HasCovenantSetupTool ? Colors.DarkGreen : Colors.Firebrick);
    }

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

    private static FrameworkElement Section(string title, params UIElement[] children)
    {
        var stack = new StackPanel { Spacing = 8 };
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

        return new Border
        {
            BorderBrush = new SolidColorBrush(Colors.Gainsboro),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(6),
            Padding = new Thickness(12),
            Child = stack
        };
    }

    private static FrameworkElement Labeled(string label, FrameworkElement control)
    {
        var stack = new StackPanel { Spacing = 4 };
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
        var grid = new Grid { ColumnSpacing = 8 };
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
        var grid = new Grid { ColumnSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
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

    private static ListView List(IEnumerable source, double height) => new()
    {
        ItemsSource = source,
        Height = height,
        SelectionMode = ListViewSelectionMode.Single
    };

    private static Button RemoveButton<T>(ListView list, ICollection<T> collection)
    {
        var button = new Button
        {
            Content = "Remove Selected",
            HorizontalAlignment = HorizontalAlignment.Right
        };
        button.Click += (_, _) =>
        {
            if (list.SelectedItem is T item)
            {
                collection.Remove(item);
            }
        };
        return button;
    }

    private static IReadOnlyList<string> SplitLines(string value) =>
        value.Split(
            [Environment.NewLine, "\n"],
            StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
}
