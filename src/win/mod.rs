use crate::{AppError, Logger, RegistryRoot};
use serde_json::json;
use std::ffi::{OsStr, c_void};
use std::fs;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, HANDLE, HWND, WIN32_ERROR,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{
    CopyFile2, CreateDirectoryW, DeleteFileW, MOVE_FILE_FLAGS, MOVEFILE_DELAY_UNTIL_REBOOT,
    MoveFileExW, RemoveDirectoryW,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile,
};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WOW64_64KEY,
    REG_OPEN_CREATE_OPTIONS, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::System::RestartManager::{
    RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources, RmStartSession,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_ProgramFilesX64, FOLDERID_ProgramFilesX86,
    FOLDERID_Windows, IShellLinkW, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath, ShellExecuteW,
    ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
use windows::core::{Interface, PCWSTR, PWSTR, w};

mod elevation;
mod fs_ops;
mod path;
mod pipe;
mod registry;
mod restart_manager;
mod shortcut;

pub(crate) use elevation::*;
pub(crate) use fs_ops::*;
pub(crate) use path::*;
pub(crate) use pipe::*;
pub(crate) use registry::*;
pub(crate) use restart_manager::*;
pub(crate) use shortcut::*;
fn close_handle(handle: HANDLE, logger: &Logger) -> Result<(), AppError> {
    logger.unsafe_enter("CloseHandle", json!({}));
    let result = unsafe { CloseHandle(handle) };
    logger.unsafe_exit("CloseHandle", json!({"ok":result.is_ok()}));
    result?;
    Ok(())
}
fn win32_ok(status: WIN32_ERROR, operation: &str) -> Result<(), AppError> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(AppError::Message(format!(
            "{operation} failed with Win32 error {}",
            status.0
        )))
    }
}

struct Utf16Arg {
    inner: Vec<u16>,
}

impl Utf16Arg {
    fn from_path(path: &Path) -> Self {
        Self {
            inner: path
                .as_os_str()
                .encode_wide()
                .chain(iter::once(0))
                .collect(),
        }
    }

    fn from_str(value: &str) -> Self {
        Self {
            inner: OsStr::new(value)
                .encode_wide()
                .chain(iter::once(0))
                .collect(),
        }
    }

    fn as_ptr(&self) -> *const u16 {
        self.inner.as_ptr()
    }

    fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.inner.as_ptr().cast::<u8>(),
                self.inner.len() * std::mem::size_of::<u16>(),
            )
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "covenant-setup-win-test-{name}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn quiet_logger() -> Logger {
        Logger {
            json: false,
            quiet: true,
        }
    }

    fn resolver() -> PathResolver {
        PathResolver::with_roots_for_test(vec![
            PathBuf::from("C:\\Program Files"),
            PathBuf::from("C:\\Program Files (x86)"),
            PathBuf::from("D:\\Windows"),
        ])
    }

    #[test]
    fn resolve_replaces_every_known_folder_token() {
        let r = PathResolver {
            program_files_x64: PathBuf::from("C:\\Program Files"),
            program_files_x86: PathBuf::from("C:\\Program Files (x86)"),
            local_app_data: PathBuf::from("C:\\Users\\alice\\AppData\\Local"),
            desktop: PathBuf::from("C:\\Users\\alice\\Desktop"),
            admin_roots: Vec::new(),
        };
        assert_eq!(
            r.resolve("{ProgramFilesX64}\\App\\bin"),
            PathBuf::from("C:\\Program Files\\App\\bin")
        );
        assert_eq!(
            r.resolve("{ProgramFilesX86}\\Vendor\\app.exe"),
            PathBuf::from("C:\\Program Files (x86)\\Vendor\\app.exe")
        );
        assert_eq!(
            r.resolve("{LocalAppData}\\Vendor"),
            PathBuf::from("C:\\Users\\alice\\AppData\\Local\\Vendor")
        );
        assert_eq!(
            r.resolve("{Desktop}\\App.lnk"),
            PathBuf::from("C:\\Users\\alice\\Desktop\\App.lnk")
        );
    }

    #[test]
    fn requires_admin_matches_subpaths() {
        let r = resolver();
        assert!(r.requires_admin(Path::new("C:\\Program Files\\App\\bin")));
        assert!(r.requires_admin(Path::new("C:\\Program Files (x86)\\Vendor\\app.exe")));
        assert!(r.requires_admin(Path::new("D:\\Windows\\System32\\drivers")));
    }

    #[test]
    fn requires_admin_matches_exact_root() {
        let r = resolver();
        assert!(r.requires_admin(Path::new("C:\\Program Files")));
        assert!(r.requires_admin(Path::new("C:\\Program Files\\")));
    }

    #[test]
    fn requires_admin_rejects_sibling_prefix() {
        let r = resolver();
        assert!(!r.requires_admin(Path::new("C:\\Program Files Custom\\App")));
        assert!(!r.requires_admin(Path::new("C:\\Program Files2\\App")));
        assert!(!r.requires_admin(Path::new("D:\\WindowsApps\\thing")));
    }

    #[test]
    fn requires_admin_rejects_user_paths() {
        let r = resolver();
        assert!(!r.requires_admin(Path::new("C:\\Users\\alice\\AppData\\Local\\App")));
        assert!(!r.requires_admin(Path::new("D:\\data\\App")));
        assert!(!r.requires_admin(Path::new("E:\\")));
    }

    #[test]
    fn requires_admin_is_case_insensitive() {
        let r = resolver();
        assert!(r.requires_admin(Path::new("c:\\PROGRAM FILES\\App")));
        assert!(r.requires_admin(Path::new("D:\\windows\\System32")));
    }

    #[test]
    fn requires_admin_normalizes_forward_slashes() {
        let r = resolver();
        assert!(r.requires_admin(Path::new("C:/Program Files/App/bin")));
        assert!(r.requires_admin(Path::new("D:/Windows/System32")));
    }

    #[test]
    fn requires_admin_handles_non_windows_roots() {
        let r = PathResolver::with_roots_for_test(vec![PathBuf::from("E:\\Apps\\Program Files")]);
        assert!(r.requires_admin(Path::new("E:\\Apps\\Program Files\\App")));
        assert!(!r.requires_admin(Path::new("C:\\Program Files\\App")));
    }

    #[test]
    fn requires_admin_with_empty_roots_returns_false() {
        let r = PathResolver::with_roots_for_test(vec![]);
        assert!(!r.requires_admin(Path::new("C:\\Program Files\\App")));
    }

    #[test]
    fn create_directory_recursive_creates_nested_directories_and_noops_existing() {
        let temp = TestDir::new("create-dir");
        let nested = temp.path.join("one").join("two").join("three");

        create_directory_recursive(&nested, &quiet_logger()).unwrap();
        create_directory_recursive(&nested, &quiet_logger()).unwrap();

        assert!(nested.is_dir());
    }

    #[test]
    fn copy_file_copies_bytes_to_destination() {
        let temp = TestDir::new("copy-file");
        let source = temp.path.join("source.bin");
        let destination = temp.path.join("destination.bin");
        fs::write(&source, b"copy me").unwrap();

        copy_file(&source, &destination, &quiet_logger()).unwrap();

        assert_eq!(fs::read(destination).unwrap(), b"copy me");
    }

    #[test]
    fn remove_directory_if_exists_removes_empty_and_defers_nonempty() {
        let temp = TestDir::new("remove-dir");
        let empty = temp.path.join("empty");
        let nonempty = temp.path.join("nonempty");
        fs::create_dir_all(&empty).unwrap();
        fs::create_dir_all(&nonempty).unwrap();
        fs::write(nonempty.join("child.txt"), b"child").unwrap();

        remove_directory_if_exists(&empty, &quiet_logger()).unwrap();
        remove_directory_if_exists(&nonempty, &quiet_logger()).unwrap();
        remove_directory_if_exists(&temp.path.join("missing"), &quiet_logger()).unwrap();

        assert!(!empty.exists());
        assert!(nonempty.is_dir());
    }

    #[test]
    fn remove_file_with_fallback_deletes_existing_file_and_noops_missing() {
        let temp = TestDir::new("remove-file");
        let file = temp.path.join("payload.bin");
        fs::write(&file, b"delete me").unwrap();

        remove_file_with_fallback(&file, &quiet_logger()).unwrap();
        remove_file_with_fallback(&file, &quiet_logger()).unwrap();

        assert!(!file.exists());
    }

    #[test]
    fn is_elevated_queries_current_process_token() {
        let _ = is_elevated(&quiet_logger()).unwrap();
    }

    #[test]
    fn delete_registry_tree_ignores_unique_missing_hkcu_key() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let subkey = format!(
            "Software\\CovenantSetupTests\\missing-{}-{unique}",
            std::process::id()
        );

        delete_registry_tree(RegistryRoot::Hkcu, &subkey, &quiet_logger()).unwrap();
    }

    #[test]
    fn create_shortcut_writes_lnk_file_with_optional_fields() {
        let temp = TestDir::new("shortcut");
        let shortcut = temp.path.join("sample.lnk");
        let target = std::env::current_exe().unwrap();
        let working_directory = target.parent().unwrap();

        create_shortcut(
            &shortcut,
            &target,
            Some("--help"),
            Some(working_directory),
            Some("Sample shortcut"),
            &quiet_logger(),
        )
        .unwrap();

        assert!(shortcut.is_file());
    }

    #[test]
    fn restart_manager_reports_locking_processes_for_unlocked_file() {
        let temp = TestDir::new("restart-manager");
        let file = temp.path.join("unlocked.txt");
        fs::write(&file, b"unlocked").unwrap();

        match get_locking_processes(&file, &quiet_logger()) {
            Ok(pids) => assert!(pids.iter().all(|pid| *pid > 0)),
            Err(err) => assert!(err.to_string().contains("RmStartSession failed")),
        }
    }

    #[test]
    fn pwstr_to_path_decodes_valid_utf16_and_rejects_invalid_utf16() {
        let mut valid = Utf16Arg::from_str("C:\\Temp").inner;
        let path = pwstr_to_path(PWSTR(valid.as_mut_ptr()), &quiet_logger()).unwrap();
        assert_eq!(path, PathBuf::from("C:\\Temp"));

        let mut invalid = vec![0xD800, 0];
        let err = pwstr_to_path(PWSTR(invalid.as_mut_ptr()), &quiet_logger())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Invalid UTF-16"));
    }

    #[test]
    fn win32_ok_accepts_success_and_formats_errors() {
        win32_ok(ERROR_SUCCESS, "Example").unwrap();

        let err = win32_ok(WIN32_ERROR(5), "Example").unwrap_err().to_string();
        assert_eq!(err, "Example failed with Win32 error 5");
    }

    #[test]
    fn quote_passthrough_when_no_special_chars() {
        assert_eq!(quote_command_line_arg("install"), "install");
        assert_eq!(
            quote_command_line_arg("C:\\Apps\\foo.exe"),
            "C:\\Apps\\foo.exe"
        );
        assert_eq!(quote_command_line_arg("--json"), "--json");
    }

    #[test]
    fn quote_wraps_when_contains_space_or_tab() {
        assert_eq!(quote_command_line_arg("hello world"), "\"hello world\"");
        assert_eq!(quote_command_line_arg("a\tb"), "\"a\tb\"");
        assert_eq!(
            quote_command_line_arg("C:\\Program Files\\App\\install.toml"),
            "\"C:\\Program Files\\App\\install.toml\""
        );
    }

    #[test]
    fn quote_escapes_embedded_double_quotes() {
        assert_eq!(quote_command_line_arg("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_command_line_arg("\""), "\"\\\"\"");
    }

    #[test]
    fn quote_doubles_trailing_backslashes_before_closing_quote() {
        // "C:\Path\" must serialize as "\"C:\\Path\\\\\"" so the parser sees
        // the backslashes as literals and the final quote as the terminator.
        assert_eq!(
            quote_command_line_arg("C:\\Path with space\\"),
            "\"C:\\Path with space\\\\\""
        );
    }

    #[test]
    fn quote_doubles_backslashes_only_when_followed_by_quote() {
        // \\ inside an unquoted-needing arg stays \\ when not before a quote.
        assert_eq!(quote_command_line_arg("a\\\\b c"), "\"a\\\\b c\"");
        // \\ immediately before a literal quote becomes \\\\\".
        assert_eq!(quote_command_line_arg("a\\\\\"b"), "\"a\\\\\\\\\\\"b\"");
    }

    #[test]
    fn quote_emits_explicit_empty_argument() {
        assert_eq!(quote_command_line_arg(""), "\"\"");
    }

    #[test]
    fn quote_round_trips_through_argv_rules() {
        // Sanity-check that re-parsing the quoted form per the
        // CommandLineToArgvW spec recovers the original argument.
        for input in [
            "simple",
            "with space",
            "a\"b",
            "C:\\Program Files\\app\\bin",
            "C:\\Path with space\\",
            "trailing\\\\",
            "embedded\\\"quote",
            "",
        ] {
            let quoted = quote_command_line_arg(input);
            let parsed = parse_argv_for_test(&quoted);
            assert_eq!(parsed, vec![input.to_string()], "input was {input:?}");
        }
    }

    #[test]
    fn utf16_arg_as_bytes_includes_null_terminator() {
        let arg = Utf16Arg::from_str("A");
        assert_eq!(arg.inner, vec![65, 0]);
        assert_eq!(
            arg.as_bytes().len(),
            arg.inner.len() * std::mem::size_of::<u16>()
        );
        assert_eq!(arg.as_bytes(), &[65, 0, 0, 0]);
    }

    // Reference parser following the CommandLineToArgvW algorithm, used only
    // to validate the encoder above.
    fn parse_argv_for_test(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut backslashes = 0usize;
        let mut started = false;

        let flush_backslashes = |current: &mut String, n: usize| {
            for _ in 0..n {
                current.push('\\');
            }
        };

        for c in line.chars() {
            match c {
                '\\' => {
                    backslashes += 1;
                    started = true;
                }
                '"' => {
                    flush_backslashes(&mut current, backslashes / 2);
                    if backslashes % 2 == 1 {
                        current.push('"');
                    } else {
                        in_quotes = !in_quotes;
                    }
                    backslashes = 0;
                    started = true;
                }
                ' ' | '\t' if !in_quotes => {
                    flush_backslashes(&mut current, backslashes);
                    backslashes = 0;
                    if started {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                _ => {
                    flush_backslashes(&mut current, backslashes);
                    backslashes = 0;
                    current.push(c);
                    started = true;
                }
            }
        }
        flush_backslashes(&mut current, backslashes);
        if started {
            args.push(current);
        }
        args
    }
}
