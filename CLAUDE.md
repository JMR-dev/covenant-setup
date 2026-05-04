# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Covenant-Setup is a Windows installer engine written in Rust. It deterministically tracks all system mutations (files, directories, registry keys, shortcuts, scripts) via a journaling model, enabling exact rollback on uninstall. Windows-only; all system operations use Win32 APIs directly.

## Build & Run Commands

```bash
cargo fmt              # Format code
cargo check            # Type-check without building
cargo build            # Debug build
cargo build --release  # Release build

# Package: bundle manifest + payload into a single-file installer EXE
cargo run -- package examples/Covenant-SetupSampleApp-install.toml --output dist

# Install: apply a manifest directly (or from embedded bundle)
cargo run -- install examples/Covenant-SetupSampleApp-install.toml --json

# Uninstall: reverse all journaled actions
cargo run -- uninstall examples/journal.json --json
```

No Rust automated test suite exists yet beyond the in-tree `#[cfg(test)]`
unit tests (`cargo test`, 96 tests). C# UI unit tests live in a sibling
project and run via:

```bash
dotnet test ui/Covenant.Setup.Ui.Tests/Covenant.Setup.Ui.Tests.csproj
```

Real Win32/UAC/registry boundaries are validated by the Vagrant harness
(`scripts/run-windows-vm-coverage.ps1`) — see
`docs/integration-tests-architecture.md`. Manual interactive testing uses
the example manifest (`examples/Covenant-SetupSampleApp-install.toml`).

## Architecture

**Three source files:**
- `src/main.rs` — CLI (clap derive), manifest parsing, install/uninstall/package logic, journaling, UI (TUI/GUI/JSON), elevation handling
- `src/sys.rs` — `Sys` trait abstracting every external boundary (Win32 elevation/registry/MoveFileEx fallback, reboot, cleanup-helper spawn, embedded-bundle probe, GUI prompts, optional `ProgressSink` injection). `WinSys` is the production implementation that delegates to `crate::win::*`, `crate::ui::*`, and the local helpers; `MockSys` (in `mod tests`) records every call for unit tests.
- `src/win.rs` — All Win32 FFI isolated here. Every `unsafe` block is bracketed with `logger.unsafe_enter()`/`unsafe_exit()` calls. Contains `PathResolver` for known-folder token resolution, file/directory/registry/shortcut operations, Restart Manager queries, and elevation checks.

**Three operational modes (CLI subcommands):**
1. `package` — Reads TOML manifest, embeds it + payload files into the EXE binary using an append format (JSON payload + u64 size + magic footer `COVENANT_SETUP_BUNDLE_V1`)
2. `install` — Parses manifest (from file or embedded bundle), executes mutations in order, writes `journal.json`, registers in Add/Remove Programs
3. `uninstall` — Reads `journal.json`, reverses actions in LIFO order, handles locked files via Restart Manager + `MoveFileEx` reboot fallback, spawns cleanup helper for self-deletion

**Key types:**
- `InstallManifest` — Declarative TOML contract: directories, files, registry, shortcuts, scripts, purge spec
- `Journal` / `JournalAction` — Serialized record of every mutation for deterministic rollback
- `MutationTracker` trait — Extensibility point (MVP uses `DeclaredTracker`; future: `ObservedTracker` for ETW-based capture)
- `PathResolver` — Resolves `{ProgramFilesX64}`, `{LocalAppData}`, `{Desktop}` tokens via `SHGetKnownFolderPath`
- `Logger` — Dual-mode output: structured JSON (`--json` flag) for IPC or human-readable text

**Elevation:** Manifest/journal is scanned for `HKLM` registry or ProgramFiles paths to determine if admin is needed. Auto-relaunches via `ShellExecuteW` with `runas` when `--elevate` flag is set. Exit code 33 signals elevation required.

**UI modes:** `--headless` forces TUI, `--headed` forces GUI (PowerShell-hosted WinForms), auto-detected from parent process otherwise. JSON mode (`--json`) is for programmatic consumers.

## Conventions

- All Win32 calls go in `src/win.rs`, never in `main.rs`
- All external boundaries (`win::*`, `ui::*` prompts, reboot/cleanup-helper spawning, embedded-bundle probe) flow through the `Sys` trait in `src/sys.rs` so orchestration code can be unit-tested with `MockSys`
- UTF-16 conversion uses the `Utf16Arg` wrapper type
- Registry always uses `KEY_WOW64_64KEY` for explicit 64-bit access
- Path tokens (`{ProgramFilesX64}`, etc.) are resolved at runtime, never hardcoded
- Subprocess calls use `CREATE_NO_WINDOW` flag
- Rust edition 2024

## VM coverage harness

`scripts\run-windows-vm-coverage.ps1` walks every scenario directory under `vm\<scenario>\install.toml` and delegates per-scenario in-guest assertions to `scripts\windows-vm\coverage\<scenario>.ps1`. The bundled scenarios (`self-test`, `uac`, `hklm-registry`, `reboot`, `bundled-exec`) exercise the elevation, MoveFileEx pending-rename, HKLM-registry, and embedded-bundle code paths. The harness builds the release binary and dispatches scenarios in-place; pass `-SkipBuild` to reuse a prior build.
