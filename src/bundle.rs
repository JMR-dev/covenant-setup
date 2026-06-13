use crate::sys::Sys;
use crate::{
    AppError, InstallManifest, Logger, UiMode, UiPreferences, enforce_manifest_path_spacing,
    expected_manifest_file_name, failure_ux_shown, install, read_install_manifest, same_path,
    select_ui, trace_event, ui_mode_name,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process;

pub(crate) const EMBEDDED_MAGIC: &[u8] = b"COVENANT_SETUP_BUNDLE_V1";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PackagedApp {
    pub(crate) app_name: String,
    pub(crate) manifest: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EmbeddedFile {
    pub(crate) relative_path: String,
    pub(crate) data: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EmbeddedBundle {
    pub(crate) metadata: PackagedApp,
    pub(crate) files: Vec<EmbeddedFile>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EmbeddedBundleIndex {
    pub(crate) metadata: PackagedApp,
    pub(crate) files: Vec<EmbeddedFileIndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EmbeddedFileIndexEntry {
    pub(crate) relative_path: String,
    pub(crate) len: u64,
}

pub(crate) fn package(
    manifest_path: &Path,
    output_root: &Path,
    logger: &Logger,
) -> Result<(), AppError> {
    enforce_manifest_path_spacing(manifest_path)?;
    let manifest = read_install_manifest(manifest_path)?;
    let current_exe = std::env::current_exe()?;
    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| AppError::Message("Manifest must have a parent directory".into()))?;
    fs::create_dir_all(output_root)?;
    let installer_target = output_root.join("covenant-setup-installer.exe");
    build_packaged_installer(
        &installer_target,
        &current_exe,
        manifest_dir,
        manifest_path,
        &manifest,
        logger,
    )?;

    logger.result(
        "ok",
        json!({
            "installer": installer_target
        }),
    );
    Ok(())
}

pub(crate) fn build_packaged_installer(
    exe_target: &Path,
    current_exe: &Path,
    manifest_dir: &Path,
    manifest_path: &Path,
    manifest: &InstallManifest,
    logger: &Logger,
) -> Result<(), AppError> {
    fs::copy(current_exe, exe_target)?;
    let bundled_manifest_name = expected_manifest_file_name(&manifest.app_name);
    let bundle = EmbeddedBundle {
        metadata: PackagedApp {
            app_name: manifest.app_name.clone(),
            manifest: bundled_manifest_name.clone(),
        },
        files: collect_bundle_files(
            manifest_dir,
            manifest_path,
            &bundled_manifest_name,
            &[exe_target.to_path_buf()],
        )?,
    };
    append_embedded_bundle(exe_target, &bundle)?;
    logger.info(
        "package_artifact",
        json!({
            "exe": exe_target,
            "embedded_files": bundle.files.len()
        }),
    );
    Ok(())
}

pub(crate) fn collect_bundle_files(
    source_root: &Path,
    manifest_path: &Path,
    bundled_manifest_name: &str,
    excluded_paths: &[PathBuf],
) -> Result<Vec<EmbeddedFile>, AppError> {
    let mut files = Vec::new();
    collect_bundle_files_recursive(
        source_root,
        source_root,
        manifest_path,
        bundled_manifest_name,
        excluded_paths,
        &mut files,
    )?;
    Ok(files)
}

fn collect_bundle_files_recursive(
    source_root: &Path,
    current: &Path,
    manifest_path: &Path,
    bundled_manifest_name: &str,
    excluded_paths: &[PathBuf],
    files: &mut Vec<EmbeddedFile>,
) -> Result<(), AppError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if excluded_paths
            .iter()
            .any(|excluded| same_path(&path, excluded))
        {
            continue;
        }
        let relative = path
            .strip_prefix(source_root)
            .map_err(|_| AppError::Message("Failed to derive embedded file path".into()))?;
        if should_exclude_from_bundle(relative) {
            continue;
        }
        if path.is_dir() {
            collect_bundle_files_recursive(
                source_root,
                &path,
                manifest_path,
                bundled_manifest_name,
                excluded_paths,
                files,
            )?;
        } else {
            let relative_path = if same_path(&path, manifest_path) {
                bundled_manifest_name.to_string()
            } else {
                relative.to_string_lossy().to_string()
            };
            files.push(EmbeddedFile {
                relative_path,
                data: fs::read(&path)?,
            });
        }
    }
    Ok(())
}

pub(crate) fn should_exclude_from_bundle(relative_path: &Path) -> bool {
    let Some(file_name) = relative_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        file_name.to_ascii_lowercase().as_str(),
        "journal.json" | "covenant-setup-uninstall.exe" | "covenant-setup-installer.exe"
    )
}

pub(crate) fn append_embedded_bundle(
    exe_target: &Path,
    bundle: &EmbeddedBundle,
) -> Result<(), AppError> {
    let index = EmbeddedBundleIndex {
        metadata: PackagedApp {
            app_name: bundle.metadata.app_name.clone(),
            manifest: bundle.metadata.manifest.clone(),
        },
        files: bundle
            .files
            .iter()
            .map(|file| EmbeddedFileIndexEntry {
                relative_path: file.relative_path.clone(),
                len: file.data.len() as u64,
            })
            .collect(),
    };
    let index_bytes = serde_json::to_vec(&index)?;
    let mut payload = Vec::with_capacity(
        std::mem::size_of::<u64>()
            + index_bytes.len()
            + bundle
                .files
                .iter()
                .map(|file| file.data.len())
                .sum::<usize>(),
    );
    payload.write_all(&(index_bytes.len() as u64).to_le_bytes())?;
    payload.write_all(&index_bytes)?;
    for file in &bundle.files {
        payload.write_all(&file.data)?;
    }
    let mut file = fs::OpenOptions::new().append(true).open(exe_target)?;
    file.write_all(&payload)?;
    file.write_all(&(payload.len() as u64).to_le_bytes())?;
    file.write_all(EMBEDDED_MAGIC)?;
    Ok(())
}

pub(crate) fn read_embedded_bundle(exe_path: &Path) -> Result<Option<EmbeddedBundle>, AppError> {
    let mut file = fs::File::open(exe_path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let footer_len = EMBEDDED_MAGIC.len() + std::mem::size_of::<u64>();
    if bytes.len() < footer_len {
        return Ok(None);
    }
    let magic_offset = bytes.len() - EMBEDDED_MAGIC.len();
    if &bytes[magic_offset..] != EMBEDDED_MAGIC {
        return Ok(None);
    }
    let size_offset = magic_offset - std::mem::size_of::<u64>();
    let payload_len = u64::from_le_bytes(
        bytes[size_offset..magic_offset]
            .try_into()
            .map_err(|_| AppError::Message("Invalid embedded payload footer".into()))?,
    ) as usize;
    if size_offset < payload_len {
        return Err(AppError::Message(
            "Embedded payload length exceeds executable size".into(),
        ));
    }
    let payload_offset = size_offset - payload_len;
    let payload = &bytes[payload_offset..size_offset];
    if payload.len() < std::mem::size_of::<u64>() {
        return Err(AppError::Message("Embedded payload is too short".into()));
    }
    let index_len = u64::from_le_bytes(
        payload[..std::mem::size_of::<u64>()]
            .try_into()
            .map_err(|_| AppError::Message("Invalid embedded index length".into()))?,
    ) as usize;
    let index_offset = std::mem::size_of::<u64>();
    let data_offset = index_offset
        .checked_add(index_len)
        .ok_or_else(|| AppError::Message("Embedded index length overflow".into()))?;
    if data_offset > payload.len() {
        return Err(AppError::Message(
            "Embedded index length exceeds payload size".into(),
        ));
    }
    let index: EmbeddedBundleIndex = serde_json::from_slice(&payload[index_offset..data_offset])?;
    let EmbeddedBundleIndex {
        metadata,
        files: index_files,
    } = index;
    let mut cursor = data_offset;
    let mut files = Vec::with_capacity(index_files.len());
    for entry in index_files {
        let len = entry.len as usize;
        let end = cursor
            .checked_add(len)
            .ok_or_else(|| AppError::Message("Embedded file length overflow".into()))?;
        if end > payload.len() {
            return Err(AppError::Message(format!(
                "Embedded file exceeds payload size: {}",
                entry.relative_path
            )));
        }
        files.push(EmbeddedFile {
            relative_path: entry.relative_path,
            data: payload[cursor..end].to_vec(),
        });
        cursor = end;
    }
    if cursor != payload.len() {
        return Err(AppError::Message(
            "Embedded payload has trailing bytes after file data".into(),
        ));
    }
    let bundle = EmbeddedBundle { metadata, files };
    Ok(Some(bundle))
}

pub(crate) fn extract_embedded_bundle(
    exe_path: &Path,
    bundle: &EmbeddedBundle,
) -> Result<PathBuf, AppError> {
    let temp_root = std::env::temp_dir().join("covenant-setup").join(format!(
        "{}-{}",
        exe_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .replace(' ', "_"),
        process::id()
    ));
    if temp_root.exists() {
        fs::remove_dir_all(&temp_root)?;
    }
    fs::create_dir_all(&temp_root)?;
    for file in &bundle.files {
        let target = temp_root.join(&file.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, &file.data)?;
    }
    Ok(temp_root)
}

pub(crate) fn has_embedded_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| read_embedded_bundle(&exe).ok().flatten())
        .is_some()
}

pub(crate) fn run_bundled_installer(
    preferences: UiPreferences,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<(), AppError> {
    trace_event("bundled_installer_start", json!({}));
    let exe = std::env::current_exe()?;
    let bundle = read_embedded_bundle(&exe)?
        .ok_or_else(|| AppError::Message("No embedded package found in installer".into()))?;
    let extraction_root = extract_embedded_bundle(&exe, &bundle)?;
    trace_event(
        "bundled_installer_extracted",
        json!({"exe": exe, "extraction_root": extraction_root}),
    );
    let metadata = bundle.metadata;
    let manifest_path = extraction_root.join(metadata.manifest.clone());
    let ui_mode = select_ui(preferences, sys, logger)?;
    trace_event(
        "bundled_installer_ui_selected",
        json!({"ui_mode": ui_mode_name(ui_mode), "automation": preferences.automation}),
    );
    match install(
        &manifest_path,
        None,
        true,
        ui_mode,
        preferences.automation,
        sys,
        None,
        logger,
    ) {
        Ok(()) => {
            trace_event("bundled_installer_install_ok", json!({}));
            // The progress window now lingers showing the success result with
            // a Close button, so no extra success prompt is needed.
            Ok(())
        }
        Err(err) => {
            if matches!(err, AppError::CancelledByUser) {
                trace_event("bundled_installer_install_cancelled", json!({}));
                return Ok(());
            }
            let err_msg = err.to_string();
            trace_event("bundled_installer_install_error", json!({"error": err_msg}));
            if ui_mode == UiMode::Gui
                && !preferences.automation
                && !failure_ux_shown()
                && !matches!(err, AppError::Cancelled)
            {
                sys.ui_report_error(&err_msg)?;
            }
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::*;

    #[test]
    fn embedded_bundle_round_trips_through_exe_footer() {
        let temp = TestDir::new("bundle-round-trip");
        let exe = temp.path().join("installer.exe");
        fs::write(&exe, b"stub executable bytes").unwrap();
        let manifest_name = expected_manifest_file_name("Round Trip App");

        let bundle = EmbeddedBundle {
            metadata: PackagedApp {
                app_name: "Round Trip App".to_string(),
                manifest: manifest_name.clone(),
            },
            files: vec![
                EmbeddedFile {
                    relative_path: manifest_name,
                    data: b"app_name = 'Round Trip App'".to_vec(),
                },
                EmbeddedFile {
                    relative_path: "payload\\tool.exe".to_string(),
                    data: vec![0, 1, 2, 3, 255],
                },
            ],
        };

        append_embedded_bundle(&exe, &bundle).unwrap();
        let decoded = read_embedded_bundle(&exe).unwrap().unwrap();

        assert_eq!(decoded, bundle);
        assert!(
            fs::read(&exe)
                .unwrap()
                .starts_with(b"stub executable bytes")
        );
    }

    #[test]
    fn read_embedded_bundle_returns_none_for_wrong_magic_footer() {
        let temp = TestDir::new("wrong-magic");
        let exe = temp.path().join("plain.exe");
        let mut bytes = vec![0; std::mem::size_of::<u64>()];
        bytes.extend_from_slice(b"COVENANT_SETUP_BUNDLE_BAD");
        fs::write(&exe, bytes).unwrap();

        assert!(read_embedded_bundle(&exe).unwrap().is_none());
    }

    #[test]
    fn read_embedded_bundle_rejects_payload_length_past_file_start() {
        let temp = TestDir::new("bad-payload-len");
        let exe = temp.path().join("installer.exe");
        let mut bytes = b"stub".to_vec();
        bytes.extend_from_slice(&100u64.to_le_bytes());
        bytes.extend_from_slice(EMBEDDED_MAGIC);
        fs::write(&exe, bytes).unwrap();

        let err = read_embedded_bundle(&exe).unwrap_err().to_string();
        assert!(err.contains("Embedded payload length exceeds executable size"));
    }

    #[test]
    fn read_embedded_bundle_rejects_short_payload() {
        let temp = TestDir::new("short-payload");
        let exe = temp.path().join("installer.exe");
        write_embedded_payload(&exe, &[1, 2, 3, 4]);

        let err = read_embedded_bundle(&exe).unwrap_err().to_string();
        assert!(err.contains("Embedded payload is too short"));
    }

    #[test]
    fn read_embedded_bundle_rejects_index_length_past_payload() {
        let temp = TestDir::new("bad-index-len");
        let exe = temp.path().join("installer.exe");
        write_embedded_payload(&exe, &100u64.to_le_bytes());

        let err = read_embedded_bundle(&exe).unwrap_err().to_string();
        assert!(err.contains("Embedded index length exceeds payload size"));
    }

    #[test]
    fn read_embedded_bundle_rejects_file_length_past_payload() {
        let temp = TestDir::new("bad-file-len");
        let exe = temp.path().join("installer.exe");
        let index = EmbeddedBundleIndex {
            metadata: PackagedApp {
                app_name: "Bad File".to_string(),
                manifest: expected_manifest_file_name("Bad File"),
            },
            files: vec![EmbeddedFileIndexEntry {
                relative_path: "payload.bin".to_string(),
                len: 10,
            }],
        };
        let mut payload = Vec::new();
        let index_bytes = serde_json::to_vec(&index).unwrap();
        payload.extend_from_slice(&(index_bytes.len() as u64).to_le_bytes());
        payload.extend_from_slice(&index_bytes);
        write_embedded_payload(&exe, &payload);

        let err = read_embedded_bundle(&exe).unwrap_err().to_string();
        assert!(err.contains("Embedded file exceeds payload size"));
    }

    #[test]
    fn read_embedded_bundle_rejects_trailing_payload_bytes() {
        let temp = TestDir::new("trailing-payload");
        let exe = temp.path().join("installer.exe");
        let index = EmbeddedBundleIndex {
            metadata: PackagedApp {
                app_name: "Trailing".to_string(),
                manifest: expected_manifest_file_name("Trailing"),
            },
            files: vec![EmbeddedFileIndexEntry {
                relative_path: "empty.bin".to_string(),
                len: 0,
            }],
        };
        let mut payload = Vec::new();
        let index_bytes = serde_json::to_vec(&index).unwrap();
        payload.extend_from_slice(&(index_bytes.len() as u64).to_le_bytes());
        payload.extend_from_slice(&index_bytes);
        payload.push(1);
        write_embedded_payload(&exe, &payload);

        let err = read_embedded_bundle(&exe).unwrap_err().to_string();
        assert!(err.contains("Embedded payload has trailing bytes"));
    }

    #[test]
    fn extract_embedded_bundle_writes_nested_files_to_temp_root() {
        let temp = TestDir::new("extract-bundle");
        let exe = temp.path().join("installer with spaces.exe");
        fs::write(&exe, b"stub").unwrap();
        let bundle = EmbeddedBundle {
            metadata: PackagedApp {
                app_name: "Extract App".to_string(),
                manifest: expected_manifest_file_name("Extract App"),
            },
            files: vec![EmbeddedFile {
                relative_path: "nested\\payload.txt".to_string(),
                data: b"payload".to_vec(),
            }],
        };

        let root = extract_embedded_bundle(&exe, &bundle).unwrap();
        assert_eq!(
            fs::read(root.join("nested\\payload.txt")).unwrap(),
            b"payload"
        );
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("installer_with_spaces")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_embedded_bundle_returns_none_without_bundle_footer() {
        let temp = TestDir::new("no-bundle");
        let exe = temp.path().join("plain.exe");
        fs::write(&exe, b"plain executable bytes").unwrap();

        assert!(read_embedded_bundle(&exe).unwrap().is_none());
    }

    #[test]
    fn build_packaged_installer_copies_stub_and_embeds_source_bundle() {
        let temp = TestDir::new("package-installer");
        let source_root = temp.path().join("source");
        let payload_dir = source_root.join("payload");
        let manifest_path = source_root.join(expected_manifest_file_name("Packaged App"));
        let current_exe = temp.path().join("current.exe");
        let exe_target = source_root
            .join("dist")
            .join("covenant-setup-installer.exe");
        fs::create_dir_all(&payload_dir).unwrap();
        fs::create_dir_all(exe_target.parent().unwrap()).unwrap();
        fs::write(&current_exe, b"stub exe").unwrap();
        fs::write(&manifest_path, b"app_name = 'Packaged App'").unwrap();
        fs::write(payload_dir.join("app.bin"), b"payload bytes").unwrap();

        let manifest = InstallManifest {
            app_name: "Packaged App".to_string(),
            directories: Vec::new(),
            files: Vec::new(),
            registry: Vec::new(),
            shortcuts: Vec::new(),
            scripts: Vec::new(),
            purge: PurgeSpec::default(),
            support_contact: None,
        };

        build_packaged_installer(
            &exe_target,
            &current_exe,
            &source_root,
            &manifest_path,
            &manifest,
            &quiet_logger(),
        )
        .unwrap();

        let exe_bytes = fs::read(&exe_target).unwrap();
        assert!(exe_bytes.starts_with(b"stub exe"));
        let mut bundle = read_embedded_bundle(&exe_target).unwrap().unwrap();
        bundle
            .files
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        assert_eq!(bundle.metadata.app_name, "Packaged App");
        assert_eq!(
            bundle.metadata.manifest,
            expected_manifest_file_name("Packaged App")
        );
        assert_eq!(bundle.files.len(), 2);
        assert_eq!(
            bundle.files[0].relative_path,
            expected_manifest_file_name("Packaged App")
        );
        assert_eq!(bundle.files[1].relative_path, "payload\\app.bin");
        assert_eq!(bundle.files[1].data, b"payload bytes");
    }

    #[test]
    fn collect_bundle_files_renames_manifest_and_preserves_nested_payloads() {
        let temp = TestDir::new("collect-bundle");
        let manifest = temp.path().join("CollectedApp-install.toml");
        let bundled_manifest_name = expected_manifest_file_name("Collected App");
        let nested_dir = temp.path().join("payload").join("bin");
        let nested_file = nested_dir.join("app.cmd");
        let journal = temp.path().join("journal.json");
        let generated_installer = temp
            .path()
            .join("dist")
            .join("covenant-setup-installer.exe");
        let generated_uninstaller = temp.path().join("covenant-setup-uninstall.exe");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::create_dir_all(generated_installer.parent().unwrap()).unwrap();
        fs::write(&manifest, b"app_name = 'Collected App'").unwrap();
        fs::write(&nested_file, b"@echo off").unwrap();
        fs::write(&journal, b"{}").unwrap();
        fs::write(&generated_installer, b"generated installer").unwrap();
        fs::write(&generated_uninstaller, b"generated uninstaller").unwrap();

        let mut files = collect_bundle_files(
            temp.path(),
            &manifest,
            &bundled_manifest_name,
            std::slice::from_ref(&generated_installer),
        )
        .unwrap();
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].relative_path, bundled_manifest_name);
        assert_eq!(files[0].data, b"app_name = 'Collected App'");
        assert_eq!(files[1].relative_path, "payload\\bin\\app.cmd");
        assert_eq!(files[1].data, b"@echo off");
    }

    #[test]
    fn should_exclude_from_bundle_matches_generated_artifacts_only() {
        assert!(should_exclude_from_bundle(Path::new("journal.json")));
        assert!(should_exclude_from_bundle(Path::new(
            "dist\\covenant-setup-installer.exe"
        )));
        assert!(should_exclude_from_bundle(Path::new(
            "covenant-setup-uninstall.exe"
        )));
        assert!(!should_exclude_from_bundle(Path::new("payload\\app.exe")));
        assert!(!should_exclude_from_bundle(Path::new("")));
    }

    #[test]
    fn embedded_bundle_probe_is_false_for_test_binary() {
        assert!(!has_embedded_bundle());
    }
}
