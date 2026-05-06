using Microsoft.UI.Xaml.Controls;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace Covenant.Setup.Authoring;

internal sealed partial class InstallerConfigDialog : ContentDialog
{
    private readonly InstallerConfigViewModel _viewModel;
    private readonly IntPtr _ownerWindowHandle;

    public InstallerConfigDialog(InstallerConfigViewModel viewModel, IntPtr ownerWindowHandle)
    {
        _viewModel = viewModel;
        _ownerWindowHandle = ownerWindowHandle;
        InitializeComponent();
        DataContext = viewModel;
    }

    private async void BrowseCovenantSetupButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs args)
    {
        var picker = new FileOpenPicker
        {
            SuggestedStartLocation = PickerLocationId.ComputerFolder
        };
        InitializeWithWindow.Initialize(picker, _ownerWindowHandle);
        picker.FileTypeFilter.Add(".exe");

        var file = await picker.PickSingleFileAsync();
        if (file is not null)
        {
            _ = await _viewModel.AcceptToolPathAsync(file.Path);
        }
    }

    private async void ChooseOutputDirectoryButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs args)
    {
        var picker = new FolderPicker
        {
            SuggestedStartLocation = PickerLocationId.DocumentsLibrary
        };
        InitializeWithWindow.Initialize(picker, _ownerWindowHandle);
        picker.FileTypeFilter.Add("*");

        var folder = await picker.PickSingleFolderAsync();
        if (folder is not null)
        {
            _viewModel.OutputDirectory = folder.Path;
        }
    }

    private async void SaveButton_Click(ContentDialog sender, ContentDialogButtonClickEventArgs args)
    {
        var deferral = args.GetDeferral();
        try
        {
            args.Cancel = !await _viewModel.SaveConfigAsync();
        }
        finally
        {
            deferral.Complete();
        }
    }
}
