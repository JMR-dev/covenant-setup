use super::*;

pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub(crate) struct CSharpUiSession {
    child: Child,
    reader: BufReader<fs::File>,
    writer: fs::File,
    /// Raw bytes of a partial line pulled in by a non-blocking drain.
    pending_input: Vec<u8>,
    /// Complete non-cancel lines drained while polling that no consumer has
    /// read yet.
    queued_lines: VecDeque<String>,
    /// Set when a `cancel_request` line was seen during a drain; consumed by
    /// `poll_cancel_request`.
    cancel_pending: bool,
    cleanup_root: Option<PathBuf>,
    closed: bool,
    child_exited: bool,
}

impl CSharpUiSession {
    pub(crate) fn start() -> Result<Self, AppError> {
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
            queued_lines: VecDeque::new(),
            cancel_pending: false,
            cleanup_root,
            closed: false,
            child_exited: false,
        })
    }

    pub(crate) fn send<T: Serialize>(&mut self, value: &T) -> Result<(), AppError> {
        let value = serde_json::to_value(value)?;
        crate::trace_event("ui_pipe_send", message_summary(&value));
        let bytes = serde_json::to_vec(&value)?;
        self.writer.write_all(&bytes)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    pub(crate) fn read<T: for<'de> Deserialize<'de>>(&mut self) -> Result<T, AppError> {
        let line = match self.queued_lines.pop_front() {
            Some(line) => line,
            None => {
                // Resume any partial line a non-blocking drain left behind
                // before falling back to a blocking read.
                let mut line = String::from_utf8_lossy(&self.pending_input).into_owned();
                self.pending_input.clear();
                let bytes = self.reader.read_line(&mut line)?;
                if bytes == 0 {
                    return Err(AppError::Message("UI pipe closed before response".into()));
                }
                line
            }
        };
        let value: Value = serde_json::from_str(&line)?;
        crate::trace_event("ui_pipe_receive", message_summary(&value));
        Ok(serde_json::from_value(value)?)
    }

    pub(crate) fn wait_for_exit(&mut self) -> Result<(), AppError> {
        self.closed = true;
        let status = self.child.wait()?;
        self.child_exited = true;
        crate::trace_event(
            "ui_failure_window_closed",
            json!({"pid": self.child.id(), "status": status.code()}),
        );
        Ok(())
    }

    /// Pulls every byte currently available on the pipe without blocking,
    /// classifying each complete line exactly once: `cancel_request` lines set
    /// `cancel_pending`, every other line is queued for the next blocking
    /// `read`. Bytes are only pulled after PeekNamedPipe says they are
    /// available, because a read left pending on this synchronous pipe handle
    /// would serialize against (and stall) the engine's progress writes on the
    /// same handle.
    fn drain_available_input(&mut self, logger: &Logger) {
        loop {
            let buffered = self.reader.buffer().len();
            if buffered > 0 {
                // Leftover bytes a previous blocking read pulled in.
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
                let line = String::from_utf8_lossy(&line).into_owned();
                if is_cancel_request(&line) {
                    crate::trace_event("ui_cancel_request", json!({}));
                    self.cancel_pending = true;
                } else {
                    self.queued_lines.push_back(line);
                }
            }
        }
    }

    /// Reports whether the UI sent a `cancel_request`. Only cancel lines are
    /// consumed; every other UI->engine line stays queued for the next
    /// blocking `read` instead of being dropped.
    pub(crate) fn poll_cancel_request(&mut self, logger: &Logger) -> bool {
        self.drain_available_input(logger);
        std::mem::take(&mut self.cancel_pending)
    }
}

pub(crate) fn is_cancel_request(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .is_ok_and(|value| value.get("type").and_then(Value::as_str) == Some("cancel_request"))
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

pub(crate) fn connect_pipe(pipe_path: &str, child: &mut Child) -> Result<fs::File, AppError> {
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
pub(crate) fn message_summary(value: &Value) -> Value {
    json!({
        "type": value.get("type").and_then(Value::as_str),
        "id": value.get("id").and_then(Value::as_str),
        "message": value.get("message").and_then(Value::as_str),
    })
}

pub(crate) fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}
