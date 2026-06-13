mod bundle;
mod cli;
mod error;
mod install;
mod journal;
mod logging;
mod manifest;
mod progress;
mod sys;
mod ui;
mod uninstall;
mod util;
mod win;

#[cfg(test)]
mod test_support;

pub(crate) use bundle::*;
pub(crate) use cli::*;
pub(crate) use error::{AppError, EXIT_CANCELLED, EXIT_ELEVATION_REQUIRED, EXIT_OPERATION_FAILED};
pub(crate) use install::*;
pub(crate) use journal::*;
pub(crate) use logging::{Logger, trace_event};
pub(crate) use manifest::*;
pub(crate) use progress::*;
pub(crate) use uninstall::*;
pub(crate) use util::*;

use clap::Parser;
use serde_json::json;
use std::ffi::OsString;
use std::process;
use std::thread;
use std::time::Duration;
use sys::{Sys, WinSys};
use ui::ProgressSink;

pub fn run() -> i32 {
    let args: Vec<_> = std::env::args_os().collect();
    trace_event(
        "process_start",
        json!({
            "pid": process::id(),
            "args": args.iter().map(|arg| arg.to_string_lossy().to_string()).collect::<Vec<_>>()
        }),
    );
    let cli = Cli::parse();
    let logger = Logger {
        json: cli.json,
        quiet: false,
    };
    let preferences = ui_preferences_from_cli(&cli);
    let sys = WinSys;
    if cli.command.is_none() && sys.has_embedded_bundle() {
        let exit_code = match run_bundled_installer(preferences, &sys, &logger) {
            Ok(()) => 0,
            Err(AppError::Message(ref message)) if message == "__elevated_relaunch__" => 0,
            Err(err) => {
                if preferences.headed
                    && !preferences.automation
                    && !preferences.json
                    && sys.ui_available()
                    && !failure_ux_shown()
                    && !matches!(err, AppError::Cancelled | AppError::CancelledByUser)
                {
                    let _ = sys.ui_report_error(&err.to_string());
                }
                let code = error_exit_code(&err);
                logger.error(err, code);
                code
            }
        };
        return exit_code;
    }

    match dispatch(cli, &sys, &logger) {
        Ok(()) => 0,
        Err(AppError::Message(message)) if message == "__elevated_relaunch__" => 0,
        Err(err) => {
            let code = error_exit_code(&err);
            logger.error(err, code);
            code
        }
    }
}
