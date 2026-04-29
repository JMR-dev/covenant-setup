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
- Uses a C# WinForms presentation process for GUI progress and prompts
- Sends GUI state over named-pipe IPC from the Rust engine to the C# UI
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

When the .NET SDK is available, the Rust build publishes a self-contained C# WinForms UI helper and embeds it into the Rust executable. Without that helper the installer still builds, but `--headed` falls back to terminal progress.

## Install and Uninstall Model

Direct engine commands:

```powershell
cargo run -- --headless install path\to\install.toml
cargo run -- --headless uninstall path\to\journal.json
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

There is now one installer/uninstaller path. UI mode must be explicit for interactive runs.

Explicit flags:

- `--headless`: force TUI
- `--headed`: force GUI
- `--json`: suppress UI and emit machine-readable events

If `--headed` is requested but the C# UI helper is not bundled and no `Covenant.Setup.Ui.exe` sidecar exists next to the installer, the engine falls back to `--headless`.

### GUI

Current GUI behavior includes:

- C# WinForms prompts for confirmation and completion
- a progress window with:
  - progress bar
  - current operation text
  - scrolling operations log
- reboot prompt when uninstall requires reboot to finish some cleanup

The Rust install engine owns all business logic and file/registry mutations. The C# process is presentation-only and receives JSON messages over a Windows named pipe.

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
- External-boundary calls (Win32, GUI prompts, reboot/cleanup spawning, embedded-bundle probe) flow through the `Sys` trait in [`src/sys.rs`](C:\Users\jasonross\workspace\covenant-setup\src\sys.rs); the production `WinSys` impl delegates to the real subsystems while `MockSys` records every call for unit tests
- Journaling currently records declared actions through `DeclaredTracker`
- The implementation is Windows-specific

## Current Limitations

- The GUI helper is embedded as a self-contained C# WinForms executable, which makes the installer binary substantially larger
- The installer is not yet generating branded/custom themed installer screens
- The manifest schema is still MVP-level and does not cover all production installer concerns
- Script execution logs the script invocation; internal script mutations are not observed beyond declared purge coverage
- The packager currently embeds payload in an appended raw bundle; this is functional but not yet compressed, signed, or tamper-resistant
- No signing, MSI generation, compression, delta updates, or patching pipeline exists yet
- No automated test suite has been added yet for end-to-end installer scenarios

## Verification Status

The codebase currently builds and formats successfully with:

```powershell
cargo fmt
cargo check
cargo build --release
```

Interactive GUI/TUI flows now have a Windows VM smoke harness for packaged installer behavior, while broader automated coverage is still limited.

## Linux Cross-Build (Optional)

Day-to-day work happens on Windows. This section documents an opt-in path for type-checking and running unit tests from a Linux host (useful for CI containers or quick iteration without a VM).

Prerequisites (Ubuntu 24.04 names):

- `dotnet-sdk-10.0` — optional for embedding the C# UI helper; without it, headed mode falls back to headless.
- `mingw-w64` — provides the `x86_64-w64-mingw32-*` toolchain that the `windows-gnu` target links against.
- `wine` — runs the resulting test binary. Already wired up via `runner = "wine"` in [`.cargo/config.toml`](.cargo/config.toml) for the `x86_64-pc-windows-gnu` target only.
- `rustup target add x86_64-pc-windows-gnu`.

The dotnet SDK needs `EnableWindowsTargeting=true` to cross-build a `net10.0-windows` project from Linux:

```bash
EnableWindowsTargeting=true cargo check --target x86_64-pc-windows-gnu
EnableWindowsTargeting=true cargo test  --target x86_64-pc-windows-gnu
```

Caveats:

- `cargo run` (and the `package`/`install`/`uninstall` flows generally) require real Win32 — Wine handles unit-test execution but is not a substitute for the Windows VM smoke harness.
- The `windows-msvc` target (the default on Windows) is unaffected by this section.

## Windows VM Smoke Test

A Windows Hyper-V Vagrant VM now lives in [`Vagrantfile`](C:\Users\jasonross\workspace\covenant-setup\Vagrantfile), and the host harness in [`scripts/run-windows-vm-smoke.ps1`](C:\Users\jasonross\workspace\covenant-setup\scripts\run-windows-vm-smoke.ps1) packages `covenant-setup`, boots the VM, opens Hyper-V's console viewer, and runs the packaged installer inside the guest's interactive desktop session.

The self-install manifest used for this path lives at [`vm/self-test/install.toml`](C:\Users\jasonross\workspace\covenant-setup\vm\self-test\install.toml). The guest verifies that install produced:

- `%LOCALAPPDATA%\CovenantSetupSelfTest\bin\covenant-setup.exe`
- `%LOCALAPPDATA%\CovenantSetupSelfTest\journal.json`
- `%LOCALAPPDATA%\CovenantSetupSelfTest\covenant-setup-uninstall.exe`
- `HKCU\Software\CovenantSetupSelfTest\InstallRoot`
- `Desktop\Covenant Setup Self Test.lnk`

It then immediately invokes the installed uninstaller with the generated journal and verifies that the install root, payload, journal, uninstaller, shortcut, application registry key, and Installed Apps registration are removed.

Run the smoke test from the repo root:

```powershell
$env:COVENANT_WINDOWS_BOX = "gusztavvargadr/windows-11"
$env:COVENANT_HYPERV_SWITCH = "Default Switch"
.\scripts\run-windows-vm-smoke.ps1
```

Notes:

- The Vagrant provider is `hyperv`, and the box you choose must support that provider.
- The harness opens `vmconnect.exe` after `vagrant up` so the guest desktop stays visible during the install.
- The default Vagrant synced folder is disabled to avoid SMB credential prompts; the harness uploads the installer and guest scripts over WinRM instead.
- The guest install is launched through an interactive scheduled task because WinRM sessions are not desktop-visible.
- The packaged installer now has a hidden automation mode that suppresses blocking GUI prompts while leaving the C# progress window visible for the VM smoke test.
- The guest scripts write a trace bundle under `dist\vagrant-self-test\trace` after each smoke run. It includes `guest-events.jsonl`, scheduler/process/event snapshots, Rust heartbeat files named `installer-heartbeat-*.jsonl`, and C# UI pipe logs named `csharp-ui-pipe-*.jsonl`.
- The Windows box should auto-log the `vagrant` user into the desktop session for the visual install path to appear.
- Set `COVENANT_HYPERV_SWITCH` to the Hyper-V virtual switch name you want Vagrant to use.
- The harness writes its verification artifact to `dist\vagrant-self-test\guest-result.json`.
- Use `-SkipViewer` if you do not want the harness to open the Hyper-V console window.

### VM Coverage Harness

In addition to the single-scenario smoke test, [`scripts/run-windows-vm-coverage.ps1`](C:\Users\jasonross\workspace\covenant-setup\scripts\run-windows-vm-coverage.ps1) walks every scenario manifest under `vm\<scenario>\install.toml` (`self-test`, `uac`, `hklm-registry`, `reboot`, `bundled-exec`) and delegates the in-guest assertions to [`scripts\windows-vm\coverage\<scenario>.ps1`](C:\Users\jasonross\workspace\covenant-setup\scripts\windows-vm\coverage). The scenarios exercise the elevation, MoveFileEx pending-rename / Restart Manager, HKLM-only registry, and bundled embedded-installer code paths that the unit tests stub out via `MockSys`.
- Use `-HaltAfter` or `-DestroyAfter` if you want the harness to stop the VM after the test run.
