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

pub struct PathResolver {
    pub program_files_x64: PathBuf,
    pub local_app_data: PathBuf,
    pub desktop: PathBuf,
    admin_roots: Vec<String>,
}

impl PathResolver {
    pub fn new(logger: &Logger) -> Result<Self, AppError> {
        let program_files_x64 = known_folder(&FOLDERID_ProgramFilesX64, logger)?;
        let program_files_x86 = known_folder(&FOLDERID_ProgramFilesX86, logger)?;
        let windows_dir = known_folder(&FOLDERID_Windows, logger)?;
        let admin_roots =
            build_admin_roots(&[&program_files_x64, &program_files_x86, &windows_dir]);
        Ok(Self {
            program_files_x64,
            local_app_data: known_folder(&FOLDERID_LocalAppData, logger)?,
            desktop: known_folder(&FOLDERID_Desktop, logger)?,
            admin_roots,
        })
    }

    pub fn resolve(&self, input: &str) -> PathBuf {
        PathBuf::from(
            input
                .replace(
                    "{ProgramFilesX64}",
                    &self.program_files_x64.to_string_lossy(),
                )
                .replace("{LocalAppData}", &self.local_app_data.to_string_lossy())
                .replace("{Desktop}", &self.desktop.to_string_lossy()),
        )
    }

    pub fn requires_admin(&self, path: &Path) -> bool {
        let candidate = normalize_for_admin_match(path);
        self.admin_roots
            .iter()
            .any(|root| candidate == *root || candidate.starts_with(&format!("{root}\\")))
    }

    #[cfg(test)]
    pub(crate) fn with_roots_for_test(roots: Vec<PathBuf>) -> Self {
        let admin_roots = build_admin_roots(&roots.iter().collect::<Vec<_>>());
        Self {
            program_files_x64: PathBuf::new(),
            local_app_data: PathBuf::new(),
            desktop: PathBuf::new(),
            admin_roots,
        }
    }
}

fn build_admin_roots(roots: &[&PathBuf]) -> Vec<String> {
    roots
        .iter()
        .map(|p| {
            let lower = p.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
            lower.trim_end_matches('\\').to_string()
        })
        .filter(|root| !root.is_empty())
        .collect()
}

fn normalize_for_admin_match(path: &Path) -> String {
    let lower = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    lower.trim_end_matches('\\').to_string()
}

// Encodes a single argument for a Windows command line that will be parsed by
// CommandLineToArgvW (which is what ShellExecuteW's lpParameters feeds into,
// and what every standard Win32 process startup uses to populate argv).
//
// Rules: quote if empty or contains space/tab/quote; inside quotes, escape `"`
// as `\"` and double any run of backslashes that immediately precedes a quote
// or the closing quote.
fn quote_command_line_arg(arg: &str) -> String {
    let needs_quoting = arg.is_empty() || arg.chars().any(|c| c == ' ' || c == '\t' || c == '"');
    if !needs_quoting {
        return arg.to_string();
    }

    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..(backslashes * 2 + 1) {
                    out.push('\\');
                }
                out.push('"');
                backslashes = 0;
            }
            _ => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                out.push(c);
                backslashes = 0;
            }
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
    out
}

pub fn is_elevated(logger: &Logger) -> Result<bool, AppError> {
    let mut token = HANDLE::default();
    logger.unsafe_enter("OpenProcessToken", json!({}));
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    logger.unsafe_exit("OpenProcessToken", json!({"opened": !token.is_invalid()}));

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    logger.unsafe_enter("GetTokenInformation", json!({"class":"TokenElevation"}));
    unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )?
    };
    logger.unsafe_exit("GetTokenInformation", json!({"returned": returned}));
    close_handle(token, logger)?;
    if returned < std::mem::size_of::<TOKEN_ELEVATION>() as u32 {
        return Err(AppError::Message("Short TOKEN_ELEVATION payload".into()));
    }
    Ok(elevation.TokenIsElevated != 0)
}

pub fn relaunch_as_admin(logger: &Logger) -> Result<(), AppError> {
    let exe = std::env::current_exe()?;
    let params = std::env::args()
        .skip(1)
        .map(|arg| quote_command_line_arg(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    logger.unsafe_enter(
        "ShellExecuteW",
        json!({"verb":"runas","exe":exe,"params":params}),
    );
    let result = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            w!("runas"),
            PCWSTR(Utf16Arg::from_path(&exe).as_ptr()),
            PCWSTR(Utf16Arg::from_str(&params).as_ptr()),
            PCWSTR::null(),
            SW_SHOW,
        )
    };
    let code = result.0 as isize;
    logger.unsafe_exit("ShellExecuteW", json!({"hinstance": code}));
    if code <= 32 {
        return Err(AppError::Message(format!("ShellExecuteW failed: {code}")));
    }
    Ok(())
}

pub fn create_directory_recursive(path: &Path, logger: &Logger) -> Result<(), AppError> {
    if path.as_os_str().is_empty() || path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if parent != path {
            create_directory_recursive(parent, logger)?;
        }
    }
    logger.unsafe_enter("CreateDirectoryW", json!({"path": path}));
    let result = unsafe { CreateDirectoryW(PCWSTR(Utf16Arg::from_path(path).as_ptr()), None) };
    logger.unsafe_exit("CreateDirectoryW", json!({"ok": result.is_ok()}));
    if let Err(err) = result {
        if !path.exists() {
            return Err(err.into());
        }
    }
    Ok(())
}

pub fn copy_file(source: &Path, destination: &Path, logger: &Logger) -> Result<(), AppError> {
    let source_w = Utf16Arg::from_path(source);
    let dest_w = Utf16Arg::from_path(destination);
    logger.unsafe_enter(
        "CopyFile2",
        json!({"source":source,"destination":destination}),
    );
    let result = unsafe { CopyFile2(PCWSTR(source_w.as_ptr()), PCWSTR(dest_w.as_ptr()), None) };
    logger.unsafe_exit("CopyFile2", json!({"ok": result.is_ok()}));
    result?;
    Ok(())
}

pub fn remove_directory_if_exists(path: &Path, logger: &Logger) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    logger.unsafe_enter("RemoveDirectoryW", json!({"path": path}));
    let result = unsafe { RemoveDirectoryW(PCWSTR(Utf16Arg::from_path(path).as_ptr())) };
    logger.unsafe_exit("RemoveDirectoryW", json!({"ok": result.is_ok()}));
    if let Err(err) = result {
        if path.is_dir() && fs::read_dir(path)?.next().is_some() {
            logger.info(
                "remove_directory_deferred",
                json!({"path":path,"reason":"not_empty"}),
            );
            return Ok(());
        }
        if path.exists() {
            return Err(err.into());
        }
    }
    Ok(())
}

pub fn remove_file_with_fallback(path: &Path, logger: &Logger) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    logger.unsafe_enter("DeleteFileW", json!({"path": path}));
    let delete_result = unsafe { DeleteFileW(PCWSTR(Utf16Arg::from_path(path).as_ptr())) };
    logger.unsafe_exit("DeleteFileW", json!({"ok": delete_result.is_ok()}));
    if delete_result.is_ok() {
        return Ok(());
    }
    let pids = get_locking_processes(path, logger).unwrap_or_default();
    if !pids.is_empty() {
        logger.info("locked_file", json!({"path":path,"processes":pids}));
    }
    logger.unsafe_enter("MoveFileExW", json!({"path": path}));
    let move_result = unsafe {
        MoveFileExW(
            PCWSTR(Utf16Arg::from_path(path).as_ptr()),
            PCWSTR::null(),
            MOVE_FILE_FLAGS(MOVEFILE_DELAY_UNTIL_REBOOT.0),
        )
    };
    logger.unsafe_exit("MoveFileExW", json!({"ok": move_result.is_ok()}));
    move_result?;
    Ok(())
}

pub fn set_registry_string(
    root: RegistryRoot,
    subkey: &str,
    name: &str,
    value: &str,
    logger: &Logger,
) -> Result<(), AppError> {
    let mut key = HKEY::default();
    logger.unsafe_enter("RegCreateKeyExW", json!({"root":root,"subkey":subkey}));
    let create_result = unsafe {
        RegCreateKeyExW(
            root_hkey(root),
            PCWSTR(Utf16Arg::from_str(subkey).as_ptr()),
            Some(0),
            PWSTR::null(),
            REG_OPEN_CREATE_OPTIONS(REG_OPTION_NON_VOLATILE.0),
            REG_SAM_FLAGS(KEY_SET_VALUE.0 | KEY_WOW64_64KEY.0),
            None,
            &mut key,
            None,
        )
    };
    logger.unsafe_exit("RegCreateKeyExW", json!({"status": create_result.0}));
    win32_ok(create_result, "RegCreateKeyExW")?;

    let utf16 = Utf16Arg::from_str(value);
    logger.unsafe_enter("RegSetValueExW", json!({"name":name}));
    let set_result = unsafe {
        RegSetValueExW(
            key,
            PCWSTR(Utf16Arg::from_str(name).as_ptr()),
            Some(0),
            REG_VALUE_TYPE(REG_SZ.0),
            Some(utf16.as_bytes()),
        )
    };
    logger.unsafe_exit("RegSetValueExW", json!({"status": set_result.0}));
    let close_result = close_registry_key(key, logger);
    win32_ok(set_result, "RegSetValueExW")?;
    close_result?;
    Ok(())
}

pub fn delete_registry_tree(
    root: RegistryRoot,
    subkey: &str,
    logger: &Logger,
) -> Result<(), AppError> {
    logger.unsafe_enter("RegDeleteTreeW", json!({"root":root,"subkey":subkey}));
    let result =
        unsafe { RegDeleteTreeW(root_hkey(root), PCWSTR(Utf16Arg::from_str(subkey).as_ptr())) };
    logger.unsafe_exit("RegDeleteTreeW", json!({"status": result.0}));
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    win32_ok(result, "RegDeleteTreeW")
}

pub fn create_shortcut(
    shortcut_path: &Path,
    target: &Path,
    arguments: Option<&str>,
    working_directory: Option<&Path>,
    description: Option<&str>,
    logger: &Logger,
) -> Result<(), AppError> {
    logger.unsafe_enter("CoInitializeEx", json!({}));
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    logger.unsafe_exit("CoInitializeEx", json!({"ok":true}));
    let result = (|| -> Result<(), AppError> {
        logger.unsafe_enter("CoCreateInstance", json!({"class":"ShellLink"}));
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
        logger.unsafe_exit("CoCreateInstance", json!({"ok":true}));

        logger.unsafe_enter("IShellLinkW::SetPath", json!({"target": target}));
        unsafe { link.SetPath(PCWSTR(Utf16Arg::from_path(target).as_ptr()))? };
        logger.unsafe_exit("IShellLinkW::SetPath", json!({"ok":true}));

        if let Some(arguments) = arguments {
            logger.unsafe_enter("IShellLinkW::SetArguments", json!({"arguments":arguments}));
            unsafe { link.SetArguments(PCWSTR(Utf16Arg::from_str(arguments).as_ptr()))? };
            logger.unsafe_exit("IShellLinkW::SetArguments", json!({"ok":true}));
        }
        if let Some(working_directory) = working_directory {
            logger.unsafe_enter(
                "IShellLinkW::SetWorkingDirectory",
                json!({"working_directory":working_directory}),
            );
            unsafe {
                link.SetWorkingDirectory(PCWSTR(Utf16Arg::from_path(working_directory).as_ptr()))?
            };
            logger.unsafe_exit("IShellLinkW::SetWorkingDirectory", json!({"ok":true}));
        }
        if let Some(description) = description {
            logger.unsafe_enter(
                "IShellLinkW::SetDescription",
                json!({"description":description}),
            );
            unsafe { link.SetDescription(PCWSTR(Utf16Arg::from_str(description).as_ptr()))? };
            logger.unsafe_exit("IShellLinkW::SetDescription", json!({"ok":true}));
        }

        logger.unsafe_enter("Interface::cast<IPersistFile>", json!({}));
        let persist: IPersistFile = link.cast()?;
        logger.unsafe_exit("Interface::cast<IPersistFile>", json!({"ok":true}));
        logger.unsafe_enter("IPersistFile::Save", json!({"path":shortcut_path}));
        unsafe { persist.Save(PCWSTR(Utf16Arg::from_path(shortcut_path).as_ptr()), true)? };
        logger.unsafe_exit("IPersistFile::Save", json!({"ok":true}));
        Ok(())
    })();
    logger.unsafe_enter("CoUninitialize", json!({}));
    unsafe { CoUninitialize() };
    logger.unsafe_exit("CoUninitialize", json!({"ok":true}));
    result
}

fn get_locking_processes(path: &Path, logger: &Logger) -> Result<Vec<u32>, AppError> {
    let mut session = 0u32;
    let mut key = [0u16; 33];
    logger.unsafe_enter("RmStartSession", json!({}));
    let start_result = unsafe { RmStartSession(&mut session, Some(0), PWSTR(key.as_mut_ptr())) };
    logger.unsafe_exit(
        "RmStartSession",
        json!({"status":start_result.0,"session":session}),
    );
    win32_ok(start_result, "RmStartSession")?;

    let file = Utf16Arg::from_path(path);
    let resources = [PCWSTR(file.as_ptr())];
    logger.unsafe_enter("RmRegisterResources", json!({"path":path}));
    let register_result = unsafe { RmRegisterResources(session, Some(&resources), None, None) };
    logger.unsafe_exit("RmRegisterResources", json!({"status":register_result.0}));
    if let Err(err) = win32_ok(register_result, "RmRegisterResources") {
        let _ = end_restart_manager_session(session, logger);
        return Err(err);
    }

    let mut needed = 0u32;
    let mut count = 0u32;
    let mut reasons = 0u32;
    logger.unsafe_enter("RmGetList", json!({"phase":"probe"}));
    let probe = unsafe { RmGetList(session, &mut needed, &mut count, None, &mut reasons) };
    logger.unsafe_exit(
        "RmGetList",
        json!({"phase":"probe","status":probe.0,"needed":needed}),
    );
    if probe != ERROR_SUCCESS && probe != ERROR_MORE_DATA {
        let _ = end_restart_manager_session(session, logger);
        return win32_ok(probe, "RmGetList").map(|_| Vec::new());
    }
    if needed == 0 {
        end_restart_manager_session(session, logger)?;
        return Ok(Vec::new());
    }

    let mut processes = vec![RM_PROCESS_INFO::default(); needed as usize];
    count = needed;
    logger.unsafe_enter(
        "RmGetList",
        json!({"phase":"fetch","capacity":processes.len()}),
    );
    let fetch = unsafe {
        RmGetList(
            session,
            &mut needed,
            &mut count,
            Some(processes.as_mut_ptr()),
            &mut reasons,
        )
    };
    logger.unsafe_exit(
        "RmGetList",
        json!({"phase":"fetch","status":fetch.0,"count":count}),
    );
    end_restart_manager_session(session, logger)?;
    win32_ok(fetch, "RmGetList")?;
    if count as usize > processes.len() {
        return Err(AppError::Message(
            "Restart Manager count exceeded allocated buffer".into(),
        ));
    }
    Ok(processes
        .into_iter()
        .take(count as usize)
        .map(|p| p.Process.dwProcessId)
        .collect())
}

fn end_restart_manager_session(session: u32, logger: &Logger) -> Result<(), AppError> {
    logger.unsafe_enter("RmEndSession", json!({"session":session}));
    let result = unsafe { RmEndSession(session) };
    logger.unsafe_exit("RmEndSession", json!({"status":result.0}));
    win32_ok(result, "RmEndSession")
}

fn known_folder(id: &windows::core::GUID, logger: &Logger) -> Result<PathBuf, AppError> {
    logger.unsafe_enter("SHGetKnownFolderPath", json!({"folder":format!("{id:?}")}));
    let raw = unsafe { SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None)? };
    logger.unsafe_exit(
        "SHGetKnownFolderPath",
        json!({"ptr_non_null":!raw.is_null()}),
    );
    if raw.is_null() {
        return Err(AppError::Message(
            "SHGetKnownFolderPath returned null".into(),
        ));
    }
    let path = pwstr_to_path(raw, logger)?;
    logger.unsafe_enter("CoTaskMemFree", json!({}));
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    logger.unsafe_exit("CoTaskMemFree", json!({"ok":true}));
    Ok(path)
}

fn pwstr_to_path(raw: PWSTR, logger: &Logger) -> Result<PathBuf, AppError> {
    logger.unsafe_enter("PWSTR decode", json!({}));
    unsafe {
        let mut len = 0usize;
        while *raw.0.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(raw.0, len);
        let path = String::from_utf16(slice)
            .map_err(|_| AppError::Message("Invalid UTF-16 from Win32".into()))?;
        logger.unsafe_exit("PWSTR decode", json!({"len":len}));
        Ok(PathBuf::from(path))
    }
}

fn close_handle(handle: HANDLE, logger: &Logger) -> Result<(), AppError> {
    logger.unsafe_enter("CloseHandle", json!({}));
    let result = unsafe { CloseHandle(handle) };
    logger.unsafe_exit("CloseHandle", json!({"ok":result.is_ok()}));
    result?;
    Ok(())
}

fn close_registry_key(key: HKEY, logger: &Logger) -> Result<(), AppError> {
    logger.unsafe_enter("RegCloseKey", json!({}));
    let result = unsafe { RegCloseKey(key) };
    logger.unsafe_exit("RegCloseKey", json!({"status":result.0}));
    win32_ok(result, "RegCloseKey")
}

fn root_hkey(root: RegistryRoot) -> HKEY {
    match root {
        RegistryRoot::Hkcu => HKEY_CURRENT_USER,
        RegistryRoot::Hklm => HKEY_LOCAL_MACHINE,
    }
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
