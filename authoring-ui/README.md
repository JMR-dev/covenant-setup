# Covenant Setup Authoring UI

WinUI 3 desktop app for creating `install.toml` manifests for `covenant-setup`.

Run from the repository root:

```powershell
dotnet run --project .\authoring-ui\Covenant.Setup.Authoring.csproj
```

The app edits the current manifest schema:

- `app_name`
- `directories`
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

When enabled, **Generate Installer EXE** writes the current TOML to the selected `install.toml` location and runs:

```powershell
covenant-setup.exe --json package install.toml --output <output-directory>
```

The manifest should be saved in the payload source root so relative file sources package correctly.
