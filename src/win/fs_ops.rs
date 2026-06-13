use super::*;

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
