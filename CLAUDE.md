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
cargo run -- package examples/install.toml --output dist

# Install: apply a manifest directly (or from embedded bundle)
cargo run -- install examples/install.toml --json

# Uninstall: reverse all journaled actions
cargo run -- uninstall examples/journal.json --json
```

No automated test suite exists yet. Manual testing uses the example manifest (`examples/install.toml`).

## Architecture

**Two source files:**
- `src/main.rs` — CLI (clap derive), manifest parsing, install/uninstall/package logic, journaling, UI (TUI/GUI/JSON), elevation handling
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
- UTF-16 conversion uses the `Utf16Arg` wrapper type
- Registry always uses `KEY_WOW64_64KEY` for explicit 64-bit access
- Path tokens (`{ProgramFilesX64}`, etc.) are resolved at runtime, never hardcoded
- Subprocess calls use `CREATE_NO_WINDOW` flag
- Rust edition 2024
