use crate::sys::Sys;
use crate::*;
use serde_json::json;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub(crate) struct InstallRuntime {
    pub(crate) journal_path: PathBuf,
    pub(crate) install_root: Option<PathBuf>,
    pub(crate) uninstall_exe_path: Option<PathBuf>,
    pub(crate) uninstall_registry_root: RegistryRoot,
    pub(crate) uninstall_registry_key: String,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn install(
    manifest_path: &Path,
    journal_path: Option<PathBuf>,
    elevate: bool,
    ui_mode: UiMode,
    automation: bool,
    sys: &dyn Sys,
    progress_override: Option<Box<dyn ProgressSink>>,
    logger: &Logger,
) -> Result<(), AppError> {
    let manifest = read_install_manifest(manifest_path)?;
    trace_event(
        "install_start",
        json!({"manifest": manifest_path, "app_name": &manifest.app_name}),
    );
    let app_name = manifest.app_name.clone();
    let _progress = start_tui_progress(ui_mode, format!("Installing {} ", manifest.app_name));
    let effective_logger = if ui_mode == UiMode::Tui {
        logger.quiet_clone()
    } else {
        logger.clone()
    };
    let resolver = win::PathResolver::new(&effective_logger)?;
    let install_total = total_install_steps(&manifest);
    let mut gui_progress = if progress_override.is_some() {
        progress_override
    } else {
        let install_root = infer_install_root(&manifest, &resolver);
        // Branding is only consumed by the GUI welcome/progress window, so skip
        // the filesystem probe entirely for TUI/JSON/headless/automation runs.
        let branding_image = if ui_mode == UiMode::Gui {
            find_branding_image(manifest_path.parent(), install_root.as_deref())
        } else {
            None
        };
        let show_welcome = !automation && !manifest.app_name.trim().is_empty();
        start_gui_progress(
            ui_mode,
            sys,
            &format!("Installing {}", manifest.app_name),
            Some(&manifest.app_name),
            install_root.as_deref(),
            branding_image.as_deref(),
            show_welcome,
            install_total,
            &effective_logger,
        )?
    };
    let mut tracker = DeclaredTracker::new();
    let mut runtime_opt: Option<InstallRuntime> = None;
    let result = (|| -> Result<(), AppError> {
        let requires_admin = manifest_requires_admin(&manifest, &resolver)?;
        ensure_elevation_if_needed(requires_admin, elevate, sys, &effective_logger)?;
        trace_event(
            "install_elevation_checked",
            json!({"requires_admin": requires_admin, "elevate": elevate}),
        );
        let runtime = build_install_runtime(
            &manifest,
            manifest_path,
            journal_path,
            requires_admin,
            &resolver,
        )?;
        runtime_opt = Some(runtime.clone());

        let mut progress_step = 0usize;
        for directory in &manifest.directories {
            let path = resolver.resolve(directory);
            effective_logger.info("create_directory", json!({"path":path}));
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Creating directory {}", path.display()),
            )?;
            win::create_directory_recursive(&path, &effective_logger)?;
            tracker.record(JournalAction::CreateDirectory { path });
        }

        for file in &manifest.files {
            let source = absolutize(manifest_path.parent(), &file.source);
            let destination = resolver.resolve(&file.destination);
            if let Some(parent) = destination.parent() {
                win::create_directory_recursive(parent, &effective_logger)?;
            }
            effective_logger.info(
                "copy_file",
                json!({"source":source,"destination":destination}),
            );
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Copying file to {}", destination.display()),
            )?;
            win::copy_file(&source, &destination, &effective_logger)?;
            tracker.record(JournalAction::CopyFile {
                source,
                destination,
            });
        }

        for entry in &manifest.registry {
            let (root, subkey) = parse_registry_key(&entry.key)?;
            let resolved_value = resolver.resolve(&entry.value).to_string_lossy().to_string();
            effective_logger.info(
                "write_registry",
                json!({"key":entry.key,"name":entry.name,"value":resolved_value}),
            );
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Writing registry value {} in {}", entry.name, entry.key),
            )?;
            sys.set_registry_string(
                root,
                &subkey,
                &entry.name,
                &resolved_value,
                &effective_logger,
            )?;
            tracker.record(JournalAction::WriteRegistry {
                root,
                subkey,
                name: entry.name.clone(),
            });
        }

        for shortcut in &manifest.shortcuts {
            let path = resolver.resolve(&shortcut.path);
            let target = resolver.resolve(&shortcut.target);
            let working_directory = shortcut
                .working_directory
                .as_deref()
                .map(|v| resolver.resolve(v));
            if let Some(parent) = path.parent() {
                win::create_directory_recursive(parent, &effective_logger)?;
            }
            effective_logger.info("create_shortcut", json!({"path":path,"target":target}));
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Creating shortcut {}", path.display()),
            )?;
            win::create_shortcut(
                &path,
                &target,
                shortcut.arguments.as_deref(),
                working_directory.as_deref(),
                shortcut.description.as_deref(),
                &effective_logger,
            )?;
            tracker.record(JournalAction::CreateShortcut { path });
        }

        for script in &manifest.scripts {
            let working_directory = script
                .working_directory
                .as_deref()
                .map(|v| resolver.resolve(v));
            effective_logger.info("execute_script", json!({"command":script.command,"args":script.args,"working_directory":working_directory}));
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Running script {}", script.command),
            )?;
            execute_script(
                script,
                manifest_path.parent(),
                working_directory.as_deref(),
                &mut gui_progress,
            )?;
            tracker.record(JournalAction::ExecuteScript {
                command: script.command.clone(),
                args: script.args.clone(),
                working_directory,
            });
        }

        if let Some(uninstall_exe_path) = &runtime.uninstall_exe_path {
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Installing uninstaller {}", uninstall_exe_path.display()),
            )?;
            install_uninstaller(uninstall_exe_path, &effective_logger)?;
            tracker.record(JournalAction::CopyFile {
                source: std::env::current_exe()?,
                destination: uninstall_exe_path.clone(),
            });
        }

        if let (Some(install_root), Some(uninstall_exe_path)) =
            (&runtime.install_root, &runtime.uninstall_exe_path)
        {
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Registering {} in Installed Apps", manifest.app_name),
            )?;
            register_uninstall_entry(
                &manifest,
                &runtime,
                install_root,
                uninstall_exe_path,
                sys,
                &effective_logger,
            )?;
            for value_name in [
                "DisplayName",
                "Publisher",
                "DisplayVersion",
                "InstallLocation",
                "DisplayIcon",
                "UninstallString",
                "QuietUninstallString",
            ] {
                tracker.record(JournalAction::WriteRegistry {
                    root: runtime.uninstall_registry_root,
                    subkey: runtime.uninstall_registry_key.clone(),
                    name: value_name.to_string(),
                });
            }
        }

        Ok(())
    })();

    let journal = tracker.finish(
        manifest.app_name.clone(),
        Some(manifest_path.to_path_buf()),
        manifest.purge.clone(),
        manifest.support_contact.clone(),
    );
    let result = result.and_then(|_| {
        let runtime = runtime_opt
            .as_ref()
            .ok_or_else(|| AppError::Message("Install runtime was not initialized".to_string()))?;
        write_journal(&runtime.journal_path, &journal)?;
        trace_event(
            "install_journal_written",
            json!({"journal": runtime.journal_path, "actions": journal.actions.len()}),
        );
        effective_logger.result(
            "ok",
            json!({"journal":runtime.journal_path,"actions":journal.actions.len()}),
        );
        finish_gui_progress(
            &mut gui_progress,
            &format!("{} installation completed successfully", manifest.app_name),
            !automation,
        )?;
        if ui_mode == UiMode::Tui {
            println!("{} installation completed successfully", manifest.app_name);
        }
        Ok(())
    });

    result.map_err(|err| {
        trace_event(
            "install_error",
            json!({"app_name": app_name, "error": err.to_string()}),
        );
        let cancelled = matches!(err, AppError::Cancelled);
        let mut rollback_error_msg = None;
        if let Some(runtime) = &runtime_opt
            && !journal.actions.is_empty() {
                let write_res = (|| -> Result<(), AppError> {
                    write_journal(&runtime.journal_path, &journal)?;
                    let read_back = fs::read_to_string(&runtime.journal_path)?;
                    let _: Journal = serde_json::from_str(&read_back)?;
                    Ok(())
                })();
                match write_res {
                    Ok(()) => {
                        if cancelled && let Some(progress) = gui_progress.as_mut() {
                            // The flag already tripped the install loop; clear it
                            // so the rollback's own progress is not cancelled too.
                            progress.clear_cancel_request();
                            let _ = progress.log("Cancel requested - reverting changes...");
                        }
                        match uninstall(
                            &runtime.journal_path,
                            elevate,
                            ui_mode,
                            automation,
                            sys,
                            &mut gui_progress,
                            logger,
                            true,
                        ) {
                            Ok(()) => {
                                trace_event(
                                    "rollback_success",
                                    json!({"journal": runtime.journal_path, "actions": journal.actions.len()}),
                                );
                            }
                            Err(u_err) => {
                                rollback_error_msg = Some(format!("Rollback failed: {u_err}"));
                            }
                        }
                    }
                    Err(w_err) => {
                        rollback_error_msg =
                            Some(format!("Could not write or verify journal.json: {w_err}"));
                    }
                }
            }

        if cancelled && rollback_error_msg.is_none() {
            let _ = finish_gui_progress(
                &mut gui_progress,
                &format!("{app_name} installation cancelled. All changes were reverted."),
                !automation,
            );
            return AppError::Cancelled;
        }

        let display_err = if let Some(r_err) = rollback_error_msg {
            AppError::Message(format!(
                "Installation failed: {err}. Additionally, rollback failed: {r_err}"
            ))
        } else {
            err
        };

        let _ = fail_gui_progress(
            &mut gui_progress,
            &app_name,
            "install",
            &display_err,
            manifest.support_contact.as_deref(),
            !automation,
        );
        display_err
    })
}

pub(crate) fn ensure_elevation_if_needed(
    required: bool,
    relaunch: bool,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<(), AppError> {
    if !required || sys.is_elevated(logger)? {
        trace_event(
            "elevation_ok",
            json!({"required": required, "relaunch": relaunch}),
        );
        return Ok(());
    }
    if relaunch {
        trace_event("elevation_relaunch", json!({}));
        sys.relaunch_as_admin(logger)?;
        return Err(AppError::Message("__elevated_relaunch__".into()));
    }
    trace_event("elevation_required_error", json!({}));
    Err(AppError::Message(
        "Elevation required for requested operation".into(),
    ))
}

pub(crate) fn build_install_runtime(
    manifest: &InstallManifest,
    manifest_path: &Path,
    journal_path: Option<PathBuf>,
    requires_admin: bool,
    resolver: &win::PathResolver,
) -> Result<InstallRuntime, AppError> {
    let install_root = infer_install_root(manifest, resolver);
    let journal_path = journal_path.unwrap_or_else(|| {
        install_root
            .clone()
            .unwrap_or_else(|| {
                manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            })
            .join("journal.json")
    });
    let uninstall_exe_path = install_root
        .clone()
        .map(|root| root.join("covenant-setup-uninstall.exe"));
    let uninstall_registry_root = if requires_admin {
        RegistryRoot::Hklm
    } else {
        RegistryRoot::Hkcu
    };
    let uninstall_registry_key = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
        sanitize_registry_component(&manifest.app_name)
    );
    Ok(InstallRuntime {
        journal_path,
        install_root,
        uninstall_exe_path,
        uninstall_registry_root,
        uninstall_registry_key,
    })
}

pub(crate) fn infer_install_root(
    manifest: &InstallManifest,
    resolver: &win::PathResolver,
) -> Option<PathBuf> {
    if let Some(path) = manifest.purge.paths.first() {
        return Some(resolver.resolve(path));
    }
    if let Some(directory) = manifest.directories.first() {
        return Some(resolver.resolve(directory));
    }
    if let Some(file) = manifest.files.first() {
        return resolver
            .resolve(&file.destination)
            .parent()
            .map(Path::to_path_buf);
    }
    None
}

/// Branding ships with the payload, so the manifest directory is probed
/// first; the install root only exists on reinstalls over a previous tree.
pub(crate) fn find_branding_image(
    manifest_dir: Option<&Path>,
    install_root: Option<&Path>,
) -> Option<String> {
    const BRANDING_FILE_NAMES: [&str; 3] = ["branding.png", "banner.png", "logo.png"];
    manifest_dir
        .into_iter()
        .chain(install_root)
        .flat_map(|root| BRANDING_FILE_NAMES.iter().map(move |name| root.join(name)))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

pub(crate) fn install_uninstaller(
    uninstall_exe_path: &Path,
    logger: &Logger,
) -> Result<(), AppError> {
    if let Some(parent) = uninstall_exe_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let current_exe = std::env::current_exe()?;
    logger.info(
        "install_uninstaller",
        json!({"source":current_exe,"destination":uninstall_exe_path}),
    );
    fs::copy(current_exe, uninstall_exe_path)?;
    Ok(())
}

pub(crate) fn register_uninstall_entry(
    manifest: &InstallManifest,
    runtime: &InstallRuntime,
    install_root: &Path,
    uninstall_exe_path: &Path,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<(), AppError> {
    let uninstall_command = format!(
        "\"{}\" uninstall \"{}\" --elevate",
        uninstall_exe_path.display(),
        runtime.journal_path.display()
    );
    let values = [
        ("DisplayName", manifest.app_name.clone()),
        ("Publisher", "covenant-setup".to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        (
            "InstallLocation",
            install_root.to_string_lossy().to_string(),
        ),
        (
            "DisplayIcon",
            uninstall_exe_path.to_string_lossy().to_string(),
        ),
        ("UninstallString", uninstall_command.clone()),
        ("QuietUninstallString", uninstall_command),
    ];
    for (name, value) in values {
        logger.info(
            "register_uninstall_value",
            json!({"key":runtime.uninstall_registry_key,"name":name,"value":value}),
        );
        sys.set_registry_string(
            runtime.uninstall_registry_root,
            &runtime.uninstall_registry_key,
            name,
            &value,
            logger,
        )?;
    }
    Ok(())
}

pub(crate) fn total_install_steps(manifest: &InstallManifest) -> usize {
    manifest.directories.len()
        + manifest.files.len()
        + manifest.registry.len()
        + manifest.shortcuts.len()
        + manifest.scripts.len()
        + 2
}

pub(crate) fn manifest_requires_admin(
    manifest: &InstallManifest,
    resolver: &win::PathResolver,
) -> Result<bool, AppError> {
    for directory in &manifest.directories {
        if resolver.requires_admin(&resolver.resolve(directory)) {
            return Ok(true);
        }
    }
    for file in &manifest.files {
        if resolver.requires_admin(&resolver.resolve(&file.destination)) {
            return Ok(true);
        }
    }
    for shortcut in &manifest.shortcuts {
        if resolver.requires_admin(&resolver.resolve(&shortcut.path)) {
            return Ok(true);
        }
    }
    for key in &manifest.registry {
        let (root, _) = parse_registry_key(&key.key)?;
        if matches!(root, RegistryRoot::Hklm) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn execute_script(
    script: &ScriptSpec,
    manifest_dir: Option<&Path>,
    working_directory: Option<&Path>,
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
) -> Result<(), AppError> {
    trace_event(
        "script_start",
        json!({"command": &script.command, "args": &script.args, "working_directory": working_directory}),
    );
    let command_path = absolutize(manifest_dir, &script.command);
    let command = if command_path.exists() {
        command_path
    } else {
        PathBuf::from(&script.command)
    };
    let mut process = Command::new(command);
    process.creation_flags(CREATE_NO_WINDOW);
    process.args(&script.args);
    if let Some(dir) = working_directory {
        process.current_dir(dir);
    }
    let output = process.output()?;
    append_gui_shell_output(gui_progress, &output.stdout)?;
    append_gui_shell_output(gui_progress, &output.stderr)?;
    let status = output.status;
    trace_event(
        "script_exit",
        json!({"command": &script.command, "status": status.code()}),
    );
    if !status.success() {
        return Err(AppError::Message(format!(
            "Script failed: {} ({status})",
            script.command
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn find_branding_image_prefers_manifest_dir_over_install_root() {
        let temp = TestDir::new("branding-image");
        let manifest_dir = temp.path().join("payload-src");
        let install_root = temp.path().join("install-root");
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::create_dir_all(&install_root).unwrap();

        assert_eq!(
            find_branding_image(Some(&manifest_dir), Some(&install_root)),
            None
        );

        fs::write(install_root.join("logo.png"), b"png").unwrap();
        assert_eq!(
            find_branding_image(Some(&manifest_dir), Some(&install_root)),
            Some(install_root.join("logo.png").to_string_lossy().into_owned())
        );

        fs::write(manifest_dir.join("branding.png"), b"png").unwrap();
        assert_eq!(
            find_branding_image(Some(&manifest_dir), Some(&install_root)),
            Some(
                manifest_dir
                    .join("branding.png")
                    .to_string_lossy()
                    .into_owned()
            )
        );
    }

    #[test]
    fn elevation_not_required_short_circuits_without_admin_probe() {
        ensure_elevation_if_needed(false, false, &WinSys, &quiet_logger()).unwrap();
    }

    #[test]
    fn total_step_helpers_count_manifest_and_journal_work() {
        let manifest = sample_manifest();
        assert_eq!(total_install_steps(&manifest), 7);

        let journal = Journal {
            app_name: "Steps".to_string(),
            manifest_path: None,
            actions: vec![
                JournalAction::CreateDirectory {
                    path: PathBuf::from("C:\\Apps\\Steps"),
                },
                JournalAction::CreateShortcut {
                    path: PathBuf::from("C:\\Users\\Public\\Desktop\\Steps.lnk"),
                },
            ],
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\Steps".to_string()],
                paths: vec!["C:\\Apps\\Steps\\Cache".to_string()],
            },
            support_contact: None,
        };
        assert_eq!(total_uninstall_steps(&journal), 6);
    }

    #[test]
    fn install_empty_manifest_writes_minimal_journal() {
        let temp = TestDir::new("install-empty");
        let manifest_path = temp.path().join("EmptyApp-install.toml");
        fs::write(&manifest_path, "app_name = 'Empty App'\n").unwrap();

        install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true,
            &WinSys,
            None,
            &quiet_logger(),
        )
        .unwrap();

        let journal_path = temp.path().join("journal.json");
        let journal: Journal =
            serde_json::from_str(&fs::read_to_string(journal_path).unwrap()).unwrap();
        assert_eq!(journal.app_name, "Empty App");
        assert_eq!(journal.manifest_path, Some(manifest_path));
        assert!(journal.actions.is_empty());
        assert_eq!(journal.purge, PurgeSpec::default());
    }

    #[test]
    fn execute_script_reports_success_and_failure_status() {
        let mut progress = None;
        let ok = ScriptSpec {
            command: "cmd.exe".to_string(),
            args: vec!["/C".to_string(), "exit 0".to_string()],
            working_directory: None,
        };
        execute_script(&ok, None, None, &mut progress).unwrap();

        let failing = ScriptSpec {
            command: "cmd.exe".to_string(),
            args: vec!["/C".to_string(), "exit 7".to_string()],
            working_directory: None,
        };
        let err = execute_script(&failing, None, None, &mut progress)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Script failed: cmd.exe"));
    }

    #[test]
    fn install_uninstaller_copies_current_exe_to_target() {
        let temp = TestDir::new("install-uninstaller");
        let target = temp.path().join("bin").join("covenant-setup-uninstall.exe");

        install_uninstaller(&target, &quiet_logger()).unwrap();

        assert!(target.is_file());
        assert!(fs::metadata(target).unwrap().len() > 0);
    }

    #[test]
    fn execute_script_uses_manifest_relative_command_and_working_directory() {
        let temp = TestDir::new("script-relative");
        let script = temp.path().join("ok.cmd");
        let working_directory = temp.path().join("wd");
        fs::create_dir_all(&working_directory).unwrap();
        fs::write(&script, "@echo off\r\ncd\r\nexit /B 0\r\n").unwrap();

        let spec = ScriptSpec {
            command: "ok.cmd".to_string(),
            args: Vec::new(),
            working_directory: None,
        };
        let mut progress = None;

        execute_script(
            &spec,
            Some(temp.path()),
            Some(&working_directory),
            &mut progress,
        )
        .unwrap();
    }

    #[test]
    fn install_runtime_uses_inferred_or_explicit_journal_paths() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let manifest = sample_manifest();
        let runtime = build_install_runtime(
            &manifest,
            Path::new("C:\\source\\install.toml"),
            None,
            true,
            &resolver,
        )
        .unwrap();

        assert_eq!(
            runtime.journal_path,
            PathBuf::from("C:\\Apps\\Sample\\journal.json")
        );
        assert_eq!(
            runtime.uninstall_exe_path,
            Some(PathBuf::from(
                "C:\\Apps\\Sample\\covenant-setup-uninstall.exe"
            ))
        );
        assert_eq!(runtime.uninstall_registry_root, RegistryRoot::Hklm);
        assert!(runtime.uninstall_registry_key.ends_with("Sample_App"));

        let explicit = build_install_runtime(
            &manifest,
            Path::new("C:\\source\\install.toml"),
            Some(PathBuf::from("D:\\journal.json")),
            false,
            &resolver,
        )
        .unwrap();
        assert_eq!(explicit.journal_path, PathBuf::from("D:\\journal.json"));
        assert_eq!(explicit.uninstall_registry_root, RegistryRoot::Hkcu);
    }

    #[test]
    fn infer_install_root_prefers_purge_then_directory_then_file_parent() {
        let resolver = win::PathResolver::with_roots_for_test(vec![]);
        let mut manifest = sample_manifest();

        assert_eq!(
            infer_install_root(&manifest, &resolver),
            Some(PathBuf::from("C:\\Apps\\Sample"))
        );

        manifest.purge.paths.clear();
        assert_eq!(
            infer_install_root(&manifest, &resolver),
            Some(PathBuf::from("C:\\Apps\\Sample\\bin"))
        );

        manifest.directories.clear();
        assert_eq!(
            infer_install_root(&manifest, &resolver),
            Some(PathBuf::from("C:\\Apps\\Sample"))
        );

        manifest.files.clear();
        assert_eq!(infer_install_root(&manifest, &resolver), None);
    }

    #[test]
    fn manifest_and_journal_admin_checks_use_resolved_paths_and_hklm() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let mut manifest = sample_manifest();
        manifest.purge.paths = vec!["C:\\Users\\Alice\\App".to_string()];
        manifest.directories = vec!["C:\\Program Files\\Sample".to_string()];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        manifest.directories.clear();
        manifest.files.clear();
        manifest.shortcuts.clear();
        manifest.registry = vec![RegistrySpec {
            key: "HKLM\\Software\\Sample".to_string(),
            name: "Value".to_string(),
            value: "Data".to_string(),
        }];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CopyFile {
                source: PathBuf::from("payload.exe"),
                destination: PathBuf::from("C:\\Program Files\\Sample\\payload.exe"),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: vec![],
            purge: PurgeSpec {
                registry_branches: vec!["HKLM\\Software\\Sample".to_string()],
                paths: vec![],
            },
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());
    }

    #[test]
    fn manifest_and_journal_admin_checks_return_false_for_user_scope_work() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let manifest = InstallManifest {
            app_name: "User App".to_string(),
            directories: vec!["C:\\Users\\Alice\\AppData\\Local\\UserApp".to_string()],
            files: vec![FileSpec {
                source: "app.exe".to_string(),
                destination: "C:\\Users\\Alice\\AppData\\Local\\UserApp\\app.exe".to_string(),
            }],
            registry: vec![RegistrySpec {
                key: "HKCU\\Software\\UserApp".to_string(),
                name: "InstallLocation".to_string(),
                value: "C:\\Users\\Alice\\AppData\\Local\\UserApp".to_string(),
            }],
            shortcuts: vec![ShortcutSpec {
                path: "C:\\Users\\Alice\\Desktop\\UserApp.lnk".to_string(),
                target: "C:\\Users\\Alice\\AppData\\Local\\UserApp\\app.exe".to_string(),
                arguments: None,
                working_directory: None,
                description: None,
            }],
            scripts: Vec::new(),
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\UserApp".to_string()],
                paths: vec!["C:\\Users\\Alice\\AppData\\Local\\UserApp".to_string()],
            },
            support_contact: None,
        };
        assert!(!manifest_requires_admin(&manifest, &resolver).unwrap());

        let journal = Journal {
            app_name: "User App".to_string(),
            manifest_path: None,
            actions: vec![
                JournalAction::CreateDirectory {
                    path: PathBuf::from("C:\\Users\\Alice\\AppData\\Local\\UserApp"),
                },
                JournalAction::WriteRegistry {
                    root: RegistryRoot::Hkcu,
                    subkey: "Software\\UserApp".to_string(),
                    name: "InstallLocation".to_string(),
                },
            ],
            purge: manifest.purge,
            support_contact: None,
        };
        assert!(!journal_requires_admin(&journal, &resolver).unwrap());
    }

    #[test]
    fn admin_checks_detect_file_shortcut_and_purge_paths() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let mut manifest = sample_manifest();
        manifest.directories.clear();
        manifest.registry.clear();
        manifest.files = vec![FileSpec {
            source: "payload.exe".to_string(),
            destination: "C:\\Program Files\\Sample\\payload.exe".to_string(),
        }];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        manifest.files.clear();
        manifest.shortcuts = vec![ShortcutSpec {
            path: "C:\\Program Files\\Sample\\Sample.lnk".to_string(),
            target: "C:\\Users\\Alice\\App\\app.exe".to_string(),
            arguments: None,
            working_directory: None,
            description: None,
        }];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CreateShortcut {
                path: PathBuf::from("C:\\Program Files\\Sample\\Sample.lnk"),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: Vec::new(),
            purge: PurgeSpec {
                registry_branches: Vec::new(),
                paths: vec!["C:\\Program Files\\Sample".to_string()],
            },
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());
    }

    // (a)
    #[test]
    fn ensure_elevation_if_needed_relaunches_when_required_and_relaunch_flag_set() {
        let sys = MockSys::new();
        sys.set_is_elevated(false);
        let err = ensure_elevation_if_needed(true, true, &sys, &quiet_logger()).unwrap_err();
        assert_eq!(err.to_string(), "__elevated_relaunch__");
        let calls = sys.recorded();
        assert!(matches!(calls[0], SysCall::IsElevated));
        assert!(matches!(calls[1], SysCall::RelaunchAsAdmin));
    }

    // (b)
    #[test]
    fn ensure_elevation_if_needed_errors_when_required_and_no_relaunch() {
        let sys = MockSys::new();
        sys.set_is_elevated(false);
        let err = ensure_elevation_if_needed(true, false, &sys, &quiet_logger()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Elevation required"));
        let calls = sys.recorded();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0], SysCall::IsElevated));
    }

    // (c)
    #[test]
    fn ensure_elevation_if_needed_passes_when_already_elevated() {
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        ensure_elevation_if_needed(true, true, &sys, &quiet_logger()).unwrap();
        let calls = sys.recorded();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0], SysCall::IsElevated));
    }

    // (d)
    #[test]
    fn register_uninstall_entry_writes_all_seven_values() {
        let manifest = InstallManifest {
            app_name: "Sample".to_string(),
            directories: Vec::new(),
            files: Vec::new(),
            registry: Vec::new(),
            shortcuts: Vec::new(),
            scripts: Vec::new(),
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        let runtime = InstallRuntime {
            journal_path: PathBuf::from("C:\\fake\\journal.json"),
            install_root: Some(PathBuf::from("C:\\Apps\\Sample")),
            uninstall_exe_path: Some(PathBuf::from("C:\\Apps\\Sample\\uninstall.exe")),
            uninstall_registry_root: RegistryRoot::Hkcu,
            uninstall_registry_key:
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sample".to_string(),
        };
        let install_root = PathBuf::from("C:\\Apps\\Sample");
        let uninstall_exe = PathBuf::from("C:\\Apps\\Sample\\uninstall.exe");
        let sys = MockSys::new();
        register_uninstall_entry(
            &manifest,
            &runtime,
            &install_root,
            &uninstall_exe,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let writes: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::SetRegistryString { name, .. } => Some(name),
                _ => None,
            })
            .collect();
        assert_eq!(
            writes,
            vec![
                "DisplayName",
                "Publisher",
                "DisplayVersion",
                "InstallLocation",
                "DisplayIcon",
                "UninstallString",
                "QuietUninstallString",
            ]
        );
    }

    // (h) — substitute: run() dispatches Install subcommand to install() through Sys.
    #[test]
    fn run_install_subcommand_uses_sys_for_registry_writes() {
        let temp = TestDir::new("run-install-sys");
        let manifest_path = temp.path().join("Mocked-install.toml");
        fs::write(
            &manifest_path,
            "app_name = 'Mocked'\n[[registry]]\nkey = 'HKCU\\\\Software\\\\Mocked'\nname = 'Foo'\nvalue = 'Bar'\n",
        )
        .unwrap();
        let journal_path = temp.path().join("journal.json");
        let cli = Cli {
            json: true,
            headed: false,
            headless: false,
            automation: true,
            elevate: false,
            command: Some(Commands::Install {
                manifest: manifest_path.clone(),
                journal: Some(journal_path),
            }),
        };
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        dispatch(cli, &sys, &quiet_logger()).unwrap();
        let writes: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::SetRegistryString { name, value, .. } => Some((name, value)),
                _ => None,
            })
            .collect();
        assert!(writes.iter().any(|(n, v)| n == "Foo" && v == "Bar"));
    }

    // (i) — substitute: install error path emits ui_report_error via run_bundled_installer
    // is exercised at the install layer: a missing manifest yields AppError::Io and the
    // automation flag suppresses the GUI fail UX. We assert the error propagates.
    #[test]
    fn install_with_missing_manifest_propagates_error_without_success_ui() {
        let sys = MockSys::new();
        let err = install(
            Path::new("C:\\does\\not\\exist\\install.toml"),
            None,
            false,
            UiMode::None,
            true,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Io(_) | AppError::Message(_)));
    }

    #[test]
    fn install_failure_triggers_partial_rollback() {
        let temp = TestDir::new("install-rollback-test");
        let manifest_path = temp.path().join("RollbackApp-install.toml");
        let dir_to_create = temp.path().join("CreatedDir");

        // Manifest specifies creating a directory, then copying a non-existent file (which will fail)
        fs::write(
            &manifest_path,
            format!(
                "app_name = 'RollbackApp'\ndirectories = {{ paths = ['{}'] }}\n[[files]]\nsource = 'nonexistent.txt'\ndestination = '{}\\\\file.txt'\n",
                dir_to_create.display().to_string().replace("\\", "\\\\"),
                dir_to_create.display().to_string().replace("\\", "\\\\")
            ),
        )
        .unwrap();

        let sys = MockSys::new();
        sys.set_is_elevated(true);

        let err = install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true, // automation = true
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();

        // The installation must have failed
        assert!(
            err.to_string().contains("nonexistent.txt")
                || err.to_string().contains("cannot find")
                || err.to_string().contains("system cannot find")
        );

        // The directory created during the partial install must have been rolled back (deleted)
        assert!(!dir_to_create.exists());
    }

    #[test]
    fn install_cancel_triggers_rollback_and_returns_cancelled() {
        let temp = TestDir::new("install-cancel-test");
        let manifest_path = temp.path().join("CancelApp-install.toml");
        let dir_to_create = temp.path().join("CreatedDir");
        let payload = temp.path().join("payload.txt");
        fs::write(&payload, b"data").unwrap();

        fs::write(
            &manifest_path,
            format!(
                "app_name = 'CancelApp'\ndirectories = {{ paths = ['{}'] }}\n[[files]]\nsource = '{}'\ndestination = '{}\\\\file.txt'\n",
                dir_to_create.display().to_string().replace("\\", "\\\\"),
                payload.display().to_string().replace("\\", "\\\\"),
                dir_to_create.display().to_string().replace("\\", "\\\\")
            ),
        )
        .unwrap();

        let sys = MockSys::new();
        sys.set_is_elevated(true);
        let sink_calls = sys.install_progress_sink();
        // Trip the cancel flag once the first mutation step has been reported,
        // as if the user clicked Cancel between steps.
        sys.set_cancel_after_advances(1);

        let err = install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            false,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();

        assert!(matches!(err, AppError::Cancelled));
        assert!(!dir_to_create.exists());

        let calls = sink_calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SinkCall::Log(m) if m.contains("Cancel requested")))
        );
        assert!(calls.iter().any(
            |c| matches!(c, SinkCall::Advance { message, .. } if message.contains("Removing directory"))
        ));
        let finishes: Vec<_> = calls
            .iter()
            .filter_map(|c| match c {
                SinkCall::Finish {
                    message,
                    wait_for_close,
                } => Some((message.clone(), *wait_for_close)),
                _ => None,
            })
            .collect();
        assert_eq!(finishes.len(), 1);
        assert!(finishes[0].0.contains("cancelled"));
        assert!(finishes[0].1, "non-automation finish must wait for close");
        assert!(!calls.iter().any(|c| matches!(c, SinkCall::Fail { .. })));
    }

    #[test]
    fn install_failure_rolls_back_through_same_sink_without_second_session() {
        let temp = TestDir::new("install-rollback-same-sink");
        let manifest_path = temp.path().join("SameSinkApp-install.toml");
        let dir_to_create = temp.path().join("CreatedDir");

        fs::write(
            &manifest_path,
            format!(
                "app_name = 'SameSinkApp'\ndirectories = {{ paths = ['{}'] }}\n[[files]]\nsource = 'nonexistent.txt'\ndestination = '{}\\\\file.txt'\n",
                dir_to_create.display().to_string().replace("\\", "\\\\"),
                dir_to_create.display().to_string().replace("\\", "\\\\")
            ),
        )
        .unwrap();

        let sys = MockSys::new();
        sys.set_is_elevated(true);
        let sink_calls = sys.install_progress_sink();

        let err = install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();
        assert!(!matches!(err, AppError::Cancelled));
        assert!(!dir_to_create.exists());

        // The rollback must reuse the live session instead of starting a second one.
        let start_progress_calls = sys
            .recorded()
            .iter()
            .filter(|c| matches!(c, SysCall::StartProgress { .. }))
            .count();
        assert_eq!(start_progress_calls, 1);

        let calls = sink_calls.lock().unwrap().clone();
        assert!(calls.iter().any(
            |c| matches!(c, SinkCall::Advance { message, .. } if message.contains("Removing directory"))
        ));
        let fails = calls
            .iter()
            .filter(|c| matches!(c, SinkCall::Fail { .. }))
            .count();
        assert_eq!(fails, 1, "exactly one terminal fail message");
        assert!(!calls.iter().any(|c| matches!(c, SinkCall::Finish { .. })));
    }

    #[test]
    fn install_success_finish_wait_for_close_follows_automation_flag() {
        for automation in [false, true] {
            let temp = TestDir::new(if automation {
                "install-finish-wait-auto"
            } else {
                "install-finish-wait-manual"
            });
            let manifest_path = temp.path().join("FinishApp-install.toml");
            let dir_to_create = temp.path().join("CreatedDir");
            fs::write(
                &manifest_path,
                format!(
                    "app_name = 'FinishApp'\ndirectories = {{ paths = ['{}'] }}\n",
                    dir_to_create.display().to_string().replace("\\", "\\\\")
                ),
            )
            .unwrap();

            let sys = MockSys::new();
            sys.set_is_elevated(true);
            let sink_calls = sys.install_progress_sink();

            install(
                &manifest_path,
                None,
                false,
                UiMode::None,
                automation,
                &sys,
                None,
                &quiet_logger(),
            )
            .unwrap();

            let calls = sink_calls.lock().unwrap().clone();
            let wait_for_close = calls
                .iter()
                .find_map(|c| match c {
                    SinkCall::Finish { wait_for_close, .. } => Some(*wait_for_close),
                    _ => None,
                })
                .expect("finish recorded");
            assert_eq!(wait_for_close, !automation);
        }
    }

    #[test]
    fn install_emits_set_registry_string_calls_for_each_registry_spec() {
        let temp = TestDir::new("install-registry-mock");
        let manifest_path = temp.path().join("RegApp-install.toml");
        fs::write(
            &manifest_path,
            "app_name = 'RegApp'\n[[registry]]\nkey = 'HKCU\\\\Software\\\\RegApp'\nname = 'Alpha'\nvalue = 'A'\n[[registry]]\nkey = 'HKCU\\\\Software\\\\RegApp'\nname = 'Beta'\nvalue = 'B'\n",
        )
        .unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap();
        let manifest_writes: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::SetRegistryString { name, value, .. }
                    if name == "Alpha" || name == "Beta" =>
                {
                    Some((name, value))
                }
                _ => None,
            })
            .collect();
        assert_eq!(manifest_writes.len(), 2);
        assert!(
            manifest_writes
                .iter()
                .any(|(n, v)| n == "Alpha" && v == "A")
        );
        assert!(manifest_writes.iter().any(|(n, v)| n == "Beta" && v == "B"));
    }

    // (k)
}
