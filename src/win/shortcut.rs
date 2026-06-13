use super::*;

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
