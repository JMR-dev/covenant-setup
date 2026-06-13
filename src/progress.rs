use crate::sys::Sys;
use crate::ui::{self, ProgressSink};
use crate::{AppError, Logger, UiMode, trace_event, ui_mode_name};
use serde_json::json;
use std::io::{self, Write};
use std::path::Path;
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) static FAILURE_UX_SHOWN: AtomicBool = AtomicBool::new(false);

pub(crate) struct TuiProgress {
    active: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TuiProgress {
    fn start(label: String) -> Self {
        let active = Arc::new(AtomicBool::new(true));
        let active_thread = active.clone();
        let handle = thread::spawn(move || {
            let frames = ["", ".", "..", "...", "....", "....."];
            let mut index = 0usize;
            while active_thread.load(Ordering::Relaxed) {
                let frame = frames[index % frames.len()];
                print!("\r{label}{frame}   ");
                let _ = io::stdout().flush();
                thread::sleep(Duration::from_millis(500));
                index = (index + 1) % frames.len();
            }
            print!("\r{}\r", " ".repeat(label.len() + 8));
            let _ = io::stdout().flush();
        });
        Self {
            active,
            handle: Some(handle),
        }
    }
}

impl Drop for TuiProgress {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(crate) fn start_tui_progress(ui_mode: UiMode, label: String) -> Option<TuiProgress> {
    if ui_mode == UiMode::Tui {
        Some(TuiProgress::start(label))
    } else {
        None
    }
}

pub(crate) fn start_gui_progress(
    ui_mode: UiMode,
    sys: &dyn Sys,
    title: &str,
    app_name: Option<&str>,
    install_root: Option<&Path>,
    branding_image: Option<&str>,
    show_welcome: bool,
    total_steps: usize,
    logger: &Logger,
) -> Result<Option<Box<dyn ProgressSink>>, AppError> {
    trace_event(
        "gui_progress_start",
        json!({
            "ui_mode": ui_mode_name(ui_mode),
            "title": title,
            "total_steps": total_steps.max(1),
            "show_welcome": show_welcome
        }),
    );
    if let Some(sink) = sys.start_progress(ui_mode, title, total_steps.max(1))? {
        return Ok(Some(sink));
    }
    if ui_mode == UiMode::Gui {
        Ok(Some(Box::new(ui::GuiProgress::start(
            title,
            app_name,
            install_root
                .map(|p| p.to_string_lossy().into_owned())
                .as_deref(),
            branding_image,
            show_welcome,
            total_steps.max(1),
            logger,
        )?)))
    } else {
        Ok(None)
    }
}

pub(crate) fn advance_gui_progress(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    current_step: usize,
    total_steps: usize,
    message: &str,
) -> Result<(), AppError> {
    if gui_progress
        .as_mut()
        .is_some_and(|progress| progress.cancel_requested())
    {
        trace_event(
            "cancel_request_detected",
            json!({"current_step": current_step}),
        );
        return Err(AppError::Cancelled);
    }
    trace_event(
        "progress",
        json!({"current_step": current_step, "message": message}),
    );
    if let Some(progress) = gui_progress.as_mut() {
        progress.advance(current_step, total_steps, message)?;
    }
    Ok(())
}

pub(crate) fn advance_gui_progress_step(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    current_step: &mut usize,
    total_steps: usize,
    message: &str,
) -> Result<(), AppError> {
    *current_step += 1;
    advance_gui_progress(gui_progress, *current_step, total_steps, message)
}

pub(crate) fn finish_gui_progress(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    message: &str,
    wait_for_close: bool,
) -> Result<(), AppError> {
    trace_event("progress_finish", json!({"message": message}));
    if let Some(progress) = gui_progress.as_mut() {
        progress.finish(message, wait_for_close)?;
    }
    Ok(())
}

pub(crate) fn fail_gui_progress(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    app_name: &str,
    operation: &str,
    err: &AppError,
    support_contact: Option<&str>,
    wait_for_close: bool,
) -> Result<(), AppError> {
    let message = format!("Error: program {app_name} failed to {operation} completely!");
    trace_event(
        "progress_fail",
        json!({"app_name": app_name, "operation": operation, "message": message, "error": err.to_string()}),
    );
    if let Some(progress) = gui_progress.as_mut() {
        progress.fail(
            app_name,
            operation,
            &message,
            &err.to_string(),
            error_errata(app_name, operation, err, support_contact),
            wait_for_close,
        )?;
        mark_failure_ux_shown();
    }
    Ok(())
}

pub(crate) fn mark_failure_ux_shown() {
    FAILURE_UX_SHOWN.store(true, Ordering::Relaxed);
}

pub(crate) fn failure_ux_shown() -> bool {
    FAILURE_UX_SHOWN.load(Ordering::Relaxed)
}

pub(crate) fn error_errata(
    app_name: &str,
    operation: &str,
    err: &AppError,
    support_contact: Option<&str>,
) -> serde_json::Value {
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis());
    json!({
        "schema": "covenant_setup_errata_v1",
        "app_name": app_name,
        "operation": operation,
        "timestamp_unix_ms": timestamp_unix_ms,
        "support_contact": support_contact,
        "error": {
            "message": err.to_string(),
            "debug": format!("{err:?}"),
        },
        "process": {
            "pid": process::id(),
            "exe": std::env::current_exe().ok().map(|path| path.display().to_string()),
            "current_dir": std::env::current_dir().ok().map(|path| path.display().to_string()),
            "args": std::env::args_os()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            "trace_dir": std::env::var_os("COVENANT_SETUP_TRACE_DIR")
                .map(|path| path.to_string_lossy().to_string()),
        }
    })
}

pub(crate) fn append_gui_shell_output(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    bytes: &[u8],
) -> Result<(), AppError> {
    if bytes.is_empty() {
        return Ok(());
    }
    if let Some(progress) = gui_progress.as_mut() {
        let text = String::from_utf8_lossy(bytes);
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            trace_event("script_output", json!({"line": line}));
            progress.log(line)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::*;

    #[test]
    fn tui_progress_can_start_and_stop() {
        let progress = start_tui_progress(UiMode::Tui, "Testing ".to_string());
        assert!(progress.is_some());
        drop(progress);
        assert!(start_tui_progress(UiMode::None, "Testing ".to_string()).is_none());
    }

    #[test]
    fn progress_helpers_are_noops_without_gui_progress() {
        let mut progress: Option<Box<dyn ProgressSink>> = None;
        let mut step = 0;

        assert!(
            start_gui_progress(
                UiMode::None,
                &WinSys,
                "No UI",
                None,
                None,
                None,
                false,
                0,
                &quiet_logger()
            )
            .unwrap()
            .is_none()
        );
        advance_gui_progress(&mut progress, 1, 3, "step").unwrap();
        advance_gui_progress_step(&mut progress, &mut step, 3, "next").unwrap();
        finish_gui_progress(&mut progress, "done", false).unwrap();
        fail_gui_progress(
            &mut progress,
            "App",
            "install",
            &AppError::Message("boom".to_string()),
            None,
            false,
        )
        .unwrap();
        append_gui_shell_output(&mut progress, b"").unwrap();
        append_gui_shell_output(&mut progress, b"line 1\n\nline 2\n").unwrap();

        assert_eq!(step, 1);
    }

    #[test]
    fn failure_ux_marker_tracks_whether_failure_was_shown() {
        FAILURE_UX_SHOWN.store(false, Ordering::Relaxed);
        assert!(!failure_ux_shown());

        mark_failure_ux_shown();
        assert!(failure_ux_shown());

        FAILURE_UX_SHOWN.store(false, Ordering::Relaxed);
    }

    #[test]
    fn error_errata_contains_operation_error_and_process_context() {
        let errata = error_errata(
            "Errata App",
            "install",
            &AppError::Message("boom".into()),
            Some("support@example.com"),
        );

        assert_eq!(errata["schema"], "covenant_setup_errata_v1");
        assert_eq!(errata["app_name"], "Errata App");
        assert_eq!(errata["operation"], "install");
        assert_eq!(errata["error"]["message"], "boom");
        assert_eq!(errata["support_contact"], "support@example.com");
        assert!(errata["process"]["pid"].as_u64().is_some());
        assert!(errata["process"]["args"].as_array().is_some());
    }

    #[test]
    fn advance_gui_progress_returns_cancelled_when_sink_requests() {
        let handle = Arc::new(Mutex::new(Vec::new()));
        let sink = MockProgressSink {
            calls: handle.clone(),
            cancel_after_advances: Some(0),
        };
        let mut progress: Option<Box<dyn ProgressSink>> = Some(Box::new(sink));

        let err = advance_gui_progress(&mut progress, 1, 3, "step").unwrap_err();
        assert!(matches!(err, AppError::Cancelled));
        assert!(handle.lock().unwrap().is_empty());

        progress.as_mut().unwrap().clear_cancel_request();
        advance_gui_progress(&mut progress, 1, 3, "step").unwrap();
        assert_eq!(handle.lock().unwrap().len(), 1);
    }

    #[test]
    fn progress_sink_mock_records_calls_through_advance_log_finish_fail() {
        let recorder = MockProgressSink::new();
        let handle = recorder.handle();
        let mut sink: Box<dyn ProgressSink> = Box::new(recorder);
        sink.advance(2, 5, "step 2").unwrap();
        sink.log("note").unwrap();
        sink.finish("done", true).unwrap();
        sink.fail(
            "App",
            "install",
            "boom",
            "io error",
            serde_json::json!({"k":"v"}),
            true,
        )
        .unwrap();
        let calls = handle.lock().unwrap().clone();
        assert_eq!(calls.len(), 4);
        assert!(matches!(
            &calls[0],
            SinkCall::Advance { current_step: 2, total_steps: 5, message } if message == "step 2"
        ));
        assert!(matches!(&calls[1], SinkCall::Log(s) if s == "note"));
        assert!(matches!(
            &calls[2],
            SinkCall::Finish { message, wait_for_close } if message == "done" && *wait_for_close
        ));
        assert!(matches!(
            &calls[3],
            SinkCall::Fail { app_name, operation, error, wait_for_close, .. }
                if app_name == "App" && operation == "install" && error == "io error" && *wait_for_close
        ));
    }
}
