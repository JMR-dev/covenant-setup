use super::*;

pub(crate) const UI_EXE_NAME: &str = "Covenant.Setup.Ui.exe";
// Must match the bundle format written by write_ui_bundle in build.rs.
pub(crate) const UI_BUNDLE_MAGIC: &[u8] = b"COVENANT_SETUP_UI_BUNDLE_V1\n";
#[cfg(covenant_setup_embedded_ui)]
pub(crate) fn embedded_ui_bundle_bytes() -> Option<&'static [u8]> {
    Some(include_bytes!(env!("COVENANT_SETUP_UI_BUNDLE")))
}

#[cfg(not(covenant_setup_embedded_ui))]
pub(crate) fn embedded_ui_bundle_bytes() -> Option<&'static [u8]> {
    None
}

pub fn is_available() -> bool {
    embedded_ui_bundle_bytes().is_some() || sidecar_ui_exe().is_some()
}

/// Trait abstraction over the live GUI progress IPC channel so install /
/// uninstall code can be unit-tested with a recording mock instead of
pub(crate) struct PreparedUiExe {
    pub(crate) path: PathBuf,
    pub(crate) cleanup_root: Option<PathBuf>,
}

pub(crate) fn prepare_ui_exe() -> Result<PreparedUiExe, AppError> {
    if let Some(bytes) = embedded_ui_bundle_bytes() {
        return extract_ui_bundle(bytes);
    }
    if let Some(path) = sidecar_ui_exe() {
        return Ok(PreparedUiExe {
            path,
            cleanup_root: None,
        });
    }
    Err(AppError::Message(format!(
        "C# UI helper is not bundled and no {UI_EXE_NAME} was found next to the installer"
    )))
}

pub(crate) fn extract_ui_bundle(bytes: &[u8]) -> Result<PreparedUiExe, AppError> {
    if !bytes.starts_with(UI_BUNDLE_MAGIC) {
        return Err(AppError::Message("Embedded C# UI bundle is invalid".into()));
    }

    let root = std::env::temp_dir().join(format!(
        "covenant-setup-ui-{}-{}",
        process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&root)?;
    let mut cursor = UI_BUNDLE_MAGIC.len();

    loop {
        let path_len = read_bundle_u32(bytes, &mut cursor)? as usize;
        let data_len = usize::try_from(read_bundle_u64(bytes, &mut cursor)?)
            .map_err(|_| AppError::Message("Embedded C# UI file is too large".into()))?;
        if path_len == 0 {
            if data_len != 0 {
                return Err(AppError::Message(
                    "Embedded C# UI bundle has an invalid terminator".into(),
                ));
            }
            break;
        }

        let relative_path = std::str::from_utf8(take_bundle_bytes(bytes, &mut cursor, path_len)?)
            .map_err(|_| {
            AppError::Message("Embedded C# UI bundle path is not valid UTF-8".into())
        })?;
        let data = take_bundle_bytes(bytes, &mut cursor, data_len)?;

        let output_path = bundle_output_path(&root, relative_path)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_path, data)?;
    }

    let path = root.join(UI_EXE_NAME);
    if !path.is_file() {
        let _ = fs::remove_dir_all(&root);
        return Err(AppError::Message(format!(
            "Embedded C# UI bundle did not contain {UI_EXE_NAME}"
        )));
    }

    Ok(PreparedUiExe {
        path,
        cleanup_root: Some(root),
    })
}

pub(crate) fn sidecar_ui_exe() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(UI_EXE_NAME);
    path.is_file().then_some(path)
}

pub(crate) fn read_bundle_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, AppError> {
    Ok(u32::from_le_bytes(
        take_bundle_bytes(bytes, cursor, 4)?.try_into().unwrap(),
    ))
}

pub(crate) fn read_bundle_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, AppError> {
    Ok(u64::from_le_bytes(
        take_bundle_bytes(bytes, cursor, 8)?.try_into().unwrap(),
    ))
}

/// Advances `cursor` past the next `len` bytes and returns them, or errors if
/// the bundle is truncated.
pub(crate) fn take_bundle_bytes<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    len: usize,
) -> Result<&'a [u8], AppError> {
    let end = cursor
        .checked_add(len)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| AppError::Message("Embedded C# UI bundle ended unexpectedly".into()))?;
    let slice = &bytes[*cursor..end];
    *cursor = end;
    Ok(slice)
}

pub(crate) fn bundle_output_path(root: &Path, relative_path: &str) -> Result<PathBuf, AppError> {
    let relative_path = Path::new(relative_path);
    if relative_path.is_absolute() {
        return Err(AppError::Message(
            "Embedded C# UI bundle contains an absolute path".into(),
        ));
    }

    let mut output_path = root.to_path_buf();
    for component in relative_path.components() {
        match component {
            Component::Normal(part) => output_path.push(part),
            _ => {
                return Err(AppError::Message(
                    "Embedded C# UI bundle contains an unsafe path".into(),
                ));
            }
        }
    }
    Ok(output_path)
}
