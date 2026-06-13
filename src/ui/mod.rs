use crate::{AppError, Logger, win};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{self, Child, Command};
use std::thread;
use std::time::{Duration, Instant};

mod bundle;
mod progress;
mod prompts;
mod session;

pub(crate) use bundle::*;
pub(crate) use progress::*;
pub(crate) use prompts::*;
pub(crate) use session::*;
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
