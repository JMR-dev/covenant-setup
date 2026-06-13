# Integration Tests Architecture

## Background

Rust unit-test coverage stalled at **70.09% line coverage**
(main.rs 74.52%, ui.rs 31.44%, win.rs 75.99%).
The remaining uncovered lines are at hard external boundaries that cannot be
exercised by pure unit tests:

| Boundary | Why it can't be unit-tested directly |
|---|---|
| UAC relaunch (`ShellExecuteW` with verb `runas`) | Spawns a new elevated process; no return value to observe |
| Reboot prompt + `shutdown.exe` spawn | System-level side effect; mutates host state |
| Cleanup-helper self-delete (copy exe → temp → `cmd /c del`) | Operates on the current process's own binary |
| HKLM registry writes | Requires admin; state persists on the host |
| Bundled-installer execution (`has_embedded_bundle()` + dispatch) | Requires a self-contained EXE with appended payload |
| Live GUI progress IPC (`CSharpUiSession` named-pipe child) | Spawns a real WinForms process |
| `MoveFileEx` reboot fallback for locked files | Requires a second process holding a file handle |

The solution is a two-layer approach:
1. **Trait-based mocking** for unit tests — inject a recording `MockSys` so
   the orchestration logic can be driven without any Win32/process side effects.
2. **Vagrant VM integration tests** for real boundary validation — run each
   scenario inside a fresh Hyper-V Windows 11 guest.

---

## Trait Layer (`src/sys.rs` and `src/ui.rs`)

### `Sys` trait (`src/sys.rs`)

```
pub(crate) trait Sys: Send + Sync { … }
```

Groups all seven external boundaries into a single injectable surface.
The production implementation `WinSys` delegates each method to the existing
free functions in `win.rs`, `ui.rs`, and `main.rs`:

```
Sys method                       → delegates to
─────────────────────────────────────────────────────────────────────────────
is_elevated / relaunch_as_admin  → win::is_elevated / win::relaunch_as_admin
spawn_reboot                     → spawn_reboot() (main.rs)
prompt_reboot_tui                → prompt_reboot_tui() (main.rs)
spawn_cleanup_helper             → spawn_cleanup_helper() (main.rs)
schedule_helper_self_cleanup     → schedule_helper_self_cleanup() (main.rs)
set_registry_string              → win::set_registry_string
delete_registry_tree             → win::delete_registry_tree
has_embedded_bundle              → has_embedded_bundle() (main.rs)
ui_available / ui_confirm_install
  / ui_report_success / …        → ui::* free functions
remove_file_with_fallback        → win::remove_file_with_fallback
```

All Win32 functions remain in `win.rs`. `sys.rs` contains no `unsafe` code;
it is purely a delegation and trait-abstraction layer.

An optional `start_progress` method (default returns `None`) lets `MockSys`
inject a recording `ProgressSink` into install/uninstall without touching the
real C# UI.

### `ProgressSink` trait (`src/ui.rs`)

```
pub trait ProgressSink: Send {
    fn advance(&mut self, current_step, message) → Result<(), AppError>;
    fn log(&mut self, message) → Result<(), AppError>;
    fn finish(&mut self, message) → Result<(), AppError>;
    fn fail(&mut self, app_name, operation, message, error, errata, wait_for_close)
        → Result<(), AppError>;
}
```

`GuiProgress` implements this trait. Install/uninstall functions now accept
`Option<Box<dyn ProgressSink>>` rather than `Option<GuiProgress>` directly,
allowing injection of a no-op or recording sink in tests.

### Call-site threading

The `&dyn Sys` reference flows through:

```
main()
 └── run(cli, sys, logger)
      ├── run_bundled_installer(prefs, sys, logger)
      ├── install(manifest, opts, sys, logger) → Option<Box<dyn ProgressSink>>
      │    └── register_uninstall_entry(…, sys, logger)
      ├── uninstall(journal, opts, sys, logger)
      └── cleanup(…, sys, logger)
           └── ensure_elevation_if_needed(…, sys, logger)
```

The production `WinSys` value is constructed once in `main()` and borrowed
everywhere below. Existing tests that call these functions directly pass
`&WinSys` unchanged; mock tests pass `&MockSys`.

---

## Mock Layer (in `src/main.rs` `#[cfg(test)]`)

### `MockSys`

```rust
struct MockSys {
    is_elevated: Mutex<bool>,             // programmable probe result
    ui_confirm: Mutex<bool>,             // programmable confirm result
    reboot_prompt: Mutex<bool>,          // programmable reboot prompt result
    schedule_cleanup_returns: Mutex<bool>,
    has_bundle: bool,
    calls: Mutex<Vec<SysCall>>,          // all recorded calls
}
```

Every `Sys` method appends a `SysCall` enum variant to `calls` before
returning. Tests assert on the recorded call sequence:

```rust
let sys = MockSys::new();
ensure_elevation_if_needed(true, true, &sys, &logger)?;
assert!(sys.recorded().contains(&SysCall::RelaunchAsAdmin));
```

### `MockProgressSink`

Records `advance`, `log`, `finish`, and `fail` calls in a `Vec<ProgressCall>`.
Injected via `MockSys::start_progress` so the install codepath exercises all
`advance_gui_progress` / `finish_gui_progress` / `fail_gui_progress` calls.

### New unit tests (14 total, in `mod tests`)

| Test | Boundary exercised |
|---|---|
| `ensure_elevation_if_needed_relaunches_when_required_and_relaunch_flag_set` | UAC relaunch path |
| `ensure_elevation_if_needed_errors_when_required_and_no_relaunch` | UAC error message |
| `ensure_elevation_if_needed_passes_when_already_elevated` | UAC no-op path |
| `cleanup_prompts_and_spawns_reboot_when_required_in_gui_mode` | Reboot spawn |
| `cleanup_skips_reboot_when_user_declines` | Reboot prompt negative |
| `cleanup_tui_path_skips_prompt_when_no_reboot_needed` | Cleanup TUI path |
| `register_uninstall_entry_writes_all_seven_values` | Registry write count |
| `run_bundled_installer_dispatches_install_and_reports_success_in_gui` | Bundled exec + UI report |
| `run_bundled_installer_reports_error_when_install_fails` | UI error path |
| `install_emits_set_registry_string_calls_for_each_registry_spec` | Registry write content |
| `uninstall_calls_delete_registry_tree_for_recorded_actions_and_purge` | Registry delete order |
| `uninstall_calls_remove_file_with_fallback_for_copy_actions_and_shortcuts` | MoveFileEx delegation |
| `uninstall_defers_self_delete_to_spawn_cleanup_helper` | Cleanup helper dispatch |
| `progress_sink_mock_records_calls_through_advance_log_finish_fail` | ProgressSink recording |

---

## Vagrant Integration Tests

Real boundary validation runs inside a Hyper-V Windows 11 VM
(`gusztavvargadr/windows-11`). Every install/uninstall side effect stays
inside the VM; the host only builds the binary and drives Vagrant over WinRM.

### File layout

```
vm/
  self-test/CovenantSetupSelfTest-install.toml        Legacy smoke test (HKCU + LocalAppData)
  uac/CovenantSetupUACScenario-install.toml           ProgramFiles target → forces elevation probe
  hklm-registry/CovenantSetupHKLMRegistryScenario-install.toml   HKLM registry key → forces elevation via root
  reboot/CovenantSetupRebootScenario-install.toml     Payload + script that self-locks file
  bundled-exec/CovenantSetupBundledExecScenario-install.toml     Packaged installer bundle (HKCU + LocalAppData)

scripts/
  run-windows-vm-coverage.ps1             Host-side orchestrator
  windows-vm/coverage/
    self-test.ps1               Guest-side assertion script
    uac.ps1
    hklm-registry.ps1
    reboot.ps1
    bundled-exec.ps1
```

### Orchestrator (`scripts/run-windows-vm-coverage.ps1`)

Mirrors the pattern of `run-windows-vm-smoke.ps1`:

1. `cargo build --release` on the host (skippable with `-SkipBuild`).
2. Stages `payload\covenant-setup.exe` into each scenario directory that
   references it (manifests that do file installs need a payload binary).
3. `vagrant up --provider hyperv` (skippable with `-SkipVmBoot`).
4. Waits for the Windows explorer shell to be responsive.
5. Uploads `covenant-setup.exe` + all scenario directories + all guest scripts
   into `C:\Users\vagrant\AppData\Local\Temp\covenant-setup-coverage\` on the
   guest.
6. For each scenario:
   - WinRM-invokes `<scenario>.ps1 -Exe … -Manifest … -WorkRoot …` in the guest.
   - Captures guest log via a second WinRM call.
   - Records `{ scenario, success, exitCode }`.
7. Writes `dist\vagrant-coverage\summary.json` with aggregated results.
8. Optionally halts or destroys the VM (`-HaltAfter` / `-DestroyAfter`).

Guest scripts run with `Set-StrictMode -Version Latest` and
`$ErrorActionPreference = 'Stop'`, matching the existing harness conventions.

### Scenario descriptions

#### `self-test`
Baseline parity with the legacy smoke test. Installs to `%LocalAppData%`,
asserts the journal records directory/file/registry/shortcut actions,
then runs uninstall and verifies all recorded paths are removed.

#### `uac`
Manifest targets `{ProgramFilesX64}`, which makes the elevation probe flag the
install as needing admin. The script asserts:
1. Running without `--elevate` fails with exit ≠ 0 and the message
   `"Elevation required"`.
2. Running with `--elevate` inside the already-elevated WinRM session succeeds.
3. Elevated uninstall cleans up without error.

#### `hklm-registry`
Manifest writes a key under `HKLM\Software\…`, which triggers the
registry-root elevation check independently of file paths. Assertions mirror
the UAC scenario: fail without `--elevate`, succeed with it, verify the
journal records an HKLM `write_registry` action.

#### `reboot`
Installs a file payload, then the scenario script locks the installed binary
by spawning it in a background process before running uninstall. The uninstaller
must fall back to `MoveFileEx(MOVEFILE_DELAY_UNTIL_REBOOT)` via the Restart
Manager path. The script asserts the uninstall log contains a
`reboot_required` / `pending_rename` / `MoveFileEx` marker.
The background lock process is stopped after uninstall so the VM stays clean.

#### `bundled-exec`
Exercises the self-contained installer packaging pipeline end-to-end:
1. `covenant-setup package <manifest> --output <dir>` produces a bundled EXE.
2. The bundled EXE is invoked with **no subcommand** (`--json --headless
   --automation install --journal …`), which triggers the
   `has_embedded_bundle()` probe path in `main()`.
3. The journal is parsed and must contain at least one recorded action.
4. Standard uninstall cleans up.

---

## Running

### Unit tests (local, safe)

```powershell
# Rust: 96 tests including the 14 mock-based boundary tests.
cargo test

# C# UI: 36 xUnit tests covering pure helpers in Program.cs.
dotnet test ui\Covenant.Setup.Ui.Tests\Covenant.Setup.Ui.Tests.csproj
```

No Win32, registry, or process side effects in either suite.

#### C# UI unit tests (`ui/Covenant.Setup.Ui.Tests/`)

The WinForms host (`ui/Covenant.Setup.Ui/Program.cs`) follows the same
"extract pure logic, mock the boundary" pattern used on the Rust side:

| Helper (`internal static`) | What it does | Tests |
|---|---|---|
| `Program.ReadPipeName` | Parses `--pipe <name>` from `args` | 6 cases: present, case-insensitive flag, mid-args, missing, dangling flag, empty |
| `InstallerUiForm.BuildErrataJson` | Serializes `message.Errata` if present, else a synthesized `{app_name, operation, message, error}` payload | 3 branches: object errata, null `Errata`, JSON `null` element |
| `InstallerUiForm.SafeMessageSummary` | Best-effort `(type,id,message)` extraction for tracing; falls back to `{RawLength}` on parse failure | Valid JSON, missing fields, invalid JSON |
| `InstallerUiForm.MapButtons` / `MapIcon` / `MapDialogResult` | String ↔ WinForms enum translation between the IPC wire format and `MessageBox*` types | Exhaustive `[Theory]` tables incl. defaults and `DialogResult.Abort/Retry/Ignore` |
| `UiMessage` / `UiResponse` JSON contract | Snake-case ↔ PascalCase mapping (`app_name`, `current_step`, `total_steps`, etc.) | Round-trip tests covering progress, fail (with errata), prompt, missing-type |

Conventions:
- Production members are `internal` (not `public`); the production csproj
  declares `<InternalsVisibleTo Include="Covenant.Setup.Ui.Tests" />` so the
  test assembly can reach them without widening the public API.
- Tests never instantiate `InstallerUiForm` directly — its constructor builds
  real `Control` instances and is not unit-testable. Only static helpers are
  exercised. Live form behaviour is covered by the GUI scenario in the
  Vagrant harness instead.
- Anonymous-object return values (`SafeMessageSummary`) are asserted by
  serializing the result and parsing the JSON, which avoids reflection-based
  property lookups against the compiler-generated anonymous type.

### Integration tests (requires Hyper-V + Vagrant)

```powershell
# Full run: boot VM, run all scenarios, halt VM
.\scripts\run-windows-vm-coverage.ps1 -HaltAfter

# Skip rebuild if binary is already current
.\scripts\run-windows-vm-coverage.ps1 -SkipBuild -HaltAfter

# Skip VM boot if it's already running
.\scripts\run-windows-vm-coverage.ps1 -SkipVmBoot -HaltAfter

# Run only specific scenarios
.\scripts\run-windows-vm-coverage.ps1 -Scenarios @('uac','hklm-registry') -HaltAfter
```

Results are written to `dist\vagrant-coverage\summary.json`.
Per-scenario guest logs are in `dist\vagrant-coverage\<scenario>\guest.log`.

---

## Design decisions

**Single `Sys` trait rather than seven separate traits.** The orchestration
functions (`install`, `uninstall`, `cleanup`, etc.) each touch three or four
boundaries in combination. A single injectable surface keeps signature noise
minimal and makes `MockSys` straightforward to construct.

**No test-only methods on `Sys`.** `start_progress` has a production-viable
default (`None`), so the trait contains no `#[cfg(test)]` methods. The mock
simply overrides it.

**Win32 code stays in `win.rs`.** `sys.rs` contains zero `unsafe` blocks.
It delegates to the already-audited Win32 wrappers rather than duplicating them.

**Guest scripts are the assertion layer, not PowerShell DSL helpers.** Each
`scripts/windows-vm/coverage/<scenario>.ps1` is a self-contained script that
installs, asserts, and uninstalls. There is no shared PowerShell assertion
library to maintain.

**Vagrant is the only real-boundary test channel.** Boundaries involving
UAC, HKLM writes, locked files, and bundled execution are not exercised on the
host dev machine. The orchestrator will always fail if Vagrant is not available,
which is intentional.
