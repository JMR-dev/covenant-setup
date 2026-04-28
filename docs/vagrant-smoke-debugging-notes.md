# Vagrant Smoke Test Debugging Notes

This document records the work done while replacing the PowerShell UI with a C# presentation layer, adding guest-side diagnostics, and stabilizing the Windows Vagrant smoke test. It is intended as a reference for future installer hangs where the VM console is not visible.

## Scope

The work covered these areas:

- Removed the PowerShell-hosted UI path.
- Added a C# WinForms UI process.
- Connected Rust business logic to the C# UI over Windows named pipes.
- Added guest trace collection so hangs can be diagnosed without watching the VM console.
- Rebuilt and tested the packaged installer in the Hyper-V Vagrant guest.
- Extended the smoke test to install, verify installed state, uninstall, and verify removed state.

## Current Architecture

The installer is still driven by Rust. The C# process is presentation only.

- Rust business logic lives primarily in `src/main.rs`.
- Rust C# UI IPC lives in `src/ui.rs`.
- C# WinForms UI lives in `ui/Covenant.Setup.Ui/Program.cs`.
- `build.rs` publishes the C# UI as a self-contained `win-x64` single-file executable.
- The Rust binary embeds the published C# UI executable with `include_bytes!`.
- At runtime, Rust extracts the C# UI executable to `%TEMP%\covenant-setup-ui`, starts it, and connects to a named pipe.
- The C# UI owns the pipe server and reads newline-delimited JSON messages.
- Rust sends messages such as `init`, `progress`, `log`, `finish`, `prompt`, and `close`.
- The C# UI writes prompt responses back as JSON.

## Trace Outputs

The guest trace directory is:

```text
C:\Users\vagrant\AppData\Local\Temp\covenant-setup-smoke\trace
```

The host harness pulls this into:

```text
dist\vagrant-self-test\trace
```

Important trace files:

- `guest-events.jsonl`: host/guest harness events, scheduled task status, verification phases.
- `installer-heartbeat-<pid>.jsonl`: Rust process heartbeat and installer phases.
- `csharp-ui-pipe-<pid>.jsonl`: C# UI process and pipe receive/send events.
- `interactive-context.json`: context captured by the interactive wrapper.
- `interactive-processes.json`: relevant guest processes during wrapper diagnostics.
- `interactive-windows.json`: visible windows and process window titles.
- `interactive-application-events.json`: recent Application event log entries.
- `abort-*.json`: snapshots created by the abort collector.

The host harness always tries to pull the trace bundle in `finally`, even when the smoke test fails.

## Errors Encountered

### 1. Host Sandbox and Vagrant Permissions

Running Vagrant and Hyper-V actions from the coding sandbox required escalation. This affected commands such as:

```powershell
.\scripts\run-windows-vm-smoke.ps1 -SkipViewer -HaltAfter
vagrant status
vagrant winrm ...
vagrant upload ...
```

This was expected: Vagrant controls an external VM, uses WinRM, and interacts with Hyper-V.

### 2. Pre-main Guest Failure: Missing VCRUNTIME140.dll

The first meaningful guest diagnostics showed a Windows system error dialog:

```text
covenant-setup-installer.exe - System Error
The code execution cannot proceed because VCRUNTIME140.dll was not found.
```

Evidence:

- `interactive-windows.json` showed a `covenant-setup-installer.exe - System Error` window.
- `system-events.json` had an `Application Popup` event for the missing DLL.
- There were no `installer-heartbeat-*.jsonl` files.
- There were no `csharp-ui-pipe-*.jsonl` files.

Conclusion:

The executable failed before Rust `main()` ran. The heartbeat and pipe logs were absent because neither Rust nor the C# UI started.

Fix:

Added `.cargo/config.toml`:

```toml
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

This statically links the MSVC C runtime into the Rust executable, removing the guest dependency on `VCRUNTIME140.dll`.

### 3. Packaging Looked Successful but Produced an Unbundled EXE

Manual PowerShell invocations of the Windows-subsystem Rust executable were misleading. A direct command such as:

```powershell
target\release\covenant-setup.exe --json package vm\self-test\install.toml --output dist\vagrant-self-test
```

could return quickly with `EXIT=0` while the output file still matched the base executable size.

Reason:

The Rust binary is built as a Windows GUI subsystem executable. Direct invocation from PowerShell does not behave like a normal console command in all cases.

Fixes:

- The smoke harness invokes the packager with `Start-Process -Wait -PassThru`.
- The harness now validates that the packaged installer ends with the bundle magic marker `COVENANT_SETUP_BUNDLE_V1`.
- The embedded bundle format was changed from JSON byte arrays to an appended raw bundle with a JSON index plus raw file data. This avoids large JSON expansion of payload bytes.

### 4. Journal Written Beside the Smoke Installer

After the VCRUNTIME fix, the installer succeeded but the smoke verifier failed with:

```text
Journal missing: C:\Users\vagrant\AppData\Local\CovenantSetupSelfTest\journal.json
```

Evidence:

The Rust heartbeat showed:

```json
{"phase":"install_journal_written","detail":{"journal":"C:\\Users\\vagrant\\AppData\\Local\\Temp\\covenant-setup-smoke\\journal.json"}}
```

Cause:

The packaged install path was passing an explicit journal path next to the packaged installer. The expected product behavior is to infer the install root and write `journal.json` there.

Fix:

`run_bundled_installer` now calls:

```rust
install(&manifest_path, None, true, ui_mode, logger)
```

This lets `build_install_runtime` infer the journal path from the install root.

### 5. Scheduled Task Re-ran During Diagnostics

The guest harness originally registered a scheduled task with a trigger one minute in the future and also started it manually.

Failure mode:

- The manual task run completed.
- If verification or diagnostics took long enough, the scheduled trigger fired and launched a second copy.

Fix:

The trigger is now set far in the future:

```powershell
New-ScheduledTaskTrigger -Once -At (Get-Date).AddDays(1)
```

The harness still starts the task manually with `Start-ScheduledTask`.

### 6. WinRM Error 1726 During Success Diagnostics

After the installer succeeded, the host sometimes saw:

```text
WSMAN ERROR CODE: 1726
The WSMan provider host process did not return a proper response.
```

The trace showed the installer succeeded and verification reached the success diagnostics phase, but the WinRM command failed while returning.

Fix:

- The success path now writes `guest-result.json` immediately after verification.
- Heavy diagnostics are retained for failure paths.
- Diagnostic file writes now emit `diagnostic_file_start`, `diagnostic_file_finish`, and `diagnostic_file_error` markers so future diagnostic hangs show the exact capture that blocked.

### 7. Install-plus-uninstall Harness Hung

When uninstall testing was added, the install scheduled task exited with `LastTaskResult=1`. The parent loop waited until timeout because no result file was written.

Evidence:

- `guest-events.jsonl` showed the install scheduled task was registered and started.
- The task quickly moved to `Ready` with `LastTaskResult=1`.
- There was no `interactive_installer_start`.
- There was no Rust heartbeat.
- There was no C# pipe log.

This meant the PowerShell wrapper failed before starting the installer.

Reproduction:

A harmless wrapper test failed:

```powershell
Invoke-InteractiveInstaller.ps1 `
  -InstallerPath C:\Windows\System32\cmd.exe `
  -InstallerArguments "/c" "exit 0"
```

Error:

```text
A positional parameter cannot be found that accepts argument 'exit 0'.
```

Cause:

Under `powershell.exe -File`, passing multiple values to a script `[string[]]` parameter was not binding as intended.

Fix:

The task wrapper now passes child process arguments as base64-encoded JSON:

```powershell
$argumentsJson = ConvertTo-Json -InputObject $Arguments -Compress
$argumentsBase64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($argumentsJson))
```

The interactive wrapper decodes that back to a real argument array:

```powershell
$argumentsJson = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($InstallerArgumentsBase64))
$decodedArguments = ConvertFrom-Json -InputObject $argumentsJson
$InstallerArguments = @()
foreach ($argument in $decodedArguments) {
    $InstallerArguments += [string]$argument
}
```

The harmless wrapper test then produced:

```json
"installerArgs": ["/c", "exit 0"],
"exitCode": 0
```

## Uninstall Test Flow

The smoke harness now does this inside the guest:

1. Schedules an interactive install task.
2. Runs the packaged installer with:

   ```text
   --headed --automation
   ```

3. Verifies installed state:

   - `%LOCALAPPDATA%\CovenantSetupSelfTest\bin\covenant-setup.exe`
   - `%LOCALAPPDATA%\CovenantSetupSelfTest\journal.json`
   - `%LOCALAPPDATA%\CovenantSetupSelfTest\covenant-setup-uninstall.exe`
   - `HKCU:\Software\CovenantSetupSelfTest`
   - `Desktop\Covenant Setup Self Test.lnk`

4. Schedules an interactive uninstall task.
5. Runs the installed uninstaller with:

   ```text
   --headed --automation uninstall <journal path>
   ```

6. Waits for cleanup helper completion.
7. Verifies removed state:

   - install root removed
   - payload removed
   - journal removed
   - installed uninstaller removed
   - desktop shortcut removed
   - application registry key removed
   - Installed Apps uninstall registration removed

## Automation Changes for Uninstall

The uninstall path can spawn a cleanup helper to delete the running uninstaller executable after the main uninstall process exits. The cleanup helper previously could still show a GUI success or reboot prompt.

Fix:

- `uninstall` now receives the automation flag.
- `cleanup` now receives the automation flag.
- `spawn_cleanup_helper` propagates `--automation`.
- When the parent UI mode is GUI, `spawn_cleanup_helper` also passes `--headed`.
- GUI success/reboot prompts are skipped in automation mode.

This keeps the C# progress UI visible while preventing blocking prompts during automated tests.

## Abort Collector

Added:

```text
scripts/windows-vm/Abort-SmokeDiagnostics.ps1
```

Purpose:

- Write an explicit `abort_requested` event.
- Capture processes, visible windows, scheduled tasks, task info, and recent event logs.
- Stop installer, uninstaller, C# UI, and smoke scheduled tasks.
- Unregister smoke scheduled tasks.
- Zip and return the trace bundle as base64.

This is useful when the host-side test command is interrupted and the normal `finally` block does not complete.

## Final Verified Result

The final smoke test command was:

```powershell
.\scripts\run-windows-vm-smoke.ps1 -SkipViewer -HaltAfter
```

It passed with:

```json
{
  "success": true,
  "exitCode": 0,
  "installExitCode": 0,
  "uninstallExitCode": 0,
  "uninstallVerified": true
}
```

The final trace showed both scheduled tasks completing:

- `CovenantSetupSelfInstall-Install-...`
- `CovenantSetupSelfInstall-Uninstall-...`

It also showed:

- install Rust heartbeat
- install C# pipe log
- uninstall Rust heartbeat
- uninstall C# pipe log
- cleanup helper heartbeat
- `uninstall_cleanup_observed` with every checked path/key absent

## Relevant Code Changes

### Build and Packaging

- `.cargo/config.toml`
  - Enables static MSVC runtime linking for the Rust executable.
- `build.rs`
  - Publishes the C# WinForms UI as self-contained `win-x64`.
  - Sets `COVENANT_SETUP_UI_EXE` for Rust embedding.
- `src/main.rs`
  - Adds trace events.
  - Uses raw embedded bundle format.
  - Uses C# UI IPC instead of PowerShell UI.
  - Writes packaged install journal to the inferred install root.
  - Propagates automation through uninstall cleanup.

### C# UI

- `src/ui.rs`
  - Extracts embedded C# UI executable.
  - Connects to a named pipe.
  - Sends JSON UI messages.
  - Logs Rust-side pipe events.
- `ui/Covenant.Setup.Ui/Program.cs`
  - Hosts the named pipe server.
  - Displays progress, logs, and prompts.
  - Logs C# pipe events to `csharp-ui-pipe-<pid>.jsonl`.

### Vagrant Harness

- `scripts/run-windows-vm-smoke.ps1`
  - Requires `dotnet`.
  - Packages with `Start-Process -Wait`.
  - Validates embedded bundle marker.
  - Uploads guest scripts.
  - Pulls trace bundle in `finally`.
  - Reports install and uninstall status.
- `scripts/windows-vm/Start-InteractiveSelfInstall.ps1`
  - Runs install and uninstall as separate interactive scheduled tasks.
  - Verifies installed state before uninstall.
  - Verifies removed state after uninstall.
  - Writes detailed guest trace events.
- `scripts/windows-vm/Invoke-InteractiveInstaller.ps1`
  - Starts a target executable with decoded argument list.
  - Polls process state instead of relying only on `WaitForExit`.
  - Writes per-operation diagnostics.
- `scripts/windows-vm/Abort-SmokeDiagnostics.ps1`
  - Captures and aborts an in-progress smoke run.

### Removed PowerShell UI

- `scripts/windows-vm/Approve-InstallerDialogs.ps1`
  - Removed because the automation path no longer clicks PowerShell UI dialogs.
- `src/win.rs`
  - PowerShell/TaskDialog UI helpers were removed from the primary UI flow.

## Troubleshooting Guide for the Next Hang

1. Check whether the host command is still running:

   ```powershell
   Get-Process | Where-Object { $_.ProcessName -match 'vagrant|ruby|covenant' }
   ```

2. If the host-side Vagrant process is stuck and the run should be aborted, stop only the Vagrant/Ruby processes for that run.

3. Pull guest diagnostics:

   ```powershell
   vagrant upload scripts\windows-vm\Abort-SmokeDiagnostics.ps1 C:\Users\vagrant\AppData\Local\Temp\covenant-setup-smoke\scripts\Abort-SmokeDiagnostics.ps1
   vagrant winrm -s powershell -c "& 'C:\Users\vagrant\AppData\Local\Temp\covenant-setup-smoke\scripts\Abort-SmokeDiagnostics.ps1'"
   ```

4. Inspect `dist\vagrant-self-test\trace\guest-events.jsonl`.

5. Interpret missing logs:

   - No `interactive_installer_start`: scheduled task or PowerShell wrapper failed before launching the installer.
   - `interactive_installer_start` exists, but no `installer-heartbeat-*.jsonl`: executable failed before Rust `main()`, usually loader/dependency/signing/OS error.
   - Rust heartbeat exists, but no `csharp-ui-pipe-*.jsonl`: C# UI failed to start or pipe connection failed.
   - Both heartbeat and pipe logs exist: inspect the last Rust phase and last C# pipe phase to find the blocked operation.

6. Check `interactive-windows.json` for modal system dialogs.

7. Check `abort-processes.json` and `abort-scheduled-task-info.json` for orphaned tasks or running installers.

