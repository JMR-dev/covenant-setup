## Project Overview: The "Glass Box" Core Engine (CLI)
**Objective:** Build a native Windows CLI installation packager in Rust that enforces a deterministic, declarative, and fully reversible state model. 

**Architecture:** A standalone, high-performance Win64 command-line tool. It reads a declarative manifest, performs system mutations via the Win32 API, and journals every action. It is designed to output structured JSON so a GUI wrapper (like C#) or a CI/CD pipeline can orchestrate it in the future.

---

## MVP Requirements & Feature List

### 1. The Rust CLI Interface & IPC Readiness
* **CLI Framework:** Utilize `clap` for robust argument parsing with standard subcommands (e.g., `glassbox install manifest.toml`, `glassbox uninstall journal.json`).
* **Structured Output Protocol:** The engine must accept a `--json` flag. When active, all standard text logs, progress percentages, and error stack traces must be suppressed and replaced with single-line serialized JSON objects emitted to `stdout`.
* **UAC Handling:** The CLI must detect if it has administrative privileges via token inspection. If elevation is required for target paths, it must gracefully exit with a specific error code or auto-relaunch itself using the `runas` verb.

### 2. Execution & State Management
* **Declarative Contract Parsing:** The engine ingests an `install.toml` manifest defining the exact expected system state (directories to create, binaries to move, registry keys to write, shortcuts to build).
* **API Adherence:** All system calls must utilize the `windows` crate, strictly employing UTF-16 Wide (`W`) Win32 functions.
* **Registry Architecture:** Registry operations must explicitly use the `KEY_WOW64_64KEY` flag to bypass 32-bit redirection, ensuring true 64-bit state management.
* **Dynamic Path Resolution:** Hardcoded paths are forbidden. The engine must use `SHGetKnownFolderPath` (Shell32) to resolve standard directories like `ProgramFilesX64`, `LocalAppData`, and `Desktop`.

### 3. Modular Mutation Tracking (Extensibility Architecture)
* **The `MutationTracker` Trait:** Internal state changes must not be written directly to the journal. Instead, they pass through a Trait/Interface. 
* **MVP Implementation:** The initial implementation will be a `DeclaredTracker`. It strictly records the actions the engine performs based on the `install.toml` manifest.
* **Future-Proofing:** This trait design allows an `ObservedTracker` (the ETW Watchdog) to be cleanly injected later to capture out-of-bounds actions performed by sub-processes without changing the core engine logic.
* **Script Execution:** The engine can execute procedural post-install scripts (e.g., PowerShell) via `std::process::Command`, but in the MVP, it will only log the *execution* of the script, not the script's internal mutations.

### 4. Journaling and Uninstallation (Deterministic Rollback)
* **The Transaction Journal:** The engine's applied mutations must be written to a local `journal.json` or `journal.toml` file in the application's root directory upon successful installation.
* **Reverse Execution:** The uninstaller sequence must parse the journal and execute deletion operations in strict reverse chronological order.
* **Locked File Handling:** If a binary is locked by a running process during uninstallation, the engine must leverage the Restart Manager API (`RmStartSession`, `RmGetList`) to identify the locking process, or fallback to `MoveFileEx` with the `MOVEFILE_DELAY_UNTIL_REBOOT` flag.
* **Namespace Purging:** The uninstaller must aggressively delete the entirety of the developer's defined configuration branches (e.g., `HKCU\Software\TargetApp` and `%LOCALAPPDATA%\TargetApp`) to ensure zero shadow residue.

---

## Technical Documentation & Reference Links

These references cover the specific Win32 API boundaries and Rust bindings required for the MVP.

### Rust & Integration Crates
* **`windows` Crate:** The official Microsoft language projection for Win32 APIs. Essential for low-level system access.
    * *Documentation:* [https://microsoft.github.io/windows-docs-rs/](https://microsoft.github.io/windows-docs-rs/)
* **`clap` Crate:** The standard for building robust CLI interfaces in Rust.
    * *Documentation:* [https://docs.rs/clap/latest/clap/](https://docs.rs/clap/latest/clap/)
* **`serde` & `serde_json` Crates:** For parsing the `install.toml` and formatting the IPC `stdout` streams.
    * *Documentation:* [https://serde.rs/](https://serde.rs/)

### Windows System APIs
* **The Windows Registry:** Understanding hives, keys, values, and x64 redirection behavior.
    * *Documentation:* [Structure of the Registry - Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/sysinfo/structure-of-the-registry)
* **Restart Manager API:** Necessary for querying which processes are locking files during uninstallation.
    * *Documentation:* [Restart Manager - Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/rstmgr/restart-manager-portal)
* **Known Folders (Shell32):** Standardizing where application data is written to avoid hardcoded paths.
    * *Documentation:* [KNOWNFOLDERID - Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)
* **File Management (MoveFileEx):** Crucial for handling delayed deletions upon reboot.
    * *Documentation:* [MoveFileExW function - Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)