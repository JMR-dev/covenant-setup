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
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WOW64_64KEY,
    REG_OPEN_CREATE_OPTIONS, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::System::RestartManager::{
    RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources, RmStartSession,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken};
use windows::Win32::UI::Controls::{
    TASKDIALOG_COMMON_BUTTON_FLAGS, TDCBF_CANCEL_BUTTON, TDCBF_NO_BUTTON, TDCBF_OK_BUTTON,
    TDCBF_YES_BUTTON, TaskDialog,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_ProgramFilesX64, IShellLinkW,
    KNOWN_FOLDER_FLAG, SHGetKnownFolderPath, ShellExecuteW, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{IDOK, IDYES, SW_SHOW};
use windows::core::{Interface, PCWSTR, PWSTR, w};

pub struct PathResolver {
    pub program_files_x64: PathBuf,
    pub local_app_data: PathBuf,
    pub desktop: PathBuf,
}

pub fn is_parent_powershell(logger: &Logger) -> Result<bool, AppError> {
    let current_pid = unsafe { GetCurrentProcessId() };
    logger.unsafe_enter("CreateToolhelp32Snapshot", json!({}));
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? };
    logger.unsafe_exit("CreateToolhelp32Snapshot", json!({"ok": true}));

    let result = (|| -> Result<bool, AppError> {
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        logger.unsafe_enter("Process32FirstW", json!({}));
        let first = unsafe { Process32FirstW(snapshot, &mut entry) };
        logger.unsafe_exit("Process32FirstW", json!({"ok": first.is_ok()}));
        if first.is_err() {
            return Ok(false);
        }

        let mut parent_pid = None;
        loop {
            if entry.th32ProcessID == current_pid {
                parent_pid = Some(entry.th32ParentProcessID);
                break;
            }
            logger.unsafe_enter("Process32NextW", json!({}));
            let next = unsafe { Process32NextW(snapshot, &mut entry) };
            logger.unsafe_exit("Process32NextW", json!({"ok": next.is_ok()}));
            if next.is_err() {
                break;
            }
        }

        let Some(parent_pid) = parent_pid else {
            return Ok(false);
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        logger.unsafe_enter("Process32FirstW", json!({"search_parent": parent_pid}));
        let first = unsafe { Process32FirstW(snapshot, &mut entry) };
        logger.unsafe_exit("Process32FirstW", json!({"ok": first.is_ok()}));
        if first.is_err() {
            return Ok(false);
        }

        loop {
            if entry.th32ProcessID == parent_pid {
                let exe = wide_array_to_string(&entry.szExeFile);
                let exe_lower = exe.to_ascii_lowercase();
                return Ok(exe_lower.contains("powershell")
                    || exe_lower == "pwsh.exe"
                    || exe_lower == "pwsh");
            }
            logger.unsafe_enter("Process32NextW", json!({"search_parent": parent_pid}));
            let next = unsafe { Process32NextW(snapshot, &mut entry) };
            logger.unsafe_exit("Process32NextW", json!({"ok": next.is_ok()}));
            if next.is_err() {
                break;
            }
        }
        Ok(false)
    })();

    close_handle(snapshot, logger)?;
    result
}

impl PathResolver {
    pub fn new(logger: &Logger) -> Result<Self, AppError> {
        Ok(Self {
            program_files_x64: known_folder(&FOLDERID_ProgramFilesX64, logger)?,
            local_app_data: known_folder(&FOLDERID_LocalAppData, logger)?,
            desktop: known_folder(&FOLDERID_Desktop, logger)?,
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
    let params = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
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

pub fn message_box(
    title: &str,
    body: &str,
    buttons: TASKDIALOG_COMMON_BUTTON_FLAGS,
    icon: PCWSTR,
    logger: &Logger,
) -> Result<i32, AppError> {
    let mut button = 0i32;
    let title_w = Utf16Arg::from_str(title);
    let body_w = Utf16Arg::from_str(body);
    logger.unsafe_enter("TaskDialog", json!({"title":title}));
    unsafe {
        TaskDialog(
            Some(HWND::default()),
            None,
            PCWSTR(title_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR(body_w.as_ptr()),
            buttons,
            icon,
            Some(&mut button),
        )?
    };
    logger.unsafe_exit("TaskDialog", json!({"result": button}));
    Ok(button)
}

pub fn gui_confirm_install(app_name: &str, logger: &Logger) -> Result<bool, AppError> {
    let result = message_box(
        "covenant-setup",
        &format!("Install {app_name} now?"),
        TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON,
        td_information_icon(),
        logger,
    )?;
    Ok(result == IDOK.0)
}

pub fn gui_report_success(app_name: &str, logger: &Logger) -> Result<(), AppError> {
    let _ = message_box(
        "covenant-setup",
        &format!("{app_name} installation completed successfully"),
        TDCBF_OK_BUTTON,
        td_information_icon(),
        logger,
    )?;
    Ok(())
}

pub fn gui_report_error(message: &str, logger: &Logger) -> Result<(), AppError> {
    let _ = message_box(
        "covenant-setup",
        message,
        TDCBF_OK_BUTTON,
        td_error_icon(),
        logger,
    )?;
    Ok(())
}

pub fn gui_report_uninstall_success(app_name: &str, logger: &Logger) -> Result<(), AppError> {
    let _ = message_box(
        "covenant-setup",
        &format!("{app_name} uninstalled successfully!"),
        TDCBF_OK_BUTTON,
        td_information_icon(),
        logger,
    )?;
    Ok(())
}

pub fn gui_prompt_uninstall_reboot(app_name: &str, logger: &Logger) -> Result<bool, AppError> {
    let result = message_box(
        "covenant-setup",
        &format!(
            "{app_name} uninstalled sucessfully! Some files from the program still remain on your computer. To complete removal of these files, restart your computer now."
        ),
        TDCBF_YES_BUTTON | TDCBF_NO_BUTTON,
        td_information_icon(),
        logger,
    )?;
    Ok(result == IDYES.0)
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

fn wide_array_to_string(buffer: &[u16]) -> String {
    let len = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
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

fn td_information_icon() -> PCWSTR {
    PCWSTR(std::ptr::without_provenance(0xFFFD))
}

fn td_error_icon() -> PCWSTR {
    PCWSTR(std::ptr::without_provenance(0xFFFE))
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
