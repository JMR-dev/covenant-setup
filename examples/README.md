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

- Creates `%LOCALAPPDATA%\CovenantSetupSample`
- Copies `sample_app.cmd` into the `bin` directory and `post_install.ps1` into the install root
- Writes `HKCU\Software\CovenantSetupSample\InstallRoot`
- Creates a desktop shortcut
- Runs `post_install.ps1` from the install root, which writes a timestamped marker under `logs\`

Every directory, file, registry value, shortcut, and script execution is recorded in the
journal, so uninstall reverses all of them (and the `purge` spec removes the whole install
root, including the `logs\` directory the script created).
