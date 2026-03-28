# covenant-setup

`covenant-setup` is a Windows installer builder and install engine written in Rust.

## Why a different Windows installer/uninstaller packager?

Windows has a mess when it comes to managing program lifecycles. Developers can leave files everywhere on install, the OS lets you do ANYTHING if you elevate to admin, and the uninstall process has no idea what files and registry entries were actually created during the install, leaving behind a mess and contributing to registry rot.

This packager aims to take a different approach by

- Observing all the places a program installs to during installation and during any post-install scripts/operations and then writing a journal.json to the same directory the application installs to. This file is referenced during uninstall to return the machine back to the state it was before the install with any files and registry entries associated with that program.
- Take a "leave the campground better than you found it" approach - this Eagle Scout practices Leave No Trace.
- Taking a "trust but verify model" to program installs and uninstalls, observing program behavior during install and uninstall in order to respect the user.
- Using the `journal.json` as a manifest of everything the program did during the install and post install process.

Its current shape is:

- a packager that takes a developer-authored `install.toml`
- a single-file installer runtime with the app payload embedded into the `.exe`
- an installed uninstaller path that reuses the same Rust engine

## Current Capabilities

- Packages an app from a manifest into a single installer executable
- Installs files, directories, registry values, shortcuts, and post-install scripts
- Journals applied mutations to support deterministic uninstall
- Uninstalls in reverse order and purges declared registry/path namespaces
- Registers the installed app in Windows Installed Apps / Add-Remove Programs
- Creates an installed uninstaller executable in the app root
- Uses Win32 APIs through the `windows` crate with unsafe isolated in [`src/win.rs`](C:\Users\jasonross\workspace\covenant-setup\src\win.rs)
- Logs every unsafe boundary transition

## Packaging Model

The packager command is:

```powershell
cargo run -- package path\to\install.toml --output dist
```

Current output:

- `dist\covenant-setup-installer.exe`

That installer is a single executable. The manifest and payload files are embedded into the binary and extracted to a temporary working directory at runtime.

## Install and Uninstall Model

Direct engine commands:

```powershell
cargo run -- install path\to\install.toml
cargo run -- uninstall path\to\journal.json
```

Packaged installer behavior:

- Running the packaged installer with no subcommand performs install
- The installed app gets:
  - `journal.json` in the install root
  - `covenant-setup-uninstall.exe` in the install root
  - an uninstall registry entry under `...\CurrentVersion\Uninstall\...`

Installed-app uninstall behavior:

- Windows Installed Apps launches the installed uninstaller executable
- The engine removes payload files first
- A cleanup helper from `%TEMP%` removes the running uninstaller after it exits
- If immediate cleanup is impossible, file removal falls back to delete-on-reboot

## UI Behavior

There is now one installer/uninstaller path. UI mode is chosen by context unless explicitly overridden.

Explicit flags:

- `--headless`: force TUI
- `--headed`: force GUI

Current automatic behavior:

- If launched from PowerShell / `pwsh`, uninstall prefers TUI
- If launched from Windows GUI context, install/uninstall prefer GUI
- Otherwise the engine can run without extra UI

### GUI

Current GUI behavior includes:

- native message-box prompts for confirmation and completion
- a progress window with:
  - progress bar
  - current operation text
  - scrolling operations log
- reboot prompt when uninstall requires reboot to finish some cleanup

### TUI

Current TUI behavior includes:

- `Installing {app_name}` or `Uninstalling {app_name}`
- animated walking dots from 0 to 5, cycling every 500ms
- final success / reboot-needed text prompts

## Manifest Scope

The current manifest supports:

- `directories`
- `files`
- `registry`
- `shortcuts`
- `scripts`
- `purge`

The sample manifest lives at [`examples/install.toml`](C:\Users\jasonross\workspace\covenant-setup\examples\install.toml).

## Architecture Notes

- Core engine flow is in [`src/main.rs`](C:\Users\jasonross\workspace\covenant-setup\src\main.rs)
- Windows FFI wrappers are isolated in [`src/win.rs`](C:\Users\jasonross\workspace\covenant-setup\src\win.rs)
- Journaling currently records declared actions through `DeclaredTracker`
- The implementation is Windows-specific

## Current Limitations

- The GUI layer is currently implemented through a PowerShell-hosted WinForms progress window rather than a native Rust GUI framework
- The installer is not yet generating branded/custom themed installer screens
- The manifest schema is still MVP-level and does not cover all production installer concerns
- Script execution logs the script invocation; internal script mutations are not observed beyond declared purge coverage
- The packager currently embeds payload as JSON-appended data; this is functional but not yet optimized for large payloads or tamper-resistance
- No signing, MSI generation, compression, delta updates, or patching pipeline exists yet
- No automated test suite has been added yet for end-to-end installer scenarios

## Verification Status

The codebase currently builds and formats successfully with:

```powershell
cargo fmt
cargo check
```

Interactive GUI/TUI flows have been exercised during development, but there is not yet a formal automated integration harness for packaged installer behavior.
