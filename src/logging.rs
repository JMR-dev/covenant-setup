use serde::Serialize;
use serde_json::json;
use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process;

#[derive(Clone)]
pub(crate) struct Logger {
    pub(crate) json: bool,
    pub(crate) quiet: bool,
}

impl Logger {
    pub(crate) fn info(&self, event: &str, detail: impl Serialize) {
        if self.quiet {
            return;
        }
        if self.json {
            println!("{}", json!({"type":"event","event":event,"detail":detail}));
        } else {
            println!(
                "{event}: {}",
                serde_json::to_string(&detail).unwrap_or_default()
            );
        }
    }

    pub(crate) fn unsafe_enter(&self, operation: &str, detail: impl Serialize) {
        self.info(
            "unsafe_enter",
            json!({"operation":operation,"detail":detail}),
        );
    }

    pub(crate) fn unsafe_exit(&self, operation: &str, detail: impl Serialize) {
        self.info(
            "unsafe_exit",
            json!({"operation":operation,"detail":detail}),
        );
    }

    pub(crate) fn result(&self, status: &str, detail: impl Serialize) {
        if self.quiet {
            return;
        }
        if self.json {
            println!(
                "{}",
                json!({"type":"result","status":status,"detail":detail})
            );
        } else {
            println!(
                "{status}: {}",
                serde_json::to_string(&detail).unwrap_or_default()
            );
        }
    }

    pub(crate) fn error(&self, message: impl Display, code: i32) {
        if self.json {
            println!(
                "{}",
                json!({"type":"error","code":code,"message":message.to_string()})
            );
        } else {
            eprintln!("error[{code}]: {message}");
        }
    }

    pub(crate) fn quiet_clone(&self) -> Self {
        Self {
            json: self.json,
            quiet: true,
        }
    }
}

pub(crate) fn trace_event(phase: &str, detail: impl Serialize) {
    let Ok(root) = std::env::var("COVENANT_SETUP_TRACE_DIR") else {
        return;
    };
    if root.trim().is_empty() {
        return;
    }

    let root = PathBuf::from(root);
    if fs::create_dir_all(&root).is_err() {
        return;
    }
    let path = root.join(format!("installer-heartbeat-{}.jsonl", process::id()));
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let event = json!({
        "time_unix_ms": timestamp,
        "pid": process::id(),
        "phase": phase,
        "detail": detail
    });
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{event}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logger_methods_and_quiet_clone_are_callable() {
        for json_output in [false, true] {
            let logger = Logger {
                json: json_output,
                quiet: false,
            };
            logger.info("test_event", json!({"value": 1}));
            logger.result("ok", json!({"value": 2}));
            logger.error("boom", 7);

            let quiet = logger.quiet_clone();
            assert!(quiet.quiet);
            assert_eq!(quiet.json, json_output);
            quiet.info("hidden", json!({}));
            quiet.result("hidden", json!({}));
        }
    }
}
