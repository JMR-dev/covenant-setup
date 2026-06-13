use crate::sys::Sys;
use crate::*;
use serde_json::json;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(clippy::too_many_arguments)]
pub(crate) fn uninstall(
    journal_path: &Path,
    elevate: bool,
    ui_mode: UiMode,
    automation: bool,
    sys: &dyn Sys,
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    logger: &Logger,
    is_rollback: bool,
) -> Result<(), AppError> {
    let journal: Journal = serde_json::from_str(&fs::read_to_string(journal_path)?)?;
    trace_event(
        if is_rollback {
            "rollback_start"
        } else {
            "uninstall_start"
        },
        json!({"journal": journal_path, "app_name": &journal.app_name}),
    );
    let app_name = journal.app_name.clone();
    let action_verb = if is_rollback {
        "Reverting"
    } else {
        "Uninstalling"
    };
    let _progress = start_tui_progress(ui_mode, format!("{} {} ", action_verb, journal.app_name));
    let uninstall_total = total_uninstall_steps(&journal);
    // A rollback reuses the live install progress session passed in by the
    // caller; only a standalone uninstall starts its own.
    if gui_progress.is_none() {
        *gui_progress = start_gui_progress(
            ui_mode,
            sys,
            &format!("{} {}", action_verb, journal.app_name),
            if is_rollback {
                Some(&journal.app_name)
            } else {
                None
            },
            None,
            None,
            false,
            uninstall_total,
            logger,
        )?;
    }
    let result = (|| -> Result<(), AppError> {
        let effective_logger = if ui_mode == UiMode::Tui {
            logger.quiet_clone()
        } else {
            logger.clone()
        };
        let resolver = win::PathResolver::new(&effective_logger)?;
        let requires_admin = journal_requires_admin(&journal, &resolver)?;
        ensure_elevation_if_needed(requires_admin, elevate, sys, &effective_logger)?;
        trace_event(
            "uninstall_elevation_checked",
            json!({"requires_admin": requires_admin, "elevate": elevate}),
        );
        let current_exe = std::env::current_exe().ok();
        let mut deferred_self_delete: Option<PathBuf> = None;
        let mut deferred_uninstall_registry: Vec<(RegistryRoot, String)> = Vec::new();
        let mut progress_step = 0usize;

        for action in journal.actions.iter().rev() {
            match action {
                JournalAction::CreateDirectory { path } => {
                    advance_gui_progress_step(
                        gui_progress,
                        &mut progress_step,
                        uninstall_total,
                        &format!("Removing directory {}", path.display()),
                    )?;
                    if journal_path.starts_with(path) && journal_path.exists() {
                        let _ = fs::remove_file(journal_path);
                    }
                    win::remove_directory_if_exists(path, &effective_logger)?
                }
                JournalAction::CopyFile { destination, .. } => {
                    if current_exe
                        .as_ref()
                        .is_some_and(|exe| same_path(exe, destination))
                    {
                        effective_logger.info("defer_self_delete", json!({"path":destination}));
                        deferred_self_delete = Some(destination.clone());
                    } else {
                        advance_gui_progress_step(
                            gui_progress,
                            &mut progress_step,
                            uninstall_total,
                            &format!("Removing file {}", destination.display()),
                        )?;
                        sys.remove_file_with_fallback(destination, &effective_logger)?
                    }
                }
                JournalAction::WriteRegistry { root, subkey, .. } => {
                    if is_uninstall_registry_key(subkey) {
                        push_unique_registry_branch(
                            &mut deferred_uninstall_registry,
                            *root,
                            subkey.clone(),
                        );
                    } else {
                        advance_gui_progress_step(
                            gui_progress,
                            &mut progress_step,
                            uninstall_total,
                            &format!("Removing registry branch {}", subkey),
                        )?;
                        sys.delete_registry_tree(*root, subkey, &effective_logger)?
                    }
                }
                JournalAction::CreateShortcut { path } => {
                    advance_gui_progress_step(
                        gui_progress,
                        &mut progress_step,
                        uninstall_total,
                        &format!("Removing shortcut {}", path.display()),
                    )?;
                    sys.remove_file_with_fallback(path, &effective_logger)?
                }
                JournalAction::ExecuteScript { .. } => {
                    effective_logger.info("skip_script_rollback", json!({}))
                }
            }
        }

        for branch in &journal.purge.registry_branches {
            let (root, subkey) = parse_registry_key(branch)?;
            advance_gui_progress_step(
                gui_progress,
                &mut progress_step,
                uninstall_total,
                &format!("Purging registry branch {}", branch),
            )?;
            sys.delete_registry_tree(root, &subkey, &effective_logger)?;
        }
        for path in &journal.purge.paths {
            let resolved = resolver.resolve(path);
            advance_gui_progress_step(
                gui_progress,
                &mut progress_step,
                uninstall_total,
                &format!("Purging path {}", resolved.display()),
            )?;
            purge_path(&resolved, sys, &effective_logger)?;
        }

        for (root, subkey) in deferred_uninstall_registry {
            advance_gui_progress_step(
                gui_progress,
                &mut progress_step,
                uninstall_total,
                &format!("Removing uninstall registration {}", subkey),
            )?;
            sys.delete_registry_tree(root, &subkey, &effective_logger)?;
        }

        if let Some(path) = deferred_self_delete {
            if !is_rollback {
                // Never wait for the window here: the cleanup helper must be
                // spawned promptly so self-deletion is not deferred on the user.
                finish_gui_progress(
                    gui_progress,
                    &format!("Finalizing removal of {}", journal.app_name),
                    false,
                )?;
            }
            sys.spawn_cleanup_helper(
                &path,
                path.parent(),
                &journal.app_name,
                ui_mode,
                automation,
                logger.json,
                &effective_logger,
            )?;
        } else if !is_rollback {
            // During a rollback the install error handler owns the single
            // terminal message on the shared progress session.
            finish_gui_progress(
                gui_progress,
                &format!("{} uninstalled successfully!", journal.app_name),
                false,
            )?;
            if ui_mode == UiMode::Gui && !automation {
                sys.ui_report_uninstall_success(&journal.app_name)?;
            }
        }

        effective_logger.result("ok", json!({"journal":journal_path}));
        let _ = fs::remove_file(journal_path);
        trace_event(
            if is_rollback {
                "rollback_ok"
            } else {
                "uninstall_ok"
            },
            json!({"journal": journal_path}),
        );
        Ok(())
    })();

    if let Err(err) = &result {
        trace_event(
            if is_rollback {
                "rollback_error"
            } else {
                "uninstall_error"
            },
            json!({"app_name": app_name, "error": err.to_string()}),
        );
        // Rollback errors bubble up so the install error handler can compose
        // the single combined failure message on the shared session.
        if !is_rollback {
            let _ = fail_gui_progress(
                gui_progress,
                &app_name,
                "uninstall",
                err,
                journal.support_contact.as_deref(),
                !automation,
            );
        }
    }

    result
}

pub(crate) fn cleanup(
    target_exe: PathBuf,
    install_root: Option<PathBuf>,
    app_name: String,
    ui_mode: UiMode,
    automation: bool,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<(), AppError> {
    trace_event(
        "cleanup_start",
        json!({"target_exe": &target_exe, "install_root": &install_root, "app_name": &app_name}),
    );
    let effective_logger = if ui_mode == UiMode::Tui {
        logger.quiet_clone()
    } else {
        logger.clone()
    };
    let mut reboot_required = false;
    for _ in 0..50 {
        if !target_exe.exists() {
            break;
        }
        if fs::remove_file(&target_exe).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if target_exe.exists() {
        sys.remove_file_with_fallback(&target_exe, &effective_logger)?;
        reboot_required = target_exe.exists();
    }
    if let Some(install_root) = install_root
        && install_root.exists()
        && fs::read_dir(&install_root)?.next().is_none()
    {
        win::remove_directory_if_exists(&install_root, &effective_logger)?;
    }
    reboot_required |= sys.schedule_helper_self_cleanup(&effective_logger)?;
    if ui_mode == UiMode::Gui && !automation {
        if reboot_required {
            if sys.ui_prompt_uninstall_reboot(&app_name)? {
                sys.spawn_reboot(&effective_logger)?;
            }
        } else {
            sys.ui_report_uninstall_success(&app_name)?;
        }
    } else if ui_mode == UiMode::Tui {
        if reboot_required {
            println!(
                "{app_name} uninstalled successfully! Some files from the program still remain on your computer. To complete removal of these files, restart your computer now."
            );
            if sys.prompt_reboot_tui()? {
                sys.spawn_reboot(&effective_logger)?;
            }
        } else {
            println!("{app_name} uninstalled successfully!");
        }
    }
    Ok(())
}

pub(crate) fn spawn_cleanup_helper(
    target_exe: &Path,
    install_root: Option<&Path>,
    app_name: &str,
    ui_mode: UiMode,
    automation: bool,
    json: bool,
    logger: &Logger,
) -> Result<(), AppError> {
    let current_exe = std::env::current_exe()?;
    let helper_path = std::env::temp_dir().join(format!(
        "covenant-setup-cleanup-{}-{}.exe",
        process::id(),
        unique_ticks()
    ));
    logger.info(
        "spawn_cleanup_helper",
        json!({"helper":helper_path,"target_exe":target_exe,"install_root":install_root}),
    );
    fs::copy(&current_exe, &helper_path)?;

    let mut command = Command::new(&helper_path);
    command.creation_flags(CREATE_NO_WINDOW);
    if json {
        command.arg("--json");
    }
    if ui_mode == UiMode::Tui {
        command.arg("--headless");
    } else if ui_mode == UiMode::Gui {
        command.arg("--headed");
    }
    if automation {
        command.arg("--automation");
    }
    command.arg("cleanup");
    command.arg("--target-exe");
    command.arg(target_exe);
    if let Some(install_root) = install_root {
        command.arg("--install-root");
        command.arg(install_root);
    }
    command.arg("--app-name");
    command.arg(app_name);
    command.spawn()?;
    Ok(())
}

pub(crate) fn total_uninstall_steps(journal: &Journal) -> usize {
    journal.actions.len() + journal.purge.registry_branches.len() + journal.purge.paths.len() + 2
}

pub(crate) fn schedule_helper_self_cleanup(logger: &Logger) -> Result<bool, AppError> {
    let self_exe = std::env::current_exe()?;
    logger.info("schedule_helper_self_cleanup", json!({"path":self_exe}));
    let delete_command = format!(
        "Start-Sleep -Seconds 2; Remove-Item -LiteralPath '{}' -Force -ErrorAction SilentlyContinue",
        powershell_single_quote(&self_exe.to_string_lossy())
    );
    let mut command = Command::new("powershell.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    command.arg("-NoProfile");
    command.arg("-WindowStyle");
    command.arg("Hidden");
    command.arg("-Command");
    command.arg(OsString::from(delete_command));
    if command.spawn().is_ok() {
        return Ok(false);
    }
    win::remove_file_with_fallback(&self_exe, logger)?;
    Ok(true)
}

pub(crate) fn journal_requires_admin(
    journal: &Journal,
    resolver: &win::PathResolver,
) -> Result<bool, AppError> {
    for action in &journal.actions {
        match action {
            JournalAction::CreateDirectory { path }
            | JournalAction::CopyFile {
                destination: path, ..
            }
            | JournalAction::CreateShortcut { path } => {
                if resolver.requires_admin(path) {
                    return Ok(true);
                }
            }
            JournalAction::WriteRegistry {
                root: RegistryRoot::Hklm,
                ..
            } => {
                return Ok(true);
            }
            _ => {}
        }
    }
    for branch in &journal.purge.registry_branches {
        let (root, _) = parse_registry_key(branch)?;
        if matches!(root, RegistryRoot::Hklm) {
            return Ok(true);
        }
    }
    for path in &journal.purge.paths {
        if resolver.requires_admin(&resolver.resolve(path)) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn purge_path(path: &Path, sys: &dyn Sys, logger: &Logger) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_file() {
        return sys.remove_file_with_fallback(path, logger);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            purge_path(&child, sys, logger)?;
        } else {
            sys.remove_file_with_fallback(&child, logger)?;
        }
    }
    win::remove_directory_if_exists(path, logger)
}

pub(crate) fn parse_registry_key(input: &str) -> Result<(RegistryRoot, String), AppError> {
    if let Some(rest) = input.strip_prefix("HKCU\\") {
        return Ok((RegistryRoot::Hkcu, rest.to_string()));
    }
    if let Some(rest) = input.strip_prefix("HKLM\\") {
        return Ok((RegistryRoot::Hklm, rest.to_string()));
    }
    Err(AppError::Message(format!(
        "Unsupported registry root: {input}"
    )))
}

pub(crate) fn is_uninstall_registry_key(subkey: &str) -> bool {
    subkey.starts_with("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\")
}

pub(crate) fn push_unique_registry_branch(
    branches: &mut Vec<(RegistryRoot, String)>,
    root: RegistryRoot,
    subkey: String,
) {
    if !branches.iter().any(|(existing_root, existing_subkey)| {
        *existing_root == root && *existing_subkey == subkey
    }) {
        branches.push((root, subkey));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn parse_registry_key_accepts_supported_roots_and_rejects_unknown_roots() {
        assert_eq!(
            parse_registry_key("HKCU\\Software\\Example").unwrap(),
            (RegistryRoot::Hkcu, "Software\\Example".to_string())
        );
        assert_eq!(
            parse_registry_key("HKLM\\Software\\Example").unwrap(),
            (RegistryRoot::Hklm, "Software\\Example".to_string())
        );
        assert!(parse_registry_key("HKCR\\Software\\Example").is_err());
    }

    #[test]
    fn uninstall_empty_journal_succeeds_without_actions() {
        let temp = TestDir::new("uninstall-empty");
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "Empty App".to_string(),
            manifest_path: None,
            actions: Vec::new(),
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &WinSys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
    }

    #[test]
    fn purge_path_removes_nested_temp_tree_and_noops_when_missing() {
        let temp = TestDir::new("purge-path");
        let root = temp.path().join("root");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("payload.txt"), b"payload").unwrap();

        purge_path(&root, &WinSys, &quiet_logger()).unwrap();
        purge_path(&root, &WinSys, &quiet_logger()).unwrap();

        assert!(!root.exists());
    }

    #[test]
    fn purge_path_removes_single_file() {
        let temp = TestDir::new("purge-file");
        let file = temp.path().join("payload.txt");
        fs::write(&file, b"payload").unwrap();

        purge_path(&file, &WinSys, &quiet_logger()).unwrap();

        assert!(!file.exists());
    }

    #[test]
    fn uninstall_registry_key_detection_matches_only_uninstall_branch() {
        assert!(is_uninstall_registry_key(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sample"
        ));
        assert!(!is_uninstall_registry_key("Software\\Sample"));
    }

    #[test]
    fn push_unique_registry_branch_deduplicates_root_and_subkey() {
        let mut branches = Vec::new();
        push_unique_registry_branch(
            &mut branches,
            RegistryRoot::Hkcu,
            "Software\\App".to_string(),
        );
        push_unique_registry_branch(
            &mut branches,
            RegistryRoot::Hkcu,
            "Software\\App".to_string(),
        );
        push_unique_registry_branch(
            &mut branches,
            RegistryRoot::Hklm,
            "Software\\App".to_string(),
        );

        assert_eq!(branches.len(), 2);
    }

    #[test]
    fn cleanup_prompts_and_spawns_reboot_when_required_in_gui_mode() {
        let temp = TestDir::new("cleanup-gui-reboot");
        let target_exe = temp.path().join("ghost.exe");
        // Don't create the file — cleanup() short-circuits on !exists().
        let sys = MockSys::new();
        sys.set_schedule_helper_self_cleanup(true);
        sys.set_ui_prompt_uninstall_reboot(true);
        cleanup(
            target_exe,
            None,
            "Sample".to_string(),
            UiMode::Gui,
            false,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let calls = sys.recorded();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SysCall::ScheduleHelperSelfCleanup))
        );
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SysCall::UiPromptUninstallReboot(name) if name == "Sample"))
        );
        assert!(calls.iter().any(|c| matches!(c, SysCall::SpawnReboot)));
    }

    // (e)
    #[test]
    fn cleanup_skips_reboot_when_user_declines() {
        let temp = TestDir::new("cleanup-decline");
        let target_exe = temp.path().join("ghost.exe");
        let sys = MockSys::new();
        sys.set_schedule_helper_self_cleanup(true);
        sys.set_ui_prompt_uninstall_reboot(false);
        cleanup(
            target_exe,
            None,
            "Sample".to_string(),
            UiMode::Gui,
            false,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let calls = sys.recorded();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SysCall::UiPromptUninstallReboot(_)))
        );
        assert!(!calls.iter().any(|c| matches!(c, SysCall::SpawnReboot)));
    }

    // (f)
    #[test]
    fn cleanup_tui_path_skips_prompt_when_no_reboot_needed() {
        let temp = TestDir::new("cleanup-tui-no-reboot");
        let target_exe = temp.path().join("ghost.exe");
        let sys = MockSys::new();
        sys.set_schedule_helper_self_cleanup(false);
        cleanup(
            target_exe,
            None,
            "Sample".to_string(),
            UiMode::Tui,
            false,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let calls = sys.recorded();
        assert!(
            !calls
                .iter()
                .any(|c| matches!(c, SysCall::UiPromptUninstallReboot(_)))
        );
        assert!(!calls.iter().any(|c| matches!(c, SysCall::PromptRebootTui)));
        assert!(!calls.iter().any(|c| matches!(c, SysCall::SpawnReboot)));
    }

    // (g)
    #[test]
    fn uninstall_cancel_mid_run_keeps_journal_and_reports_uninstall_failure() {
        let temp = TestDir::new("uninstall-cancel");
        let copied = temp.path().join("copied.bin");
        fs::write(&copied, b"x").unwrap();
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "CancelApp".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CopyFile {
                source: temp.path().join("source.bin"),
                destination: copied.clone(),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

        let handle = Arc::new(Mutex::new(Vec::new()));
        let sink = MockProgressSink {
            calls: handle.clone(),
            cancel_after_advances: Some(0),
        };
        let mut progress: Option<Box<dyn ProgressSink>> = Some(Box::new(sink));
        let sys = MockSys::new();
        sys.set_is_elevated(true);

        let err = uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut progress,
            &quiet_logger(),
            false,
        )
        .unwrap_err();

        assert!(matches!(err, AppError::Cancelled));
        assert!(
            journal_path.exists(),
            "journal must survive a cancelled uninstall"
        );
        assert!(copied.exists(), "no actions ran before the cancel tripped");
        let calls = handle.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SinkCall::Fail { operation, .. } if operation == "uninstall"))
        );
    }

    // (j)
    #[test]
    fn uninstall_calls_delete_registry_tree_for_recorded_actions_and_purge() {
        let temp = TestDir::new("uninstall-deltree");
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "RegApp".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::WriteRegistry {
                root: RegistryRoot::Hkcu,
                subkey: "Software\\RegApp".to_string(),
                name: "Alpha".to_string(),
            }],
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\Purged".to_string()],
                paths: vec![],
            },
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
        let trees: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::DeleteRegistryTree { subkey, .. } => Some(subkey),
                _ => None,
            })
            .collect();
        assert!(trees.iter().any(|s| s == "Software\\RegApp"));
        assert!(trees.iter().any(|s| s == "Software\\Purged"));
    }

    // (l)
    #[test]
    fn uninstall_calls_remove_file_with_fallback_for_copy_actions_and_shortcuts() {
        let temp = TestDir::new("uninstall-rmfile");
        let copied = temp.path().join("copied.bin");
        let shortcut = temp.path().join("Shortcut.lnk");
        fs::write(&copied, b"x").unwrap();
        fs::write(&shortcut, b"x").unwrap();
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "FileApp".to_string(),
            manifest_path: None,
            actions: vec![
                JournalAction::CopyFile {
                    source: temp.path().join("source.bin"),
                    destination: copied.clone(),
                },
                JournalAction::CreateShortcut {
                    path: shortcut.clone(),
                },
            ],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
        let removed: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::RemoveFileWithFallback(p) => Some(p),
                _ => None,
            })
            .collect();
        assert!(removed.iter().any(|p| p == &copied));
        assert!(removed.iter().any(|p| p == &shortcut));
    }

    // (m): the self-delete branch is triggered when a CopyFile destination
    // matches the current exe — uninstall() then calls spawn_cleanup_helper.
    #[test]
    fn uninstall_defers_self_delete_to_spawn_cleanup_helper() {
        let temp = TestDir::new("uninstall-self");
        let current_exe = std::env::current_exe().unwrap();
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "SelfApp".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CopyFile {
                source: temp.path().join("source.exe"),
                destination: current_exe.clone(),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
        let helper = sys
            .recorded()
            .into_iter()
            .find(|c| matches!(c, SysCall::SpawnCleanupHelper { .. }))
            .expect("expected SpawnCleanupHelper recorded");
        match helper {
            SysCall::SpawnCleanupHelper {
                target_exe,
                app_name,
                ..
            } => {
                assert!(same_path(&target_exe, &current_exe));
                assert_eq!(app_name, "SelfApp");
            }
            _ => unreachable!(),
        }
    }

    // (n)
}
