use crate::{AppError, Logger};
use serde_json::json;
use std::fs;
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(crate) fn unique_ticks() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub(crate) fn powershell_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

pub(crate) fn spawn_reboot(logger: &Logger) -> Result<(), AppError> {
    logger.info("spawn_reboot", json!({}));
    let mut command = Command::new("shutdown.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    command.args(["/r", "/t", "0"]);
    command.spawn()?;
    Ok(())
}

pub(crate) fn prompt_reboot_tui() -> Result<bool, AppError> {
    print!("Restart now? [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

pub(crate) fn sanitize_registry_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "covenant_setup".to_string()
    } else {
        sanitized
    }
}

pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    if let (Ok(left), Ok(right)) = (fs::canonicalize(left), fs::canonicalize(right)) {
        return normalize_path_for_compare(&left) == normalize_path_for_compare(&right);
    }
    normalize_path_for_compare(left) == normalize_path_for_compare(right)
}

pub(crate) fn normalize_path_for_compare(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('/', "\\");
    if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
        value = format!("\\\\{rest}");
    } else if let Some(rest) = value.strip_prefix("\\\\?\\") {
        value = rest.to_string();
    }
    while value.ends_with('\\') && !is_windows_root(&value) {
        value.pop();
    }
    value.to_ascii_lowercase()
}

pub(crate) fn is_windows_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() == 3 && bytes[1] == b':' && bytes[2] == b'\\')
        || (path.starts_with("\\\\") && path[2..].matches('\\').count() <= 1)
}

pub(crate) fn absolutize(base: Option<&Path>, value: &str) -> PathBuf {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        candidate
    } else {
        base.unwrap_or_else(|| Path::new(".")).join(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn sanitize_registry_component_replaces_punctuation_and_defaults_empty_input() {
        assert_eq!(sanitize_registry_component(""), "covenant_setup");
        assert_eq!(
            sanitize_registry_component("Vendor App: 1.0/alpha"),
            "Vendor_App__1_0_alpha"
        );
        assert_eq!(sanitize_registry_component("AZaz09-_"), "AZaz09-_");
    }

    #[test]
    fn normalize_path_for_compare_handles_case_slashes_and_verbatim_prefixes() {
        assert_eq!(
            normalize_path_for_compare(Path::new("C:/Apps/Example/")),
            "c:\\apps\\example"
        );
        assert_eq!(
            normalize_path_for_compare(Path::new(r"\\?\C:\Apps\Example")),
            "c:\\apps\\example"
        );
        assert_eq!(
            normalize_path_for_compare(Path::new(r"\\?\UNC\server\share\Example")),
            r"\\server\share\example"
        );
    }

    #[test]
    fn path_root_and_absolutize_helpers_handle_expected_shapes() {
        assert!(is_windows_root("C:\\"));
        assert!(is_windows_root("\\\\server\\share"));
        assert!(!is_windows_root("C:\\Apps"));
        assert_eq!(
            absolutize(Some(Path::new("C:\\Base")), "relative\\file.txt"),
            PathBuf::from("C:\\Base\\relative\\file.txt")
        );
        assert_eq!(
            absolutize(Some(Path::new("C:\\Base")), "D:\\absolute\\file.txt"),
            PathBuf::from("D:\\absolute\\file.txt")
        );
    }

    #[test]
    fn same_path_uses_normalized_fallback_for_missing_paths() {
        assert!(same_path(
            Path::new("C:/Missing/Example/"),
            Path::new(r"\\?\C:\Missing\Example")
        ));
        assert!(!same_path(
            Path::new("C:/Missing/Example"),
            Path::new("C:/Missing/Other")
        ));
    }

    #[test]
    fn same_path_uses_canonicalized_existing_paths() {
        let temp = TestDir::new("same-path");
        let nested = temp.path().join("nested");
        let file = nested.join("payload.txt");
        fs::create_dir_all(&nested).unwrap();
        fs::write(&file, b"payload").unwrap();

        assert!(same_path(&file, &nested.join(".").join("payload.txt")));
    }

    #[test]
    fn powershell_single_quote_doubles_embedded_quotes() {
        assert_eq!(
            powershell_single_quote("C:\\Alice's App"),
            "C:\\Alice''s App"
        );
    }

    #[test]
    fn unique_ticks_returns_nonzero_timestamp() {
        assert!(unique_ticks() > 0);
    }
}
