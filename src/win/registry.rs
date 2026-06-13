use super::*;

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
