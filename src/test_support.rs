//! Shared test doubles and fixtures for the per-module unit suites.
//! Relocated verbatim from the former monolithic test module.
//!
//! Re-exports the crate prelude plus the std items the doubles and the
//! distributed test modules rely on, so each `mod tests` only needs
//! `use super::*; use crate::test_support::*;`.

pub(crate) use crate::*;
pub(crate) use std::fs;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process;
pub(crate) use std::sync::{Arc, Mutex};
pub(crate) use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "covenant-setup-test-{name}-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// -------------------------------------------------------------------------
// Mock infrastructure for the Sys trait + ProgressSink trait. These mocks
// record every boundary call the orchestration code makes so tests can
// assert on exact sequences without spawning Win32 / process / GUI side
// effects.
// -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SysCall {
    IsElevated,
    RelaunchAsAdmin,
    SpawnReboot,
    PromptRebootTui,
    SpawnCleanupHelper {
        target_exe: PathBuf,
        install_root: Option<PathBuf>,
        app_name: String,
        ui_mode: UiMode,
        automation: bool,
        json: bool,
    },
    ScheduleHelperSelfCleanup,
    SetRegistryString {
        root: RegistryRoot,
        subkey: String,
        name: String,
        value: String,
    },
    DeleteRegistryTree {
        root: RegistryRoot,
        subkey: String,
    },
    HasEmbeddedBundle,
    UiAvailable,
    UiReportError(String),
    UiReportUninstallSuccess(String),
    UiPromptUninstallReboot(String),
    RemoveFileWithFallback(PathBuf),
    StartProgress {
        ui_mode: UiMode,
        title: String,
        total_steps: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SinkCall {
    Advance {
        current_step: usize,
        total_steps: usize,
        message: String,
    },
    Log(String),
    Finish {
        message: String,
        wait_for_close: bool,
    },
    Fail {
        app_name: String,
        operation: String,
        message: String,
        error: String,
        wait_for_close: bool,
    },
}

#[derive(Default)]
pub(crate) struct MockProgressSink {
    pub(crate) calls: Arc<Mutex<Vec<SinkCall>>>,
    // When Some(n), cancel_requested() reports true once n Advance calls
    // have been recorded; clear_cancel_request() resets it to None.
    pub(crate) cancel_after_advances: Option<usize>,
}

impl MockProgressSink {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn handle(&self) -> Arc<Mutex<Vec<SinkCall>>> {
        self.calls.clone()
    }

    pub(crate) fn recorded_advances(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, SinkCall::Advance { .. }))
            .count()
    }
}

impl ProgressSink for MockProgressSink {
    fn advance(
        &mut self,
        current_step: usize,
        total_steps: usize,
        message: &str,
    ) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(SinkCall::Advance {
            current_step,
            total_steps,
            message: message.to_string(),
        });
        Ok(())
    }

    fn log(&mut self, message: &str) -> Result<(), AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SinkCall::Log(message.to_string()));
        Ok(())
    }

    fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(SinkCall::Finish {
            message: message.to_string(),
            wait_for_close,
        });
        Ok(())
    }

    fn cancel_requested(&mut self) -> bool {
        self.cancel_after_advances
            .is_some_and(|threshold| self.recorded_advances() >= threshold)
    }

    fn clear_cancel_request(&mut self) {
        self.cancel_after_advances = None;
    }

    fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        _errata: serde_json::Value,
        wait_for_close: bool,
    ) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(SinkCall::Fail {
            app_name: app_name.to_string(),
            operation: operation.to_string(),
            message: message.to_string(),
            error: error.to_string(),
            wait_for_close,
        });
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct MockSys {
    calls: Mutex<Vec<SysCall>>,
    is_elevated: Mutex<bool>,
    ui_available: Mutex<bool>,
    ui_prompt_uninstall_reboot: Mutex<bool>,
    schedule_helper_self_cleanup: Mutex<bool>,
    prompt_reboot_tui: Mutex<bool>,
    has_embedded_bundle: Mutex<bool>,
    progress_sink_calls: Mutex<Option<Arc<Mutex<Vec<SinkCall>>>>>,
    progress_sink_cancel_after: Mutex<Option<usize>>,
}

#[allow(dead_code)]
impl MockSys {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn recorded(&self) -> Vec<SysCall> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn set_is_elevated(&self, value: bool) {
        *self.is_elevated.lock().unwrap() = value;
    }

    pub(crate) fn set_ui_available(&self, value: bool) {
        *self.ui_available.lock().unwrap() = value;
    }

    pub(crate) fn set_ui_prompt_uninstall_reboot(&self, value: bool) {
        *self.ui_prompt_uninstall_reboot.lock().unwrap() = value;
    }

    pub(crate) fn set_schedule_helper_self_cleanup(&self, value: bool) {
        *self.schedule_helper_self_cleanup.lock().unwrap() = value;
    }

    pub(crate) fn set_prompt_reboot_tui(&self, value: bool) {
        *self.prompt_reboot_tui.lock().unwrap() = value;
    }

    pub(crate) fn install_progress_sink(&self) -> Arc<Mutex<Vec<SinkCall>>> {
        let sink = MockProgressSink::new();
        let handle = sink.handle();
        *self.progress_sink_calls.lock().unwrap() = Some(handle.clone());
        // Box and stash a fresh sink each call to start_progress; use the
        // shared handle so tests can read the recorded calls.
        handle
    }

    pub(crate) fn set_cancel_after_advances(&self, threshold: usize) {
        *self.progress_sink_cancel_after.lock().unwrap() = Some(threshold);
    }
}

impl Sys for MockSys {
    fn is_elevated(&self, _logger: &Logger) -> Result<bool, AppError> {
        self.calls.lock().unwrap().push(SysCall::IsElevated);
        Ok(*self.is_elevated.lock().unwrap())
    }

    fn relaunch_as_admin(&self, _logger: &Logger) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(SysCall::RelaunchAsAdmin);
        Ok(())
    }

    fn spawn_reboot(&self, _logger: &Logger) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(SysCall::SpawnReboot);
        Ok(())
    }

    fn prompt_reboot_tui(&self) -> Result<bool, AppError> {
        self.calls.lock().unwrap().push(SysCall::PromptRebootTui);
        Ok(*self.prompt_reboot_tui.lock().unwrap())
    }

    fn spawn_cleanup_helper(
        &self,
        target_exe: &Path,
        install_root: Option<&Path>,
        app_name: &str,
        ui_mode: UiMode,
        automation: bool,
        json: bool,
        _logger: &Logger,
    ) -> Result<(), AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SysCall::SpawnCleanupHelper {
                target_exe: target_exe.to_path_buf(),
                install_root: install_root.map(Path::to_path_buf),
                app_name: app_name.to_string(),
                ui_mode,
                automation,
                json,
            });
        Ok(())
    }

    fn schedule_helper_self_cleanup(&self, _logger: &Logger) -> Result<bool, AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SysCall::ScheduleHelperSelfCleanup);
        Ok(*self.schedule_helper_self_cleanup.lock().unwrap())
    }

    fn set_registry_string(
        &self,
        root: RegistryRoot,
        subkey: &str,
        name: &str,
        value: &str,
        _logger: &Logger,
    ) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(SysCall::SetRegistryString {
            root,
            subkey: subkey.to_string(),
            name: name.to_string(),
            value: value.to_string(),
        });
        Ok(())
    }

    fn delete_registry_tree(
        &self,
        root: RegistryRoot,
        subkey: &str,
        _logger: &Logger,
    ) -> Result<(), AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SysCall::DeleteRegistryTree {
                root,
                subkey: subkey.to_string(),
            });
        Ok(())
    }

    fn has_embedded_bundle(&self) -> bool {
        self.calls.lock().unwrap().push(SysCall::HasEmbeddedBundle);
        *self.has_embedded_bundle.lock().unwrap()
    }

    fn ui_available(&self) -> bool {
        self.calls.lock().unwrap().push(SysCall::UiAvailable);
        *self.ui_available.lock().unwrap()
    }

    fn ui_report_error(&self, message: &str) -> Result<(), AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SysCall::UiReportError(message.to_string()));
        Ok(())
    }

    fn ui_report_uninstall_success(&self, app_name: &str) -> Result<(), AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SysCall::UiReportUninstallSuccess(app_name.to_string()));
        Ok(())
    }

    fn ui_prompt_uninstall_reboot(&self, app_name: &str) -> Result<bool, AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SysCall::UiPromptUninstallReboot(app_name.to_string()));
        Ok(*self.ui_prompt_uninstall_reboot.lock().unwrap())
    }

    fn remove_file_with_fallback(&self, path: &Path, _logger: &Logger) -> Result<(), AppError> {
        self.calls
            .lock()
            .unwrap()
            .push(SysCall::RemoveFileWithFallback(path.to_path_buf()));
        // Best-effort delete the real file so subsequent fs::read_dir checks
        // in cleanup() see the directory as empty.
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    fn start_progress(
        &self,
        ui_mode: UiMode,
        title: &str,
        total_steps: usize,
    ) -> Result<Option<Box<dyn ProgressSink>>, AppError> {
        self.calls.lock().unwrap().push(SysCall::StartProgress {
            ui_mode,
            title: title.to_string(),
            total_steps,
        });
        // Only inject a recording sink when test code explicitly opted in.
        if let Some(handle) = self.progress_sink_calls.lock().unwrap().clone() {
            let sink = MockProgressSink {
                calls: handle.clone(),
                cancel_after_advances: *self.progress_sink_cancel_after.lock().unwrap(),
            };
            Ok(Some(Box::new(sink) as Box<dyn ProgressSink>))
        } else {
            Ok(None)
        }
    }
}

pub(crate) fn sample_manifest() -> InstallManifest {
    InstallManifest {
        app_name: "Sample App".to_string(),
        directories: vec!["C:\\Apps\\Sample\\bin".to_string()],
        files: vec![FileSpec {
            source: "payload\\app.exe".to_string(),
            destination: "C:\\Apps\\Sample\\app.exe".to_string(),
        }],
        registry: vec![RegistrySpec {
            key: "HKCU\\Software\\Sample".to_string(),
            name: "InstallLocation".to_string(),
            value: "C:\\Apps\\Sample".to_string(),
        }],
        shortcuts: vec![ShortcutSpec {
            path: "C:\\Users\\Public\\Desktop\\Sample.lnk".to_string(),
            target: "C:\\Apps\\Sample\\app.exe".to_string(),
            arguments: None,
            working_directory: None,
            description: None,
        }],
        scripts: vec![ScriptSpec {
            command: "post-install.cmd".to_string(),
            args: vec!["--ok".to_string()],
            working_directory: None,
        }],
        purge: PurgeSpec {
            registry_branches: vec![],
            paths: vec!["C:\\Apps\\Sample".to_string()],
        },
        support_contact: None,
    }
}

pub(crate) fn write_embedded_payload(exe: &Path, payload: &[u8]) {
    let mut bytes = payload.to_vec();
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(EMBEDDED_MAGIC);
    fs::write(exe, bytes).unwrap();
}

pub(crate) fn quiet_logger() -> Logger {
    Logger {
        json: false,
        quiet: true,
    }
}
