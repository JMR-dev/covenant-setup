use crate::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{self, Child, Command};
use std::thread;
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const UI_EXE_NAME: &str = "Covenant.Setup.Ui.exe";

#[cfg(covenant_setup_embedded_ui)]
fn embedded_ui_bytes() -> Option<&'static [u8]> {
    Some(include_bytes!(env!("COVENANT_SETUP_UI_EXE")))
}

#[cfg(not(covenant_setup_embedded_ui))]
fn embedded_ui_bytes() -> Option<&'static [u8]> {
    None
}

pub fn is_available() -> bool {
    embedded_ui_bytes().is_some() || sidecar_ui_exe().is_some()
}

/// Trait abstraction over the live GUI progress IPC channel so install /
/// uninstall code can be unit-tested with a recording mock instead of
/// spawning the real C# UI.
pub trait ProgressSink: Send {
    fn advance(&mut self, current_step: usize, message: &str) -> Result<(), AppError>;
    fn log(&mut self, message: &str) -> Result<(), AppError>;
    fn finish(&mut self, message: &str) -> Result<(), AppError>;
    fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        errata: Value,
        wait_for_close: bool,
    ) -> Result<(), AppError>;
}

impl ProgressSink for GuiProgress {
    fn advance(&mut self, current_step: usize, message: &str) -> Result<(), AppError> {
        GuiProgress::advance(self, current_step, message)
    }

    fn log(&mut self, message: &str) -> Result<(), AppError> {
        GuiProgress::log(self, message)
    }

    fn finish(&mut self, message: &str) -> Result<(), AppError> {
        GuiProgress::finish(self, message)
    }

    fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        errata: Value,
        wait_for_close: bool,
    ) -> Result<(), AppError> {
        GuiProgress::fail(
            self,
            app_name,
            operation,
            message,
            error,
            errata,
            wait_for_close,
        )
    }
}

pub struct GuiProgress {
    session: CSharpUiSession,
    total_steps: usize,
}

impl GuiProgress {
    pub fn start(title: &str, initial_message: &str, total_steps: usize) -> Result<Self, AppError> {
        let mut session = CSharpUiSession::start()?;
        session.send(&json!({
            "type": "init",
            "title": title,
            "message": initial_message,
            "total_steps": total_steps.max(1),
        }))?;
        Ok(Self {
            session,
            total_steps: total_steps.max(1),
        })
    }

    pub fn advance(&mut self, current_step: usize, message: &str) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "progress",
            "current_step": current_step,
            "total_steps": self.total_steps,
            "message": message,
        }))
    }

    pub fn log(&mut self, message: &str) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "log",
            "message": message,
        }))
    }

    pub fn finish(&mut self, message: &str) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "finish",
            "message": message,
        }))
    }

    pub fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        errata: Value,
        wait_for_close: bool,
    ) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "fail",
            "app_name": app_name,
            "operation": operation,
            "message": message,
            "error": error,
            "errata": errata,
        }))?;
        if wait_for_close {
            self.session.wait_for_exit()?;
        }
        Ok(())
    }
}

pub fn confirm_install(app_name: &str) -> Result<bool, AppError> {
    crate::trace_event("ui_prompt_confirm_install", json!({"app_name": app_name}));
    let result = prompt(
        "covenant-setup",
        &format!("Install {app_name} now?"),
        PromptButtons::OkCancel,
        PromptIcon::Information,
    )?;
    Ok(matches!(result, PromptResult::Ok))
}

pub fn report_success(app_name: &str) -> Result<(), AppError> {
    crate::trace_event("ui_prompt_report_success", json!({"app_name": app_name}));
    let _ = prompt(
        "covenant-setup",
        &format!("{app_name} installation completed successfully"),
        PromptButtons::Ok,
        PromptIcon::Information,
    )?;
    Ok(())
}

pub fn report_error(message: &str) -> Result<(), AppError> {
    crate::trace_event("ui_prompt_report_error", json!({"message": message}));
    let _ = prompt(
        "covenant-setup",
        message,
        PromptButtons::Ok,
        PromptIcon::Error,
    )?;
    Ok(())
}

pub fn report_uninstall_success(app_name: &str) -> Result<(), AppError> {
    crate::trace_event(
        "ui_prompt_report_uninstall_success",
        json!({"app_name": app_name}),
    );
    let _ = prompt(
        "covenant-setup",
        &format!("{app_name} uninstalled successfully!"),
        PromptButtons::Ok,
        PromptIcon::Information,
    )?;
    Ok(())
}

pub fn prompt_uninstall_reboot(app_name: &str) -> Result<bool, AppError> {
    crate::trace_event("ui_prompt_uninstall_reboot", json!({"app_name": app_name}));
    let result = prompt(
        "covenant-setup",
        &format!(
            "{app_name} uninstalled successfully! Some files from the program still remain on your computer. To complete removal of these files, restart your computer now."
        ),
        PromptButtons::YesNo,
        PromptIcon::Information,
    )?;
    Ok(matches!(result, PromptResult::Yes))
}

fn prompt(
    title: &str,
    message: &str,
    buttons: PromptButtons,
    icon: PromptIcon,
) -> Result<PromptResult, AppError> {
    let mut session = CSharpUiSession::start()?;
    let id = format!("prompt-{}", unique_suffix());
    session.send(&json!({
        "type": "prompt",
        "id": id,
        "title": title,
        "message": message,
        "buttons": buttons.as_str(),
        "icon": icon.as_str(),
    }))?;
    let response: PromptResponse = session.read()?;
    if response.message_type != "prompt_response" || response.id.as_deref() != Some(&id) {
        return Err(AppError::Message("Unexpected UI prompt response".into()));
    }
    let result = PromptResult::from_str(response.result.as_deref().unwrap_or("none"))?;
    crate::trace_event(
        "ui_prompt_response",
        json!({"id": id, "result": response.result}),
    );
    Ok(result)
}

enum PromptButtons {
    Ok,
    OkCancel,
    YesNo,
}

impl PromptButtons {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::OkCancel => "ok_cancel",
            Self::YesNo => "yes_no",
        }
    }
}

enum PromptIcon {
    Information,
    Error,
}

impl PromptIcon {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Information => "information",
            Self::Error => "error",
        }
    }
}

enum PromptResult {
    Ok,
    Cancel,
    Yes,
    No,
    None,
}

impl PromptResult {
    fn from_str(value: &str) -> Result<Self, AppError> {
        match value {
            "ok" => Ok(Self::Ok),
            "cancel" => Ok(Self::Cancel),
            "yes" => Ok(Self::Yes),
            "no" => Ok(Self::No),
            "none" => Ok(Self::None),
            other => Err(AppError::Message(format!(
                "Unknown UI prompt response: {other}"
            ))),
        }
    }
}

struct CSharpUiSession {
    child: Child,
    reader: BufReader<fs::File>,
    writer: fs::File,
    exe_path: PathBuf,
    remove_exe_on_drop: bool,
    closed: bool,
    child_exited: bool,
}

impl CSharpUiSession {
    fn start() -> Result<Self, AppError> {
        let pipe_name = format!("covenant-setup-ui-{}-{}", process::id(), unique_suffix());
        crate::trace_event("ui_start", json!({"pipe_name": pipe_name}));
        let prepared_exe = prepare_ui_exe()?;
        let exe_path = prepared_exe.path;
        crate::trace_event("ui_extracted", json!({"exe_path": &exe_path}));
        let mut child = Command::new(&exe_path)
            .creation_flags(CREATE_NO_WINDOW)
            .arg("--pipe")
            .arg(&pipe_name)
            .spawn()?;
        crate::trace_event(
            "ui_spawned",
            json!({"pid": child.id(), "exe_path": &exe_path}),
        );
        let pipe_path = format!(r"\\.\pipe\{pipe_name}");
        let pipe = connect_pipe(&pipe_path, &mut child)?;
        crate::trace_event("ui_pipe_connected", json!({"pipe_path": pipe_path}));
        let writer = pipe.try_clone()?;
        Ok(Self {
            child,
            reader: BufReader::new(pipe),
            writer,
            exe_path,
            remove_exe_on_drop: prepared_exe.remove_on_drop,
            closed: false,
            child_exited: false,
        })
    }

    fn send<T: Serialize>(&mut self, value: &T) -> Result<(), AppError> {
        let value = serde_json::to_value(value)?;
        crate::trace_event("ui_pipe_send", message_summary(&value));
        let bytes = serde_json::to_vec(&value)?;
        self.writer.write_all(&bytes)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    fn read<T: for<'de> Deserialize<'de>>(&mut self) -> Result<T, AppError> {
        let mut line = String::new();
        let bytes = self.reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err(AppError::Message("UI pipe closed before response".into()));
        }
        let value: Value = serde_json::from_str(&line)?;
        crate::trace_event("ui_pipe_receive", message_summary(&value));
        Ok(serde_json::from_value(value)?)
    }

    fn wait_for_exit(&mut self) -> Result<(), AppError> {
        self.closed = true;
        let status = self.child.wait()?;
        self.child_exited = true;
        crate::trace_event(
            "ui_failure_window_closed",
            json!({"pid": self.child.id(), "status": status.code()}),
        );
        Ok(())
    }
}

impl Drop for CSharpUiSession {
    fn drop(&mut self) {
        if self.child_exited {
            if self.remove_exe_on_drop {
                let _ = fs::remove_file(&self.exe_path);
            }
            return;
        }
        if !self.closed {
            crate::trace_event("ui_close_send", json!({"pid": self.child.id()}));
            let _ = self.send(&json!({"type": "close"}));
            self.closed = true;
        }
        for _ in 0..20 {
            if self.child.try_wait().ok().flatten().is_some() {
                crate::trace_event("ui_exited", json!({"pid": self.child.id()}));
                if self.remove_exe_on_drop {
                    let _ = fs::remove_file(&self.exe_path);
                }
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        crate::trace_event("ui_killed", json!({"pid": self.child.id()}));
        if self.remove_exe_on_drop {
            let _ = fs::remove_file(&self.exe_path);
        }
    }
}

#[derive(Deserialize)]
struct PromptResponse {
    #[serde(rename = "type")]
    message_type: String,
    id: Option<String>,
    result: Option<String>,
}

fn connect_pipe(pipe_path: &str, child: &mut Child) -> Result<fs::File, AppError> {
    let deadline = Instant::now() + Duration::from_secs(15);
    crate::trace_event("ui_pipe_connect_wait", json!({"pipe_path": pipe_path}));
    loop {
        match OpenOptions::new().read(true).write(true).open(pipe_path) {
            Ok(file) => return Ok(file),
            Err(err) => {
                if let Some(status) = child.try_wait()? {
                    crate::trace_event(
                        "ui_pipe_connect_child_exited",
                        json!({"pipe_path": pipe_path, "status": status.to_string()}),
                    );
                    return Err(AppError::Message(format!(
                        "C# UI exited before pipe connection: {status}"
                    )));
                }
                if Instant::now() >= deadline {
                    crate::trace_event(
                        "ui_pipe_connect_timeout",
                        json!({"pipe_path": pipe_path, "error": err.to_string()}),
                    );
                    return Err(AppError::Message(format!(
                        "Timed out connecting to C# UI pipe {pipe_path}: {err}"
                    )));
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

struct PreparedUiExe {
    path: PathBuf,
    remove_on_drop: bool,
}

fn prepare_ui_exe() -> Result<PreparedUiExe, AppError> {
    if let Some(bytes) = embedded_ui_bytes() {
        return extract_ui_exe(bytes);
    }
    if let Some(path) = sidecar_ui_exe() {
        return Ok(PreparedUiExe {
            path,
            remove_on_drop: false,
        });
    }
    Err(AppError::Message(format!(
        "C# UI helper is not bundled and no {UI_EXE_NAME} was found next to the installer"
    )))
}

fn extract_ui_exe(bytes: &[u8]) -> Result<PreparedUiExe, AppError> {
    let root = std::env::temp_dir().join("covenant-setup-ui");
    fs::create_dir_all(&root)?;
    let path = root.join(format!(
        "Covenant.Setup.Ui-{}-{}.exe",
        process::id(),
        unique_suffix()
    ));
    fs::write(&path, bytes)?;
    Ok(PreparedUiExe {
        path,
        remove_on_drop: true,
    })
}

fn sidecar_ui_exe() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(UI_EXE_NAME);
    path.is_file().then_some(path)
}

fn message_summary(value: &Value) -> Value {
    json!({
        "type": value.get("type").and_then(Value::as_str),
        "id": value.get("id").and_then(Value::as_str),
        "message": value.get("message").and_then(Value::as_str),
    })
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_button_and_icon_names_match_protocol() {
        assert_eq!(PromptButtons::Ok.as_str(), "ok");
        assert_eq!(PromptButtons::OkCancel.as_str(), "ok_cancel");
        assert_eq!(PromptButtons::YesNo.as_str(), "yes_no");
        assert_eq!(PromptIcon::Information.as_str(), "information");
        assert_eq!(PromptIcon::Error.as_str(), "error");
    }

    #[test]
    fn prompt_result_parses_known_values_and_rejects_unknown_values() {
        assert!(matches!(
            PromptResult::from_str("ok").unwrap(),
            PromptResult::Ok
        ));
        assert!(matches!(
            PromptResult::from_str("cancel").unwrap(),
            PromptResult::Cancel
        ));
        assert!(matches!(
            PromptResult::from_str("yes").unwrap(),
            PromptResult::Yes
        ));
        assert!(matches!(
            PromptResult::from_str("no").unwrap(),
            PromptResult::No
        ));
        assert!(matches!(
            PromptResult::from_str("none").unwrap(),
            PromptResult::None
        ));
        assert!(PromptResult::from_str("maybe").is_err());
    }

    #[test]
    fn message_summary_extracts_only_safe_protocol_fields() {
        let summary = message_summary(&json!({
            "type": "progress",
            "id": "abc",
            "message": "Working",
            "errata": {"secret": true}
        }));

        assert_eq!(summary["type"], "progress");
        assert_eq!(summary["id"], "abc");
        assert_eq!(summary["message"], "Working");
        assert!(summary.get("errata").is_none());
    }

    #[test]
    fn extract_ui_exe_writes_temp_executable_and_marks_it_for_cleanup() {
        let prepared = extract_ui_exe(b"fake exe").unwrap();

        assert_eq!(fs::read(&prepared.path).unwrap(), b"fake exe");
        assert!(prepared.remove_on_drop);
        fs::remove_file(prepared.path).unwrap();
    }

    #[test]
    fn prepare_ui_exe_returns_available_helper_or_clear_missing_error() {
        match prepare_ui_exe() {
            Ok(prepared) => {
                assert!(prepared.path.is_file());
                if prepared.remove_on_drop {
                    fs::remove_file(prepared.path).unwrap();
                }
            }
            Err(err) => {
                assert!(err.to_string().contains("C# UI helper is not bundled"));
                assert!(err.to_string().contains(UI_EXE_NAME));
            }
        }
    }

    #[test]
    fn availability_and_suffix_helpers_are_callable_without_side_effect_requirements() {
        let _ = is_available();
        assert!(unique_suffix() > 0);
    }
}
