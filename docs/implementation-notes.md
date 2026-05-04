# Code Review: feat-mvp

  ## Overview

A Windows installer engine that: parses a TOML manifest → executes mutations via Win32 → journals each action → reverses on uninstall. Three CLI verbs (package, install, uninstall) plus a hidden cleanup. Adds a single-file packager that appends payload+index+magic-footer onto the EXE, an out-of-process WinForms GUI (C# binary embedded at build time, talks to Rust over a named pipe, JSON-per-line), a TUI spinner mode, and a --json IPC mode. Tracking goes through the MutationTracker trait (DeclaredTracker is the only impl, matching the MVP spec).

  ## What's Solid

  - Adherence to the MVP spec: W APIs everywhere, KEY_WOW64_64KEY set on every RegCreateKeyExW, SHGetKnownFolderPath for {ProgramFilesX64} / {LocalAppData} / {Desktop}, Restart Manager (RmStartSession/RmGetList) + MoveFileEx(MOVEFILE_DELAY_UNTIL_REBOOT) fallback for locked files, runas elevation via ShellExecuteW, exit code 33 for elevation-required.
  - Glass-box logging: every unsafe block is bracketed by unsafe_enter/unsafe_exit (src/win.rs), and trace_event writes JSONL heartbeat for debugging. This is the most distinctive strength of the code.
  - Module discipline: src/win.rs owns 100% of FFI; no unsafe leaks into main.rs. Utf16Arg correctly null-terminates and exposes as_bytes() with terminator (right for REG_SZ).
  - Self-deletion strategy: spawn helper EXE → original exits → helper deletes target + schedules its own cleanup via PowerShell + MoveFileEx reboot fallback. Sound design.
  - Bundle format (src/main.rs:577–687): payload + length-prefixed JSON index + payload-len + magic footer is a clean append-only design that survives any leading
  binary signing layout.

  ## Correctness Issues

  - [x] path_requires_admin (src/main.rs:1844) hardcodes c:\\program files / c:\\windows. This contradicts the MVP requirement "Hardcoded paths are forbidden" and the convention in CLAUDE.md. Compare against FOLDERID_ProgramFiles* / FOLDERID_Windows from PathResolver. Will misdetect on a non-C: Windows install. Resolved: admin checks now route through PathResolver roots.
  - [x] relaunch_as_admin (src/win.rs:162): std::env::args().skip(1).collect().join(" ") does not Windows-quote arguments. A manifest path with spaces ("C:\Users\Alice's Apps\install.toml") survives as separate tokens after runas. Use CommandLineToArgvW-compatible quoting. Resolved: args are quoted with CommandLineToArgvW-compatible rules.
  - [x] select_ui defaults are inverted (src/main.rs:1474): when stdout is a terminal but parent isn't PowerShell, returns UiMode::None (silent install with no progress); when stdout is not a terminal (piped/redirected), returns UiMode::Gui. So installer install foo.toml | tee log.txt pops a GUI. Default for terminals should be TUI. Resolved differently: UI mode is now explicit; --json suppresses UI and --headed falls back to headless if GUI is unavailable.
  - [x] is_bundled_runtime_invocation (src/main.rs:1460) scans all args for package|install|uninstall|cleanup. If any value (e.g. a path, hidden value, future
  positional) ever equals one of these strings, routing breaks. Inspect only the first non-flag positional. Resolved: the pre-clap routing helper was removed; clap now parses optional subcommands and bundled mode is selected only when no subcommand is present and an embedded bundle exists.
  - [x] fail_gui_progress vs finish_gui_progress (src/main.rs:1547,1558) are identical — both call progress.finish(message). There's no fail message type to the C# side,
   so a failed install gets a "completed" UX. Either add a "fail" message variant or red-state the C# form on a known sentinel. Resolved: GUI progress now has a fail IPC message, persistent failure UX, and errata export.
  - [x] install_uninstaller records seven separate WriteRegistry actions for the ARP key (src/main.rs:962), but uninstall short-circuits to delete_registry_tree on first match (src/main.rs:1086). Functionally fine; the other six are dead journal entries. Either record one branch action or deduplicate during rollback. Resolved: uninstall defers each uninstall-registry branch only once.
  - [x] remove_directory_if_exists (src/win.rs:228) silently swallows ERROR_DIR_NOT_EMPTY and returns Ok without surfacing it to the journal/UI. Worth at least a warn-level event so users know residue exists. Resolved: not-empty directories emit a `remove_directory_deferred` event with reason `not_empty`.
  - [x] same_path (src/main.rs:1834) is a lowercased string compare. Doesn't handle \\?\ prefixes, 8.3 names, or junctions. Use dunce::canonicalize /
  std::fs::canonicalize with a string-fallback for missing paths. Resolved: same_path canonicalizes both sides when possible and falls back to normalized string comparison with verbatim-prefix handling.
  - [x] collect_bundle_files_recursive (src/main.rs:548) has no exclude list. Re-running package from a directory that was previously installed-from will pick up
  journal.json (and any temp scratch) into the new bundle. Resolved: bundle collection skips known generated artifacts and the current output installer path.
  - [x] run_bundled_installer (src/main.rs:721) has a single-variant enum RuntimeMode::Bundled; match arm is dead branching. Either drop the enum or commit to
  multi-mode. Resolved: RuntimeMode was removed.
  - [x] Typo replicated: "uninstalled sucessfully" (missing 's') in src/main.rs:1235 and src/ui.rs:111. Resolved.

  ## Architecture / Style

  - [ ] main.rs is 1856 lines mixing CLI, manifest types, journal types, install/uninstall logic, bundle (de)serialization, IPC plumbing, and a dozen helpers. Split into manifest.rs, journal.rs, bundle.rs, install.rs, uninstall.rs, cli.rs. The ui.rs / win.rs split is good — extend that pattern.
  - [x] Manual arg parsing in parse_ui_preferences (src/main.rs:1433) duplicates clap. The bundled-runtime detection happens before clap parses, which is why this exists, but the duplication of the subcommand keyword list is brittle. Consider running Cli::try_parse_from in detect-only mode first, or feed clap a pre-stripped args vector. Resolved: parse_ui_preferences was removed and clap parses the optional subcommand path directly.
  - [x] start_gui_progress ignores its app_name parameter (src/main.rs:1505); &format!("{title}") is a no-op clone. Remove the dead arg. Resolved.
  - [x] UiPhase enum is effectively unused in select_ui — both arms return UiMode::None. Resolved: UiPhase was removed.
  - [x] Many effective_logger.info("create_directory", json!({"path":path})) blocks are near-clones. A step! macro or per-action helper would shrink the install loop substantially. Resolved partially: repeated progress-step increment/advance plumbing now goes through `advance_gui_progress_step`.

  ## Tests

  - [x] Zero automated tests (CLAUDE.md confirms). The smoke is end-to-end on a Vagrant Windows VM, which is good for integration but doesn't catch regressions cheaply. Resolved: unit tests now cover bundle/journal helpers plus Win32 quoting/admin-root matching.
  Easy unit-test wins, all OS-portable:
    - [x] Bundle round-trip (append_embedded_bundle → read_embedded_bundle)
    - [x] Journal serde round-trip
    - [x] parse_registry_key (HKCU, HKLM, error)
    - [x] sanitize_registry_component (empty input, mixed punctuation)
    - [x] same_path / normalize_path_for_compare
    - [x] path_requires_admin (after de-hardcoding)
    - [x] Utf16Arg::as_bytes length math
    - [x] is_bundled_runtime_invocation truth table. Resolved by removing the helper and testing clap parsing for bundled flags without manual preparse.

  ## Security

  Threat model is "developer authors a trusted manifest" — under that assumption, mostly fine. Concrete items:

  - Bundle has no integrity check. Anyone who can write to the EXE can swap the appended payload without breaking Authenticode signing of the original PE. For a shipping installer, hash the embedded bundle into the binary at build time and verify on read.
  - execute_script is by-design arbitrary code execution under the elevation context — document this in the manifest schema. Consider a --no-scripts switch for
  paranoid environments.
  - PowerShell single-quote escape (powershell_single_quote) is correct for single-quoted strings. Good.
  - Registry component sanitizer maps anything outside [A-Za-z0-9_-] to _. Good against subkey-traversal injection.
  - extract_embedded_bundle writes to %TEMP%\covenant-setup\{stem}-{pid} and remove_dir_alls any existing path first — TOCTOU window if a hostile user has write access to that temp tree. Low risk on Windows ACLs, but consider creating with a random suffix.

  ## Performance

  - read_embedded_bundle reads the entire EXE into memory (read_to_end, src/main.rs:617). For an installer with a multi-hundred-MB payload, this doubles peak RSS.
  Seek to len - 32 to read footer, then seek back to payload_offset and stream into the extraction directory.
  - extract_embedded_bundle clones each file's bytes from the in-memory bundle to disk; combined with the above, the payload is held in RAM twice. Streaming
  extraction would fix both.

  Verdict

  For an MVP this is in good shape and tracks the spec closely. The biggest deltas to address before calling it MVP-complete:

  - [x] De-hardcode path_requires_admin (spec violation).
  - [x] Quote args in relaunch_as_admin (real-world breakage on paths-with-spaces).
  - [x] Fix the select_ui default (silent installs from terminals, GUIs from pipes). Resolved by requiring explicit UI mode and honoring --json.
  - [x] Add a handful of unit tests for the pure-Rust helpers — bundle round-trip and journal serde at minimum.
  - [x] Decide whether fail_gui_progress should actually surface failure UX.


### Q: How would you suggest fixing item 1?

**A:** The fix is to resolve the admin-required roots at runtime through PathResolver (same pattern as {ProgramFilesX64} etc.), then test ancestry against those resolved roots.

  Concrete change

1. Add the missing known folders to PathResolver (src/win.rs:39):

```rust
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_ProgramFilesX64,
    FOLDERID_ProgramFilesX86, FOLDERID_Windows, /* … */
};

pub struct PathResolver {
    pub program_files_x64: PathBuf,
    pub program_files_x86: PathBuf,
    pub windows_dir: PathBuf,
    pub local_app_data: PathBuf,
    pub desktop: PathBuf,
    admin_roots: Vec<String>, // pre-lowercased for matching
}

impl PathResolver {
    pub fn new(logger: &Logger) -> Result<Self, AppError> {
        let program_files_x64 = known_folder(&FOLDERID_ProgramFilesX64, logger)?;
        let program_files_x86 = known_folder(&FOLDERID_ProgramFilesX86, logger)?;
        let windows_dir = known_folder(&FOLDERID_Windows, logger)?;
        let admin_roots = [&program_files_x64, &program_files_x86, &windows_dir]
            .iter()
            .map(|p| p.to_string_lossy().to_ascii_lowercase())
            .collect();
        Ok(Self {
            program_files_x64,
            program_files_x86,
            windows_dir,
            local_app_data: known_folder(&FOLDERID_LocalAppData, logger)?,
            desktop: known_folder(&FOLDERID_Desktop, logger)?,
            admin_roots,
        })
    }

    pub fn requires_admin(&self, path: &Path) -> bool {
        let candidate = path.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
        self.admin_roots.iter().any(|root| {
            candidate == *root
                || candidate.starts_with(&format!("{root}\\"))
        })
    }
}
```

The trailing \ check avoids matching C:\Program Files Custom against C:\Program Files.

2. Delete the standalone heuristic (src/main.rs:1844) and route callers through the resolver:

```rust
fn manifest_requires_admin(
    manifest: &InstallManifest,
    resolver: &win::PathResolver,
) -> Result<bool, AppError> {
    let mut paths = Vec::new();
    paths.extend(manifest.directories.iter());
    paths.extend(manifest.files.iter().map(|f| &f.destination));
    paths.extend(manifest.shortcuts.iter().map(|s| &s.path));
    if paths.iter().any(|p| resolver.requires_admin(&resolver.resolve(p))) {
        return Ok(true);
    }
    for key in &manifest.registry {
        if matches!(parse_registry_key(&key.key)?.0, RegistryRoot::Hklm) {
            return Ok(true);
        }
    }
    Ok(false)
}
```

journal_requires_admin collapses similarly — it already takes a resolver, so just call resolver.requires_admin(path) instead of the free function.

#### Why this approach

- Removes the c:\ and \program files literals — the spec violation goes away.
- Works on non-C: Windows installs and on locale-translated path variants (German "Programme", etc., though modern Windows always returns the English form via SHGetKnownFolderPath).
- Catches Program Files (x86) explicitly via FOLDERID_ProgramFilesX86, which the current code only matched accidentally via the program files prefix.
- Reuses the resolver that's already constructed in both install and uninstall, so no extra Win32 calls per check.

#### Test it cheaply

Once requires_admin is a method, you can fake roots in a unit test without touching Win32:

```rust
#[test]
fn requires_admin_matches_subpaths_only() {
    let r = PathResolver::with_roots_for_test(vec![
        "C:\\Program Files".into(),
        "D:\\Windows".into(),
    ]);
    assert!(r.requires_admin(Path::new("C:\\Program Files\\App\\bin")));
    assert!(r.requires_admin(Path::new("D:\\Windows\\System32")));
    assert!(!r.requires_admin(Path::new("C:\\Program Files Custom\\App")));
    assert!(!r.requires_admin(Path::new("C:\\Users\\alice")));
}
```

That requires a small test-only constructor (#[cfg(test)] fn with_roots_for_test) but it's the cheapest way to lock in the prefix-boundary behavior — exactly the case the current implementation gets wrong.


### Q: Fix the select_ui default (silent installs from terminals, GUIs from pipes). Explain this.

**A:** What select_ui does today

The function lives at src/main.rs:1474 and decides between UiMode::Tui (in-terminal spinner), UiMode::Gui (out-of-process WinForms), or UiMode::None (no UI at all). The current decision table:

  ┌─────────────────────────────────────────────────────┬────────┐
  │              User input / environment               │ Result │
  ├─────────────────────────────────────────────────────┼────────┤
  │ --headless                                          │ Tui    │
  ├─────────────────────────────────────────────────────┼────────┤
  │ --headed                                            │ Gui    │
  ├─────────────────────────────────────────────────────┼────────┤
  │ stdout is a terminal and parent is PowerShell       │ Tui    │
  ├─────────────────────────────────────────────────────┼────────┤
  │ stdout is not a terminal (pipe/redirect/no console) │ Gui    │
  ├─────────────────────────────────────────────────────┼────────┤
  │ stdout is a terminal but parent isn't PowerShell    │ None   │
  └─────────────────────────────────────────────────────┴────────┘

  Two of those rows produce the wrong UX.

#### Bug 1: silent installs in cmd.exe / Windows Terminal

A user opens cmd.exe (or a Windows Terminal tab hosting cmd, or double-clicks a .bat that runs the installer) and types covenant-setup install foo.toml. They are staring at a console. They expect to see something — a spinner, log lines, anything. The current code goes:

1. --headless / --headed → no, neither set.
2. is_terminal() && is_parent_powershell() → terminal yes, parent is cmd.exe not powershell.exe/pwsh.exe → no.
3. !is_terminal() → no, stdout is a terminal.
4. Falls through to UiPhase::Install => UiMode::None.

Result: silent install. The TUI spinner only fires when the parent process happens to be PowerShell, which discriminates against every other shell — cmd.exe, Git Bash, Cygwin, MSYS2, ConEmu hosts, anything spawned from a launcher, etc.

The PowerShell check (win::is_parent_powershell) was probably added because is_terminal() returns true for the PowerShell ISE / VS Code integrated terminal cases that handle ANSI well. But conflating "is a terminal" with "is a PowerShell terminal" is the wrong gate. Any TTY-attached stdout deserves TUI by default.

#### Bug 2: GUI pops up from pipes and CI logs

A CI script or a developer runs:

```
covenant-setup install foo.toml --json | tee install.log
covenant-setup install foo.toml > install.log 2>&1
```

Stdout is not a terminal (it's a pipe / file). The current rule:

```rust
if !io::stdout().is_terminal() {
    return Ok(UiMode::Gui);
}
```

…spawns the WinForms process. On a CI runner with no interactive desktop session this either fails to render, blocks on a hidden modal, or — on a developer box — pops a window in front of whatever they were doing while their tee happily collects an empty log. Worse, the --json flag is not even consulted in this path — a JSON-mode consumer who explicitly opted into machine-readable output gets a GUI anyway.

The intent was clearly "if there's no console attached, we must be a double-clicked .exe → show a GUI." But is_terminal() == false doesn't mean "no console" — it means "stdout isn't a TTY," which is true for pipes, files, and detached subprocesses just as much as for window-launched processes.

#### What the heuristic should be

The decision tree should disambiguate three different concepts the current code is collapsing:

1. Did the user explicitly ask for a UI mode? → honor --headless / --headed.
2. Did the user explicitly ask for IPC mode? → --json implies UiMode::None.
3. Is stdout being consumed by something? → is_terminal() == false means "redirected"; respect that, don't pop a GUI.
4. Is there a console attached at all? → if there's no console, we're a windowed launch and Gui is correct.

A corrected table:

  ┌───────────────────────────────────────┬────────┐
  │     Condition (checked in order)      │ Result │
  ├───────────────────────────────────────┼────────┤
  │ --json                                │ None   │
  ├───────────────────────────────────────┼────────┤
  │ --headless                            │ Tui    │
  ├───────────────────────────────────────┼────────┤
  │ --headed                              │ Gui    │
  ├───────────────────────────────────────┼────────┤
  │ stdout is a terminal (any shell)      │ Tui    │
  ├───────────────────────────────────────┼────────┤
  │ stdout redirected, console attached   │ None   │
  ├───────────────────────────────────────┼────────┤
  │ no console attached (windowed launch) │ Gui    │
  └───────────────────────────────────────┴────────┘

The "no console attached" check is the missing primitive. On Windows it's GetConsoleWindow() == NULL or, equivalently, querying whether the process inherited a console handle. That replaces the brittle parent-process sniff. is_parent_powershell can be retired entirely — its only legitimate signal (the host is interactive) is already captured by is_terminal().

#### Concrete implementation sketch

Three small pieces:

1. Add --json to UiPreferences (src/main.rs:343) and propagate it through ui_preferences_from_cli and parse_ui_preferences. Right now Logger::json is the only place the flag lands.
2. Add win::has_console() -> bool in src/win.rs calling GetConsoleWindow (cheap, no allocation, single Win32 call). Bracket it with the same unsafe_enter/unsafe_exit pattern used elsewhere.
3. Rewrite select_ui to the table above. The UiPhase enum becomes unused (both arms returned the same value anyway) and can be deleted along with all call sites. is_parent_powershell and its 60-line CreateToolhelp32Snapshot walk in src/win.rs:45 can also go.

Net diff is probably +20 / −80 lines, with a clear behavioral improvement and dead-code reduction.

#### Why it matters beyond cosmetics

- CI correctness: an installer that pops a GUI on a headless agent is a hard-to-diagnose hang or a flaky failure. Getting --json mode to actually mean "no UI ever" is the difference between a usable and unusable IPC contract.
- Spec compliance: project_mvp.md says JSON mode "must suppress" all UI. The current code silently violates this whenever stdout is also redirected — which is exactly the case JSON mode is for.
- User trust in cmd.exe: silently completing a multi-step install with no feedback feels broken even when it succeeds. First-run perception of an installer is dominated by what happens in the first second.


### Feedback: Do piece 1, but then simplify

Just force the user to pass either --headed or --headless, with an error message if not supplied, and a fallback to --headless if the C# binary is either not bundled with the installer or not present on the system to install to.
