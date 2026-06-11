use crate::{AppError, Logger, win};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{self, Child, Command};
use std::thread;
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const UI_EXE_NAME: &str = "Covenant.Setup.Ui.exe";
// Must match the bundle format written by write_ui_bundle in build.rs.
const UI_BUNDLE_MAGIC: &[u8] = b"COVENANT_SETUP_UI_BUNDLE_V1\n";

#[cfg(covenant_setup_embedded_ui)]
fn embedded_ui_bundle_bytes() -> Option<&'static [u8]> {
    Some(include_bytes!(env!("COVENANT_SETUP_UI_BUNDLE")))
}

#[cfg(not(covenant_setup_embedded_ui))]
fn embedded_ui_bundle_bytes() -> Option<&'static [u8]> {
    None
}

pub fn is_available() -> bool {
    embedded_ui_bundle_bytes().is_some() || sidecar_ui_exe().is_some()
}

/// Trait abstraction over the live GUI progress IPC channel so install /
/// uninstall code can be unit-tested with a recording mock instead of
/// spawning the real C# UI.
pub trait ProgressSink: Send {
    fn advance(
        &mut self,
        current_step: usize,
        total_steps: usize,
        message: &str,
    ) -> Result<(), AppError>;
    fn log(&mut self, message: &str) -> Result<(), AppError>;
    fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError>;
    fn fail(
        &mut self,
        app_name: &str,
        operation: &str,
        message: &str,
        error: &str,
        errata: Value,
        wait_for_close: bool,
    ) -> Result<(), AppError>;
    fn cancel_requested(&mut self) -> bool {
        false
    }
    fn clear_cancel_request(&mut self) {}
}

impl ProgressSink for GuiProgress {
    fn advance(
        &mut self,
        current_step: usize,
        total_steps: usize,
        message: &str,
    ) -> Result<(), AppError> {
        GuiProgress::advance(self, current_step, total_steps, message)
    }

    fn log(&mut self, message: &str) -> Result<(), AppError> {
        GuiProgress::log(self, message)
    }

    fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError> {
        GuiProgress::finish(self, message, wait_for_close)
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

    fn cancel_requested(&mut self) -> bool {
        if !self.cancel_seen {
            self.cancel_seen = self.session.poll_cancel_request(&self.logger);
        }
        self.cancel_seen
    }

    fn clear_cancel_request(&mut self) {
        self.cancel_seen = false;
    }
}

pub struct GuiProgress {
    session: CSharpUiSession,
    logger: Logger,
    cancel_seen: bool,
}

impl GuiProgress {
    pub fn start(
        title: &str,
        app_name: Option<&str>,
        install_root: Option<&str>,
        total_steps: usize,
        automation: bool,
        logger: &Logger,
    ) -> Result<Self, AppError> {
        let mut session = CSharpUiSession::start()?;

        let branding_image = if let (Some(_), Some(root_path)) = (app_name, install_root) {
            let root = Path::new(root_path);
            ["branding.png", "banner.png", "logo.png"]
                .iter()
                .map(|name| root.join(name))
                .find(|path| path.is_file())
                .map(|path| path.to_string_lossy().into_owned())
        } else {
            None
        };

        session.send(&json!({
            "type": "init",
            "title": title,
            "app_name": app_name,
            "install_dir": install_root,
            "branding_image": branding_image,
            "total_steps": total_steps.max(1),
            "automation": automation,
        }))?;

        #[derive(Deserialize)]
        struct WelcomeResponse {
            #[serde(rename = "type")]
            message_type: String,
            result: Option<String>,
        }

        if app_name.is_some() && install_root.is_some() && !automation {
            let response: WelcomeResponse = session.read()?;
            if response.message_type != "welcome_response"
                || response.result.as_deref() != Some("install")
            {
                return Err(AppError::CancelledByUser);
            }
        }

        Ok(Self {
            session,
            logger: logger.clone(),
            cancel_seen: false,
        })
    }

    pub fn advance(
        &mut self,
        current_step: usize,
        total_steps: usize,
        message: &str,
    ) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "progress",
            "current_step": current_step,
            "total_steps": total_steps.max(1),
            "message": message,
        }))
    }

    pub fn log(&mut self, message: &str) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "log",
            "message": message,
        }))
    }

    pub fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError> {
        self.session.send(&json!({
            "type": "finish",
            "message": message,
        }))?;
        if wait_for_close {
            self.session.wait_for_exit()?;
        }
        Ok(())
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
    YesNo,
}

impl PromptButtons {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
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
    pending_input: Vec<u8>,
    cleanup_root: Option<PathBuf>,
    closed: bool,
    child_exited: bool,
}

impl CSharpUiSession {
    fn start() -> Result<Self, AppError> {
        let pipe_name = format!("covenant-setup-ui-{}-{}", process::id(), unique_suffix());
        crate::trace_event("ui_start", json!({"pipe_name": pipe_name}));
        let PreparedUiExe {
            path: exe_path,
            cleanup_root,
        } = prepare_ui_exe()?;
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
            pending_input: Vec::new(),
            cleanup_root,
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

    /// Drains any complete UI->engine lines and reports whether one was a
    /// `cancel_request`. Never blocks: bytes are only pulled after
    /// PeekNamedPipe says they are available, because a read left pending on
    /// this synchronous pipe handle would serialize against (and stall) the
    /// engine's progress writes on the same handle.
    fn poll_cancel_request(&mut self, logger: &Logger) -> bool {
        let mut saw_cancel = false;
        loop {
            let buffered = self.reader.buffer().len();
            if buffered > 0 {
                // Leftover bytes the welcome-handshake read pulled in.
                let chunk = self.reader.buffer().to_vec();
                self.reader.consume(buffered);
                self.pending_input.extend_from_slice(&chunk);
            } else {
                let available = win::peek_named_pipe_available(self.reader.get_ref(), logger)
                    .unwrap_or(0) as usize;
                if available == 0 {
                    break;
                }
                let mut chunk = vec![0u8; available];
                match self.reader.get_mut().read(&mut chunk) {
                    Ok(count) if count > 0 => self.pending_input.extend_from_slice(&chunk[..count]),
                    _ => break,
                }
            }

            while let Some(newline) = self.pending_input.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.pending_input.drain(..=newline).collect();
                let line = String::from_utf8_lossy(&line);
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && value.get("type").and_then(Value::as_str) == Some("cancel_request")
                {
                    crate::trace_event("ui_cancel_request", json!({}));
                    saw_cancel = true;
                }
            }
        }
        saw_cancel
    }
}

impl Drop for CSharpUiSession {
    fn drop(&mut self) {
        if self.child_exited {
            self.cleanup_ui_files();
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
                self.cleanup_ui_files();
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        crate::trace_event("ui_killed", json!({"pid": self.child.id()}));
        self.cleanup_ui_files();
    }
}

impl CSharpUiSession {
    fn cleanup_ui_files(&self) {
        if let Some(root) = &self.cleanup_root {
            let _ = fs::remove_dir_all(root);
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
    cleanup_root: Option<PathBuf>,
}

fn prepare_ui_exe() -> Result<PreparedUiExe, AppError> {
    if let Some(bytes) = embedded_ui_bundle_bytes() {
        return extract_ui_bundle(bytes);
    }
    if let Some(path) = sidecar_ui_exe() {
        return Ok(PreparedUiExe {
            path,
            cleanup_root: None,
        });
    }
    Err(AppError::Message(format!(
        "C# UI helper is not bundled and no {UI_EXE_NAME} was found next to the installer"
    )))
}

fn extract_ui_bundle(bytes: &[u8]) -> Result<PreparedUiExe, AppError> {
    if !bytes.starts_with(UI_BUNDLE_MAGIC) {
        return Err(AppError::Message("Embedded C# UI bundle is invalid".into()));
    }

    let root = std::env::temp_dir().join(format!(
        "covenant-setup-ui-{}-{}",
        process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&root)?;
    let mut cursor = UI_BUNDLE_MAGIC.len();

    loop {
        let path_len = read_bundle_u32(bytes, &mut cursor)? as usize;
        let data_len = usize::try_from(read_bundle_u64(bytes, &mut cursor)?)
            .map_err(|_| AppError::Message("Embedded C# UI file is too large".into()))?;
        if path_len == 0 {
            if data_len != 0 {
                return Err(AppError::Message(
                    "Embedded C# UI bundle has an invalid terminator".into(),
                ));
            }
            break;
        }

        let relative_path = std::str::from_utf8(take_bundle_bytes(bytes, &mut cursor, path_len)?)
            .map_err(|_| {
            AppError::Message("Embedded C# UI bundle path is not valid UTF-8".into())
        })?;
        let data = take_bundle_bytes(bytes, &mut cursor, data_len)?;

        let output_path = bundle_output_path(&root, relative_path)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_path, data)?;
    }

    let path = root.join(UI_EXE_NAME);
    if !path.is_file() {
        let _ = fs::remove_dir_all(&root);
        return Err(AppError::Message(format!(
            "Embedded C# UI bundle did not contain {UI_EXE_NAME}"
        )));
    }

    Ok(PreparedUiExe {
        path,
        cleanup_root: Some(root),
    })
}

fn sidecar_ui_exe() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(UI_EXE_NAME);
    path.is_file().then_some(path)
}

fn read_bundle_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, AppError> {
    Ok(u32::from_le_bytes(
        take_bundle_bytes(bytes, cursor, 4)?.try_into().unwrap(),
    ))
}

fn read_bundle_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, AppError> {
    Ok(u64::from_le_bytes(
        take_bundle_bytes(bytes, cursor, 8)?.try_into().unwrap(),
    ))
}

/// Advances `cursor` past the next `len` bytes and returns them, or errors if
/// the bundle is truncated.
fn take_bundle_bytes<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    len: usize,
) -> Result<&'a [u8], AppError> {
    let end = cursor
        .checked_add(len)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| AppError::Message("Embedded C# UI bundle ended unexpectedly".into()))?;
    let slice = &bytes[*cursor..end];
    *cursor = end;
    Ok(slice)
}

fn bundle_output_path(root: &Path, relative_path: &str) -> Result<PathBuf, AppError> {
    let relative_path = Path::new(relative_path);
    if relative_path.is_absolute() {
        return Err(AppError::Message(
            "Embedded C# UI bundle contains an absolute path".into(),
        ));
    }

    let mut output_path = root.to_path_buf();
    for component in relative_path.components() {
        match component {
            Component::Normal(part) => output_path.push(part),
            _ => {
                return Err(AppError::Message(
                    "Embedded C# UI bundle contains an unsafe path".into(),
                ));
            }
        }
    }
    Ok(output_path)
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
    fn extract_ui_bundle_writes_temp_application_folder_and_marks_it_for_cleanup() {
        let bundle = make_ui_bundle(&[
            (UI_EXE_NAME, b"fake exe".as_slice()),
            ("runtimes/win-x64/native/helper.dll", b"dll".as_slice()),
        ]);
        let prepared = extract_ui_bundle(&bundle).unwrap();

        assert_eq!(fs::read(&prepared.path).unwrap(), b"fake exe");
        let cleanup_root = prepared.cleanup_root.clone().unwrap();
        assert!(
            cleanup_root
                .join("runtimes/win-x64/native/helper.dll")
                .is_file()
        );
        fs::remove_dir_all(cleanup_root).unwrap();
    }

    #[test]
    fn prepare_ui_exe_returns_available_helper_or_clear_missing_error() {
        match prepare_ui_exe() {
            Ok(prepared) => {
                assert!(prepared.path.is_file());
                if let Some(cleanup_root) = prepared.cleanup_root {
                    fs::remove_dir_all(cleanup_root).unwrap();
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

    fn make_ui_bundle(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bundle = Vec::new();
        bundle.extend_from_slice(UI_BUNDLE_MAGIC);
        for (relative_path, data) in files {
            bundle.extend_from_slice(&(relative_path.len() as u32).to_le_bytes());
            bundle.extend_from_slice(&(data.len() as u64).to_le_bytes());
            bundle.extend_from_slice(relative_path.as_bytes());
            bundle.extend_from_slice(data);
        }
        bundle.extend_from_slice(&0u32.to_le_bytes());
        bundle.extend_from_slice(&0u64.to_le_bytes());
        bundle
    }
}
