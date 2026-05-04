# Covenant-Setup Smoke Test

This example stays in `HKCU` and `{LocalAppData}` so it can be exercised without elevation.

Build single-file installers:

```powershell
cargo run -- package examples/Covenant-SetupSampleApp-install.toml --output dist
```

This emits:

- `dist\covenant-setup-installer.exe`

The generated installer is a single executable with the manifest and payload embedded into it.
It chooses GUI or TUI mode from context, or you can force one explicitly with `--headed` or `--headless`.

Run install:

```powershell
cargo run -- install examples/Covenant-SetupSampleApp-install.toml --json
```

Write the journal somewhere explicit:

```powershell
cargo run -- install examples/Covenant-SetupSampleApp-install.toml --journal examples/journal.json
```

Run uninstall:

```powershell
cargo run -- uninstall examples/journal.json --json
```

Expected effects:

- Creates `%LOCALAPPDATA%\CovenantSetupExample`
- Copies `sample_app.cmd` into the `bin` directory
- Writes `HKCU\Software\CovenantSetupExample\InstallRoot`
- Creates a desktop shortcut
- Runs an inline PowerShell post-install command and records only the script execution in the journal
