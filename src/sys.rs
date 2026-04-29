use crate::ui::ProgressSink;
use crate::{AppError, Logger, RegistryRoot, UiMode};
use std::path::Path;

/// Abstraction over every external boundary the installer engine touches:
/// UAC elevation, reboot, cleanup-helper self-delete, registry writes, the
/// embedded-bundle probe, the high-level UI prompts, and the MoveFileEx
/// pending-rename fallback.
///
/// This trait exists so the `install` / `uninstall` / `cleanup` /
/// `run_bundled_installer` orchestration code can be unit-tested with mocks
/// without spawning Win32 / process / GUI IPC side-effects.
pub(crate) trait Sys: Send + Sync {
    // (1) UAC relaunch
    fn is_elevated(&self, logger: &Logger) -> Result<bool, AppError>;
    fn relaunch_as_admin(&self, logger: &Logger) -> Result<(), AppError>;

    // (2) reboot
    fn spawn_reboot(&self, logger: &Logger) -> Result<(), AppError>;
    fn prompt_reboot_tui(&self) -> Result<bool, AppError>;

    // (3) cleanup helper self-delete
    fn spawn_cleanup_helper(
        &self,
        target_exe: &Path,
        install_root: Option<&Path>,
        app_name: &str,
        ui_mode: UiMode,
        automation: bool,
        json: bool,
        logger: &Logger,
    ) -> Result<(), AppError>;
    fn schedule_helper_self_cleanup(&self, logger: &Logger) -> Result<bool, AppError>;

    // (4) registry writes
    fn set_registry_string(
        &self,
        root: RegistryRoot,
        subkey: &str,
        name: &str,
        value: &str,
        logger: &Logger,
    ) -> Result<(), AppError>;
    fn delete_registry_tree(
        &self,
        root: RegistryRoot,
        subkey: &str,
        logger: &Logger,
    ) -> Result<(), AppError>;

    // (5) bundled-installer probe
    fn has_embedded_bundle(&self) -> bool;

    // (6) UI prompts (high level — the GuiProgress trait handles the live IPC)
    fn ui_available(&self) -> bool;
    fn ui_confirm_install(&self, app_name: &str) -> Result<bool, AppError>;
    fn ui_report_success(&self, app_name: &str) -> Result<(), AppError>;
    fn ui_report_error(&self, message: &str) -> Result<(), AppError>;
    fn ui_report_uninstall_success(&self, app_name: &str) -> Result<(), AppError>;
    fn ui_prompt_uninstall_reboot(&self, app_name: &str) -> Result<bool, AppError>;

    // (7) MoveFileEx reboot fallback
    fn remove_file_with_fallback(&self, path: &Path, logger: &Logger) -> Result<(), AppError>;

    // Optional: lets MockSys substitute a recording ProgressSink in tests.
    // Default returns None so install/uninstall fall back to constructing a
    // real GuiProgress via crate::start_gui_progress when desired.
    fn start_progress(
        &self,
        _ui_mode: UiMode,
        _title: &str,
        _total_steps: usize,
    ) -> Result<Option<Box<dyn ProgressSink>>, AppError> {
        Ok(None)
    }
}

/// Production implementation that delegates to the real Win32 / process /
/// GUI IPC functions.
pub(crate) struct WinSys;

impl Sys for WinSys {
    fn is_elevated(&self, logger: &Logger) -> Result<bool, AppError> {
        crate::win::is_elevated(logger)
    }

    fn relaunch_as_admin(&self, logger: &Logger) -> Result<(), AppError> {
        crate::win::relaunch_as_admin(logger)
    }

    fn spawn_reboot(&self, logger: &Logger) -> Result<(), AppError> {
        crate::spawn_reboot(logger)
    }

    fn prompt_reboot_tui(&self) -> Result<bool, AppError> {
        crate::prompt_reboot_tui()
    }

    fn spawn_cleanup_helper(
        &self,
        target_exe: &Path,
        install_root: Option<&Path>,
        app_name: &str,
        ui_mode: UiMode,
        automation: bool,
        json: bool,
        logger: &Logger,
    ) -> Result<(), AppError> {
        crate::spawn_cleanup_helper(
            target_exe,
            install_root,
            app_name,
            ui_mode,
            automation,
            json,
            logger,
        )
    }

    fn schedule_helper_self_cleanup(&self, logger: &Logger) -> Result<bool, AppError> {
        crate::schedule_helper_self_cleanup(logger)
    }

    fn set_registry_string(
        &self,
        root: RegistryRoot,
        subkey: &str,
        name: &str,
        value: &str,
        logger: &Logger,
    ) -> Result<(), AppError> {
        crate::win::set_registry_string(root, subkey, name, value, logger)
    }

    fn delete_registry_tree(
        &self,
        root: RegistryRoot,
        subkey: &str,
        logger: &Logger,
    ) -> Result<(), AppError> {
        crate::win::delete_registry_tree(root, subkey, logger)
    }

    fn has_embedded_bundle(&self) -> bool {
        crate::has_embedded_bundle()
    }

    fn ui_available(&self) -> bool {
        crate::ui::is_available()
    }

    fn ui_confirm_install(&self, app_name: &str) -> Result<bool, AppError> {
        crate::ui::confirm_install(app_name)
    }

    fn ui_report_success(&self, app_name: &str) -> Result<(), AppError> {
        crate::ui::report_success(app_name)
    }

    fn ui_report_error(&self, message: &str) -> Result<(), AppError> {
        crate::ui::report_error(message)
    }

    fn ui_report_uninstall_success(&self, app_name: &str) -> Result<(), AppError> {
        crate::ui::report_uninstall_success(app_name)
    }

    fn ui_prompt_uninstall_reboot(&self, app_name: &str) -> Result<bool, AppError> {
        crate::ui::prompt_uninstall_reboot(app_name)
    }

    fn remove_file_with_fallback(&self, path: &Path, logger: &Logger) -> Result<(), AppError> {
        crate::win::remove_file_with_fallback(path, logger)
    }
}
