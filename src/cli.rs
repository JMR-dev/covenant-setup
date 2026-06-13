use crate::sys::Sys;
use crate::{
    AppError, EXIT_CANCELLED, EXIT_ELEVATION_REQUIRED, EXIT_OPERATION_FAILED, Logger, cleanup,
    install, package, uninstall,
};
use clap::{ArgAction, Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "covenant-setup",
    version,
    about = "Windows installer builder and engine"
)]
pub(crate) struct Cli {
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub(crate) json: bool,
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub(crate) headless: bool,
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "headless")]
    pub(crate) headed: bool,
    #[arg(long, global = true, hide = true, action = ArgAction::SetTrue)]
    pub(crate) automation: bool,
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub(crate) elevate: bool,
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Commands {
    Package {
        manifest: PathBuf,
        #[arg(long, default_value = "dist")]
        output: PathBuf,
    },
    Install {
        manifest: PathBuf,
        #[arg(long)]
        journal: Option<PathBuf>,
    },
    Uninstall {
        journal: PathBuf,
    },
    #[command(hide = true)]
    Cleanup {
        #[arg(long)]
        target_exe: PathBuf,
        #[arg(long)]
        install_root: Option<PathBuf>,
        #[arg(long)]
        app_name: String,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct UiPreferences {
    pub(crate) json: bool,
    pub(crate) headless: bool,
    pub(crate) headed: bool,
    pub(crate) automation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiMode {
    None,
    Gui,
    Tui,
}

pub(crate) fn error_exit_code(err: &AppError) -> i32 {
    match err {
        AppError::Cancelled | AppError::CancelledByUser => EXIT_CANCELLED,
        AppError::Message(message) if message.contains("Elevation required") => {
            EXIT_ELEVATION_REQUIRED
        }
        _ => EXIT_OPERATION_FAILED,
    }
}

pub(crate) fn dispatch(cli: Cli, sys: &dyn Sys, logger: &Logger) -> Result<(), AppError> {
    let preferences = ui_preferences_from_cli(&cli);
    let command = cli.command.ok_or_else(|| {
        AppError::Message(
            "Missing command: expected package, install, uninstall, or cleanup".into(),
        )
    })?;
    match command {
        Commands::Package { manifest, output } => package(&manifest, &output, logger),
        Commands::Install { manifest, journal } => install(
            &manifest,
            journal,
            cli.elevate,
            select_ui(preferences, sys, logger)?,
            preferences.automation,
            sys,
            None,
            logger,
        ),
        Commands::Uninstall { journal } => uninstall(
            &journal,
            cli.elevate,
            select_ui(preferences, sys, logger)?,
            preferences.automation,
            sys,
            &mut None,
            logger,
            false,
        ),
        Commands::Cleanup {
            target_exe,
            install_root,
            app_name,
        } => cleanup(
            target_exe,
            install_root,
            app_name,
            select_ui(preferences, sys, logger)?,
            preferences.automation,
            sys,
            logger,
        ),
    }
}

pub(crate) fn ui_preferences_from_cli(cli: &Cli) -> UiPreferences {
    UiPreferences {
        json: cli.json,
        headless: cli.headless,
        headed: cli.headed,
        automation: cli.automation,
    }
}

pub(crate) fn select_ui(
    preferences: UiPreferences,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<UiMode, AppError> {
    if preferences.json {
        return Ok(UiMode::None);
    }
    if preferences.headless && preferences.headed {
        return Err(AppError::Message(
            "Pass either --headed or --headless, not both".into(),
        ));
    }
    if preferences.headless {
        return Ok(UiMode::Tui);
    }
    if preferences.headed {
        if !sys.ui_available() {
            logger.info("gui_unavailable_fallback", json!({"fallback": "headless"}));
            return Ok(UiMode::Tui);
        }
        return Ok(UiMode::Gui);
    }
    Err(AppError::Message(
        "UI mode is required. Pass --headed for the WinForms UI, --headless for terminal progress, or --json for machine-readable output".into(),
    ))
}

pub(crate) fn ui_mode_name(ui_mode: UiMode) -> &'static str {
    match ui_mode {
        UiMode::None => "none",
        UiMode::Gui => "gui",
        UiMode::Tui => "tui",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::*;

    #[test]
    fn cli_parses_bundled_flags_without_manual_preparse() {
        let bundled = Cli::try_parse_from(["setup.exe", "--headed", "--automation"]).unwrap();
        assert!(bundled.command.is_none());
        assert!(bundled.headed);
        assert!(bundled.automation);

        let direct =
            Cli::try_parse_from(["setup.exe", "--headless", "install", "manifest.toml"]).unwrap();
        assert!(matches!(direct.command, Some(Commands::Install { .. })));
        assert!(direct.headless);
    }

    #[test]
    fn run_without_command_reports_missing_command() {
        let cli = Cli::try_parse_from(["setup.exe", "--headless"]).unwrap();
        let err = dispatch(cli, &WinSys, &quiet_logger())
            .unwrap_err()
            .to_string();

        assert!(err.contains("Missing command"));
    }

    #[test]
    fn run_package_command_creates_packaged_installer() {
        let temp = TestDir::new("run-package");
        let manifest = temp.path().join("RunPackage-install.toml");
        let output = temp.path().join("dist");
        fs::write(&manifest, "app_name = 'Run Package'\n").unwrap();

        let cli = Cli::try_parse_from([
            OsString::from("setup.exe"),
            OsString::from("--json"),
            OsString::from("package"),
            manifest.clone().into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ])
        .unwrap();
        dispatch(cli, &WinSys, &quiet_logger()).unwrap();

        let installer = output.join("covenant-setup-installer.exe");
        assert!(installer.is_file());
        assert_eq!(
            read_embedded_bundle(&installer)
                .unwrap()
                .unwrap()
                .metadata
                .app_name,
            "Run Package"
        );
    }

    #[test]
    fn ui_preferences_and_selection_cover_explicit_modes() {
        let json_cli = Cli::try_parse_from(["setup.exe", "--json", "--headed"]).unwrap();
        let json_preferences = ui_preferences_from_cli(&json_cli);
        assert!(json_preferences.json);
        assert_eq!(
            select_ui(json_preferences, &WinSys, &quiet_logger()).unwrap(),
            UiMode::None
        );

        let headless_cli = Cli::try_parse_from(["setup.exe", "--headless"]).unwrap();
        assert_eq!(
            select_ui(
                ui_preferences_from_cli(&headless_cli),
                &WinSys,
                &quiet_logger()
            )
            .unwrap(),
            UiMode::Tui
        );

        let missing = UiPreferences {
            json: false,
            headless: false,
            headed: false,
            automation: false,
        };
        assert!(select_ui(missing, &WinSys, &quiet_logger()).is_err());

        let conflict = UiPreferences {
            json: false,
            headless: true,
            headed: true,
            automation: false,
        };
        assert!(select_ui(conflict, &WinSys, &quiet_logger()).is_err());
    }

    #[test]
    fn headed_selection_uses_gui_or_documented_fallback() {
        let mode = select_ui(
            UiPreferences {
                json: false,
                headless: false,
                headed: true,
                automation: false,
            },
            &WinSys,
            &quiet_logger(),
        )
        .unwrap();

        assert!(matches!(mode, UiMode::Gui | UiMode::Tui));
    }

    #[test]
    fn ui_mode_names_are_stable() {
        assert_eq!(ui_mode_name(UiMode::None), "none");
        assert_eq!(ui_mode_name(UiMode::Gui), "gui");
        assert_eq!(ui_mode_name(UiMode::Tui), "tui");
    }

    #[test]
    fn error_exit_code_maps_cancelled_elevation_and_default() {
        assert_eq!(error_exit_code(&AppError::Cancelled), EXIT_CANCELLED);
        assert_eq!(error_exit_code(&AppError::CancelledByUser), EXIT_CANCELLED);
        assert_eq!(
            error_exit_code(&AppError::Message(
                "Elevation required to write to HKLM".into()
            )),
            EXIT_ELEVATION_REQUIRED
        );
        assert_eq!(
            error_exit_code(&AppError::Message("boom".into())),
            EXIT_OPERATION_FAILED
        );
    }
}
