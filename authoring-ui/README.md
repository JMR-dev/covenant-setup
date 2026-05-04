# Covenant Setup Authoring UI

WinUI 3 desktop app for creating `${AppName}-install.toml` manifests for `covenant-setup`. Manifest paths must not contain spaces; spaces in the app name are removed from the suggested file name. In manifest content, only `app_name` and shortcut `description` may contain spaces.

Run from the repository root:

```powershell
dotnet run --project .\authoring-ui\Covenant.Setup.Authoring.csproj
```

Run unit tests from the repository root:

```powershell
dotnet test .\authoring-ui.Tests\Covenant.Setup.Authoring.Tests.csproj
```

Build a release publish from the repository root:

```powershell
dotnet publish .\authoring-ui\Covenant.Setup.Authoring.csproj -c Release -r win-x64 --self-contained true -o .\dist\authoring-ui
```

The published app entry point is:

```powershell
.\dist\authoring-ui\Covenant.Setup.Authoring.exe
```

Keep the full `dist\authoring-ui` folder together when copying it to another Windows machine. The release output includes the WinUI, Windows App SDK, and .NET runtime files required by the app.

Build the Inno Setup installer from the repository root:

```powershell
iscc .\authoring-ui\Covenant.Setup.Authoring.iss
```

The installer output is:

```powershell
.\dist\inno\covenant-setup-authoring-installer.exe
```

The app edits the current manifest schema:

- `app_name`
- `directories.paths`
- `files`
- `registry`
- `shortcuts`
- `scripts`
- `purge`

It keeps a live TOML preview, validates required fields locally, warns when a manifest appears to require elevation, and saves a TOML file through the Windows file picker.

## Installer EXE Generation

The Installer EXE section is enabled only when the app finds `covenant-setup.exe`. It checks:

- next to the authoring app
- the current working directory
- `target\release` and `target\debug` under nearby repo roots
- directories on `PATH`

When enabled, **Generate Installer EXE** writes the current TOML to the selected `${AppName}-install.toml` location and runs:

```powershell
covenant-setup.exe --json package <AppName>-install.toml --output <output-directory>
```

The manifest should be saved in the payload source root so relative file sources package correctly.
