#![windows_subsystem = "windows"]
mod sys;
mod ui;
mod win;

use clap::{ArgAction, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs;
use std::io;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sys::{Sys, WinSys};
use thiserror::Error;
use ui::ProgressSink;

const EXIT_ELEVATION_REQUIRED: i32 = 33;
const EXIT_OPERATION_FAILED: i32 = 1;
const EXIT_CANCELLED: i32 = 40;
const EMBEDDED_MAGIC: &[u8] = b"COVENANT_SETUP_BUNDLE_V1";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
static FAILURE_UX_SHOWN: AtomicBool = AtomicBool::new(false);

#[derive(Parser, Debug)]
#[command(
    name = "covenant-setup",
    version,
    about = "Windows installer builder and engine"
)]
struct Cli {
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    json: bool,
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    headless: bool,
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "headless")]
    headed: bool,
    #[arg(long, global = true, hide = true, action = ArgAction::SetTrue)]
    automation: bool,
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    elevate: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Package {
        manifest: PathBuf,
        #[arg(long, default_value = "dist")]
        output: PathBuf,
    },
    Install {
        manifest: PathBuf,
        #[arg(long)]
        journal: Option<PathBuf>,
    },
    Uninstall {
        journal: PathBuf,
    },
    #[command(hide = true)]
    Cleanup {
        #[arg(long)]
        target_exe: PathBuf,
        #[arg(long)]
        install_root: Option<PathBuf>,
        #[arg(long)]
        app_name: String,
    },
}

#[derive(Debug, Deserialize)]
struct InstallManifest {
    app_name: String,
    #[serde(default, deserialize_with = "deserialize_directory_paths")]
    directories: Vec<String>,
    #[serde(default)]
    files: Vec<FileSpec>,
    #[serde(default)]
    registry: Vec<RegistrySpec>,
    #[serde(default)]
    shortcuts: Vec<ShortcutSpec>,
    #[serde(default)]
    scripts: Vec<ScriptSpec>,
    #[serde(default)]
    purge: PurgeSpec,
    #[serde(default)]
    support_contact: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PurgeSpec {
    #[serde(default)]
    registry_branches: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DirectoryPaths {
    #[serde(default)]
    paths: Vec<String>,
}

fn deserialize_directory_paths<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(DirectoryPaths::deserialize(deserializer)?.paths)
}

#[derive(Debug, Deserialize)]
struct FileSpec {
    source: String,
    destination: String,
}

#[derive(Debug, Deserialize)]
struct RegistrySpec {
    key: String,
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct ShortcutSpec {
    path: String,
    target: String,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScriptSpec {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Debug, Clone)]
struct InstallRuntime {
    journal_path: PathBuf,
    install_root: Option<PathBuf>,
    uninstall_exe_path: Option<PathBuf>,
    uninstall_registry_root: RegistryRoot,
    uninstall_registry_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Journal {
    app_name: String,
    manifest_path: Option<PathBuf>,
    actions: Vec<JournalAction>,
    purge: PurgeSpec,
    #[serde(default)]
    support_contact: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JournalAction {
    CreateDirectory {
        path: PathBuf,
    },
    CopyFile {
        source: PathBuf,
        destination: PathBuf,
    },
    WriteRegistry {
        root: RegistryRoot,
        subkey: String,
        name: String,
    },
    CreateShortcut {
        path: PathBuf,
    },
    ExecuteScript {
        command: String,
        args: Vec<String>,
        working_directory: Option<PathBuf>,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PackagedApp {
    app_name: String,
    manifest: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct EmbeddedFile {
    relative_path: String,
    data: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct EmbeddedBundle {
    metadata: PackagedApp,
    files: Vec<EmbeddedFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmbeddedBundleIndex {
    metadata: PackagedApp,
    files: Vec<EmbeddedFileIndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmbeddedFileIndexEntry {
    relative_path: String,
    len: u64,
}

trait MutationTracker {
    fn record(&mut self, action: JournalAction);
    fn finish(
        self,
        app_name: String,
        manifest_path: Option<PathBuf>,
        purge: PurgeSpec,
        support_contact: Option<String>,
    ) -> Journal;
}

struct DeclaredTracker {
    actions: Vec<JournalAction>,
}

impl DeclaredTracker {
    fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }
}

impl MutationTracker for DeclaredTracker {
    fn record(&mut self, action: JournalAction) {
        self.actions.push(action);
    }

    fn finish(
        self,
        app_name: String,
        manifest_path: Option<PathBuf>,
        purge: PurgeSpec,
        support_contact: Option<String>,
    ) -> Journal {
        Journal {
            app_name,
            manifest_path,
            actions: self.actions,
            purge,
            support_contact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RegistryRoot {
    Hkcu,
    Hklm,
}

#[derive(Debug, Error)]
enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("Installation cancelled; changes were reverted")]
    Cancelled,
    #[error("Installation cancelled by user")]
    CancelledByUser,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Windows(#[from] windows::core::Error),
}

#[derive(Clone)]
struct Logger {
    json: bool,
    quiet: bool,
}

impl Logger {
    fn info(&self, event: &str, detail: impl Serialize) {
        if self.quiet {
            return;
        }
        if self.json {
            println!("{}", json!({"type":"event","event":event,"detail":detail}));
        } else {
            println!(
                "{event}: {}",
                serde_json::to_string(&detail).unwrap_or_default()
            );
        }
    }

    fn unsafe_enter(&self, operation: &str, detail: impl Serialize) {
        self.info(
            "unsafe_enter",
            json!({"operation":operation,"detail":detail}),
        );
    }

    fn unsafe_exit(&self, operation: &str, detail: impl Serialize) {
        self.info(
            "unsafe_exit",
            json!({"operation":operation,"detail":detail}),
        );
    }

    fn result(&self, status: &str, detail: impl Serialize) {
        if self.quiet {
            return;
        }
        if self.json {
            println!(
                "{}",
                json!({"type":"result","status":status,"detail":detail})
            );
        } else {
            println!(
                "{status}: {}",
                serde_json::to_string(&detail).unwrap_or_default()
            );
        }
    }

    fn error(&self, message: impl Display, code: i32) {
        if self.json {
            println!(
                "{}",
                json!({"type":"error","code":code,"message":message.to_string()})
            );
        } else {
            eprintln!("error[{code}]: {message}");
        }
    }

    fn quiet_clone(&self) -> Self {
        Self {
            json: self.json,
            quiet: true,
        }
    }
}

#[derive(Clone, Copy)]
struct UiPreferences {
    json: bool,
    headless: bool,
    headed: bool,
    automation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiMode {
    None,
    Gui,
    Tui,
}

struct TuiProgress {
    active: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TuiProgress {
    fn start(label: String) -> Self {
        let active = Arc::new(AtomicBool::new(true));
        let active_thread = active.clone();
        let handle = thread::spawn(move || {
            let frames = ["", ".", "..", "...", "....", "....."];
            let mut index = 0usize;
            while active_thread.load(Ordering::Relaxed) {
                let frame = frames[index % frames.len()];
                print!("\r{label}{frame}   ");
                let _ = io::stdout().flush();
                thread::sleep(Duration::from_millis(500));
                index = (index + 1) % frames.len();
            }
            print!("\r{}\r", " ".repeat(label.len() + 8));
            let _ = io::stdout().flush();
        });
        Self {
            active,
            handle: Some(handle),
        }
    }
}

impl Drop for TuiProgress {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    trace_event(
        "process_start",
        json!({
            "pid": process::id(),
            "args": args.iter().map(|arg| arg.to_string_lossy().to_string()).collect::<Vec<_>>()
        }),
    );
    let cli = Cli::parse();
    let logger = Logger {
        json: cli.json,
        quiet: false,
    };
    let preferences = ui_preferences_from_cli(&cli);
    let sys = WinSys;
    if cli.command.is_none() && sys.has_embedded_bundle() {
        let exit_code = match run_bundled_installer(preferences, &sys, &logger) {
            Ok(()) => 0,
            Err(AppError::Message(ref message)) if message == "__elevated_relaunch__" => 0,
            Err(err) => {
                if preferences.headed
                    && !preferences.automation
                    && !preferences.json
                    && sys.ui_available()
                    && !failure_ux_shown()
                    && !matches!(err, AppError::Cancelled)
                {
                    let _ = sys.ui_report_error(&err.to_string());
                }
                let code = error_exit_code(&err);
                logger.error(err, code);
                code
            }
        };
        process::exit(exit_code);
    }

    let exit_code = match run(cli, &sys, &logger) {
        Ok(()) => 0,
        Err(AppError::Message(message)) if message == "__elevated_relaunch__" => 0,
        Err(err) => {
            let code = error_exit_code(&err);
            logger.error(err, code);
            code
        }
    };
    process::exit(exit_code);
}

fn error_exit_code(err: &AppError) -> i32 {
    match err {
        AppError::Cancelled => EXIT_CANCELLED,
        AppError::Message(message) if message.contains("Elevation required") => {
            EXIT_ELEVATION_REQUIRED
        }
        _ => EXIT_OPERATION_FAILED,
    }
}

fn run(cli: Cli, sys: &dyn Sys, logger: &Logger) -> Result<(), AppError> {
    let preferences = ui_preferences_from_cli(&cli);
    let command = cli.command.ok_or_else(|| {
        AppError::Message(
            "Missing command: expected package, install, uninstall, or cleanup".into(),
        )
    })?;
    match command {
        Commands::Package { manifest, output } => package(&manifest, &output, logger),
        Commands::Install { manifest, journal } => install(
            &manifest,
            journal,
            cli.elevate,
            select_ui(preferences, sys, logger)?,
            preferences.automation,
            sys,
            None,
            logger,
        ),
        Commands::Uninstall { journal } => uninstall(
            &journal,
            cli.elevate,
            select_ui(preferences, sys, logger)?,
            preferences.automation,
            sys,
            &mut None,
            logger,
            false,
        ),
        Commands::Cleanup {
            target_exe,
            install_root,
            app_name,
        } => cleanup(
            target_exe,
            install_root,
            app_name,
            select_ui(preferences, sys, logger)?,
            preferences.automation,
            sys,
            logger,
        ),
    }
}

fn package(manifest_path: &Path, output_root: &Path, logger: &Logger) -> Result<(), AppError> {
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

fn read_install_manifest(manifest_path: &Path) -> Result<InstallManifest, AppError> {
    let manifest: InstallManifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;
    enforce_manifest_file_name(manifest_path, &manifest)?;
    enforce_manifest_field_spacing(&manifest)?;
    Ok(manifest)
}

fn enforce_manifest_file_name(
    manifest_path: &Path,
    manifest: &InstallManifest,
) -> Result<(), AppError> {
    let manifest_path_text = manifest_path.to_string_lossy();
    if contains_whitespace(&manifest_path_text) {
        return Err(AppError::Message(format!(
            "Manifest path cannot contain spaces: {}",
            manifest_path.display()
        )));
    }

    let expected = expected_manifest_file_name(&manifest.app_name);
    let actual = manifest_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Message("Manifest path must include a file name".into()))?;
    if actual != expected {
        return Err(AppError::Message(format!(
            "Manifest file name must be '{expected}' for app_name '{}'; got '{actual}'",
            manifest.app_name
        )));
    }

    Ok(())
}

fn enforce_manifest_field_spacing(manifest: &InstallManifest) -> Result<(), AppError> {
    for (index, directory) in manifest.directories.iter().enumerate() {
        enforce_no_whitespace(&format!("directories.paths[{index}]"), directory)?;
    }

    for (index, file) in manifest.files.iter().enumerate() {
        enforce_no_whitespace(&format!("files[{index}].source"), &file.source)?;
        enforce_no_whitespace(&format!("files[{index}].destination"), &file.destination)?;
    }

    for (index, registry) in manifest.registry.iter().enumerate() {
        enforce_no_whitespace(&format!("registry[{index}].key"), &registry.key)?;
        enforce_no_whitespace(&format!("registry[{index}].name"), &registry.name)?;
        enforce_no_whitespace(&format!("registry[{index}].value"), &registry.value)?;
    }

    for (index, shortcut) in manifest.shortcuts.iter().enumerate() {
        enforce_no_whitespace(&format!("shortcuts[{index}].path"), &shortcut.path)?;
        enforce_no_whitespace(&format!("shortcuts[{index}].target"), &shortcut.target)?;
        enforce_optional_no_whitespace(
            &format!("shortcuts[{index}].arguments"),
            shortcut.arguments.as_deref(),
        )?;
        enforce_optional_no_whitespace(
            &format!("shortcuts[{index}].working_directory"),
            shortcut.working_directory.as_deref(),
        )?;
    }

    for (index, script) in manifest.scripts.iter().enumerate() {
        enforce_no_whitespace(&format!("scripts[{index}].command"), &script.command)?;
        for (arg_index, arg) in script.args.iter().enumerate() {
            enforce_no_whitespace(&format!("scripts[{index}].args[{arg_index}]"), arg)?;
        }
        enforce_optional_no_whitespace(
            &format!("scripts[{index}].working_directory"),
            script.working_directory.as_deref(),
        )?;
    }

    for (index, branch) in manifest.purge.registry_branches.iter().enumerate() {
        enforce_no_whitespace(&format!("purge.registry_branches[{index}]"), branch)?;
    }

    for (index, path) in manifest.purge.paths.iter().enumerate() {
        enforce_no_whitespace(&format!("purge.paths[{index}]"), path)?;
    }

    Ok(())
}

fn enforce_optional_no_whitespace(field: &str, value: Option<&str>) -> Result<(), AppError> {
    if let Some(value) = value {
        enforce_no_whitespace(field, value)?;
    }
    Ok(())
}

fn enforce_no_whitespace(field: &str, value: &str) -> Result<(), AppError> {
    if contains_whitespace(value) {
        return Err(AppError::Message(format!(
            "Manifest field '{field}' cannot contain spaces or other whitespace: {value}"
        )));
    }

    Ok(())
}

fn contains_whitespace(value: &str) -> bool {
    value.chars().any(char::is_whitespace)
}

fn expected_manifest_file_name(app_name: &str) -> String {
    let name: String = app_name
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    if name.is_empty() {
        "install.toml".to_string()
    } else {
        format!("{name}-install.toml")
    }
}

fn build_packaged_installer(
    exe_target: &Path,
    current_exe: &Path,
    manifest_dir: &Path,
    manifest_path: &Path,
    manifest: &InstallManifest,
    logger: &Logger,
) -> Result<(), AppError> {
    fs::copy(current_exe, &exe_target)?;
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

fn collect_bundle_files(
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

fn should_exclude_from_bundle(relative_path: &Path) -> bool {
    let Some(file_name) = relative_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        file_name.to_ascii_lowercase().as_str(),
        "journal.json" | "covenant-setup-uninstall.exe" | "covenant-setup-installer.exe"
    )
}

fn append_embedded_bundle(exe_target: &Path, bundle: &EmbeddedBundle) -> Result<(), AppError> {
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

fn read_embedded_bundle(exe_path: &Path) -> Result<Option<EmbeddedBundle>, AppError> {
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

fn extract_embedded_bundle(exe_path: &Path, bundle: &EmbeddedBundle) -> Result<PathBuf, AppError> {
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

fn run_bundled_installer(
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

fn write_journal(path: &Path, journal: &Journal) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(journal)?)?;
    Ok(())
}

fn install(
    manifest_path: &Path,
    journal_path: Option<PathBuf>,
    elevate: bool,
    ui_mode: UiMode,
    automation: bool,
    sys: &dyn Sys,
    progress_override: Option<Box<dyn ProgressSink>>,
    logger: &Logger,
) -> Result<(), AppError> {
    let manifest = read_install_manifest(manifest_path)?;
    trace_event(
        "install_start",
        json!({"manifest": manifest_path, "app_name": &manifest.app_name}),
    );
    let app_name = manifest.app_name.clone();
    let _progress = start_tui_progress(ui_mode, format!("Installing {} ", manifest.app_name));
    let effective_logger = if ui_mode == UiMode::Tui {
        logger.quiet_clone()
    } else {
        logger.clone()
    };
    let resolver = win::PathResolver::new(&effective_logger)?;
    let install_total = total_install_steps(&manifest);
    let mut gui_progress = if progress_override.is_some() {
        progress_override
    } else {
        let install_root = infer_install_root(&manifest, &resolver);
        start_gui_progress(
            ui_mode,
            sys,
            &format!("Installing {}", manifest.app_name),
            Some(&manifest.app_name),
            install_root.as_deref(),
            install_total,
            automation,
            &effective_logger,
        )?
    };
    let mut tracker = DeclaredTracker::new();
    let mut runtime_opt: Option<InstallRuntime> = None;
    let result = (|| -> Result<(), AppError> {
        let requires_admin = manifest_requires_admin(&manifest, &resolver)?;
        ensure_elevation_if_needed(requires_admin, elevate, sys, &effective_logger)?;
        trace_event(
            "install_elevation_checked",
            json!({"requires_admin": requires_admin, "elevate": elevate}),
        );
        let runtime = build_install_runtime(
            &manifest,
            manifest_path,
            journal_path,
            requires_admin,
            &resolver,
        )?;
        runtime_opt = Some(runtime.clone());

        let mut progress_step = 0usize;
        for directory in &manifest.directories {
            let path = resolver.resolve(directory);
            effective_logger.info("create_directory", json!({"path":path}));
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Creating directory {}", path.display()),
            )?;
            win::create_directory_recursive(&path, &effective_logger)?;
            tracker.record(JournalAction::CreateDirectory { path });
        }

        for file in &manifest.files {
            let source = absolutize(manifest_path.parent(), &file.source);
            let destination = resolver.resolve(&file.destination);
            if let Some(parent) = destination.parent() {
                win::create_directory_recursive(parent, &effective_logger)?;
            }
            effective_logger.info(
                "copy_file",
                json!({"source":source,"destination":destination}),
            );
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Copying file to {}", destination.display()),
            )?;
            win::copy_file(&source, &destination, &effective_logger)?;
            tracker.record(JournalAction::CopyFile {
                source,
                destination,
            });
        }

        for entry in &manifest.registry {
            let (root, subkey) = parse_registry_key(&entry.key)?;
            let resolved_value = resolver.resolve(&entry.value).to_string_lossy().to_string();
            effective_logger.info(
                "write_registry",
                json!({"key":entry.key,"name":entry.name,"value":resolved_value}),
            );
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Writing registry value {} in {}", entry.name, entry.key),
            )?;
            sys.set_registry_string(
                root,
                &subkey,
                &entry.name,
                &resolved_value,
                &effective_logger,
            )?;
            tracker.record(JournalAction::WriteRegistry {
                root,
                subkey,
                name: entry.name.clone(),
            });
        }

        for shortcut in &manifest.shortcuts {
            let path = resolver.resolve(&shortcut.path);
            let target = resolver.resolve(&shortcut.target);
            let working_directory = shortcut
                .working_directory
                .as_deref()
                .map(|v| resolver.resolve(v));
            if let Some(parent) = path.parent() {
                win::create_directory_recursive(parent, &effective_logger)?;
            }
            effective_logger.info("create_shortcut", json!({"path":path,"target":target}));
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Creating shortcut {}", path.display()),
            )?;
            win::create_shortcut(
                &path,
                &target,
                shortcut.arguments.as_deref(),
                working_directory.as_deref(),
                shortcut.description.as_deref(),
                &effective_logger,
            )?;
            tracker.record(JournalAction::CreateShortcut { path });
        }

        for script in &manifest.scripts {
            let working_directory = script
                .working_directory
                .as_deref()
                .map(|v| resolver.resolve(v));
            effective_logger.info("execute_script", json!({"command":script.command,"args":script.args,"working_directory":working_directory}));
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Running script {}", script.command),
            )?;
            execute_script(
                script,
                manifest_path.parent(),
                working_directory.as_deref(),
                &mut gui_progress,
            )?;
            tracker.record(JournalAction::ExecuteScript {
                command: script.command.clone(),
                args: script.args.clone(),
                working_directory,
            });
        }

        if let Some(uninstall_exe_path) = &runtime.uninstall_exe_path {
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Installing uninstaller {}", uninstall_exe_path.display()),
            )?;
            install_uninstaller(uninstall_exe_path, &effective_logger)?;
            tracker.record(JournalAction::CopyFile {
                source: std::env::current_exe()?,
                destination: uninstall_exe_path.clone(),
            });
        }

        if let (Some(install_root), Some(uninstall_exe_path)) =
            (&runtime.install_root, &runtime.uninstall_exe_path)
        {
            advance_gui_progress_step(
                &mut gui_progress,
                &mut progress_step,
                install_total,
                &format!("Registering {} in Installed Apps", manifest.app_name),
            )?;
            register_uninstall_entry(
                &manifest,
                &runtime,
                install_root,
                uninstall_exe_path,
                sys,
                &effective_logger,
            )?;
            for value_name in [
                "DisplayName",
                "Publisher",
                "DisplayVersion",
                "InstallLocation",
                "DisplayIcon",
                "UninstallString",
                "QuietUninstallString",
            ] {
                tracker.record(JournalAction::WriteRegistry {
                    root: runtime.uninstall_registry_root,
                    subkey: runtime.uninstall_registry_key.clone(),
                    name: value_name.to_string(),
                });
            }
        }

        Ok(())
    })();

    let journal = tracker.finish(
        manifest.app_name.clone(),
        Some(manifest_path.to_path_buf()),
        manifest.purge.clone(),
        manifest.support_contact.clone(),
    );
    let result = result.and_then(|_| {
        let runtime = runtime_opt
            .as_ref()
            .ok_or_else(|| AppError::Message("Install runtime was not initialized".to_string()))?;
        write_journal(&runtime.journal_path, &journal)?;
        trace_event(
            "install_journal_written",
            json!({"journal": runtime.journal_path, "actions": journal.actions.len()}),
        );
        effective_logger.result(
            "ok",
            json!({"journal":runtime.journal_path,"actions":journal.actions.len()}),
        );
        finish_gui_progress(
            &mut gui_progress,
            &format!("{} installation completed successfully", manifest.app_name),
            !automation,
        )?;
        if ui_mode == UiMode::Tui {
            println!("{} installation completed successfully", manifest.app_name);
        }
        Ok(())
    });

    result.map_err(|err| {
        trace_event(
            "install_error",
            json!({"app_name": app_name, "error": err.to_string()}),
        );
        let cancelled = matches!(err, AppError::Cancelled);
        let mut rollback_error_msg = None;
        if let Some(runtime) = &runtime_opt {
            if !journal.actions.is_empty() {
                let write_res = (|| -> Result<(), AppError> {
                    write_journal(&runtime.journal_path, &journal)?;
                    let read_back = fs::read_to_string(&runtime.journal_path)?;
                    let _: Journal = serde_json::from_str(&read_back)?;
                    Ok(())
                })();
                match write_res {
                    Ok(()) => {
                        if cancelled && let Some(progress) = gui_progress.as_mut() {
                            // The flag already tripped the install loop; clear it
                            // so the rollback's own progress is not cancelled too.
                            progress.clear_cancel_request();
                            let _ = progress.log("Cancel requested - reverting changes...");
                        }
                        match uninstall(
                            &runtime.journal_path,
                            elevate,
                            ui_mode,
                            automation,
                            sys,
                            &mut gui_progress,
                            logger,
                            true,
                        ) {
                            Ok(()) => {
                                trace_event(
                                    "rollback_success",
                                    json!({"journal": runtime.journal_path, "actions": journal.actions.len()}),
                                );
                            }
                            Err(u_err) => {
                                rollback_error_msg = Some(format!("Rollback failed: {u_err}"));
                            }
                        }
                    }
                    Err(w_err) => {
                        rollback_error_msg =
                            Some(format!("Could not write or verify journal.json: {w_err}"));
                    }
                }
            }
        }

        if cancelled && rollback_error_msg.is_none() {
            let _ = finish_gui_progress(
                &mut gui_progress,
                &format!("{app_name} installation cancelled. All changes were reverted."),
                !automation,
            );
            return AppError::Cancelled;
        }

        let display_err = if let Some(r_err) = rollback_error_msg {
            AppError::Message(format!(
                "Installation failed: {err}. Additionally, rollback failed: {r_err}"
            ))
        } else {
            err
        };

        let _ = fail_gui_progress(
            &mut gui_progress,
            &app_name,
            "install",
            &display_err,
            manifest.support_contact.as_deref(),
            !automation,
        );
        display_err
    })
}

fn uninstall(
    journal_path: &Path,
    elevate: bool,
    ui_mode: UiMode,
    automation: bool,
    sys: &dyn Sys,
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    logger: &Logger,
    is_rollback: bool,
) -> Result<(), AppError> {
    let journal: Journal = serde_json::from_str(&fs::read_to_string(journal_path)?)?;
    trace_event(
        if is_rollback {
            "rollback_start"
        } else {
            "uninstall_start"
        },
        json!({"journal": journal_path, "app_name": &journal.app_name}),
    );
    let app_name = journal.app_name.clone();
    let action_verb = if is_rollback {
        "Reverting"
    } else {
        "Uninstalling"
    };
    let _progress = start_tui_progress(ui_mode, format!("{} {} ", action_verb, journal.app_name));
    let uninstall_total = total_uninstall_steps(&journal);
    // A rollback reuses the live install progress session passed in by the
    // caller; only a standalone uninstall starts its own.
    if gui_progress.is_none() {
        *gui_progress = start_gui_progress(
            ui_mode,
            sys,
            &format!("{} {}", action_verb, journal.app_name),
            if is_rollback {
                Some(&journal.app_name)
            } else {
                None
            },
            None,
            uninstall_total,
            automation,
            logger,
        )?;
    }
    let result = (|| -> Result<(), AppError> {
        let effective_logger = if ui_mode == UiMode::Tui {
            logger.quiet_clone()
        } else {
            logger.clone()
        };
        let resolver = win::PathResolver::new(&effective_logger)?;
        let requires_admin = journal_requires_admin(&journal, &resolver)?;
        ensure_elevation_if_needed(requires_admin, elevate, sys, &effective_logger)?;
        trace_event(
            "uninstall_elevation_checked",
            json!({"requires_admin": requires_admin, "elevate": elevate}),
        );
        let current_exe = std::env::current_exe().ok();
        let mut deferred_self_delete: Option<PathBuf> = None;
        let mut deferred_uninstall_registry: Vec<(RegistryRoot, String)> = Vec::new();
        let mut progress_step = 0usize;

        for action in journal.actions.iter().rev() {
            match action {
                JournalAction::CreateDirectory { path } => {
                    advance_gui_progress_step(
                        gui_progress,
                        &mut progress_step,
                        uninstall_total,
                        &format!("Removing directory {}", path.display()),
                    )?;
                    if journal_path.starts_with(path) && journal_path.exists() {
                        let _ = fs::remove_file(journal_path);
                    }
                    win::remove_directory_if_exists(path, &effective_logger)?
                }
                JournalAction::CopyFile { destination, .. } => {
                    if current_exe
                        .as_ref()
                        .is_some_and(|exe| same_path(exe, destination))
                    {
                        effective_logger.info("defer_self_delete", json!({"path":destination}));
                        deferred_self_delete = Some(destination.clone());
                    } else {
                        advance_gui_progress_step(
                            gui_progress,
                            &mut progress_step,
                            uninstall_total,
                            &format!("Removing file {}", destination.display()),
                        )?;
                        sys.remove_file_with_fallback(destination, &effective_logger)?
                    }
                }
                JournalAction::WriteRegistry { root, subkey, .. } => {
                    if is_uninstall_registry_key(subkey) {
                        push_unique_registry_branch(
                            &mut deferred_uninstall_registry,
                            *root,
                            subkey.clone(),
                        );
                    } else {
                        advance_gui_progress_step(
                            gui_progress,
                            &mut progress_step,
                            uninstall_total,
                            &format!("Removing registry branch {}", subkey),
                        )?;
                        sys.delete_registry_tree(*root, subkey, &effective_logger)?
                    }
                }
                JournalAction::CreateShortcut { path } => {
                    advance_gui_progress_step(
                        gui_progress,
                        &mut progress_step,
                        uninstall_total,
                        &format!("Removing shortcut {}", path.display()),
                    )?;
                    sys.remove_file_with_fallback(path, &effective_logger)?
                }
                JournalAction::ExecuteScript { .. } => {
                    effective_logger.info("skip_script_rollback", json!({}))
                }
            }
        }

        for branch in &journal.purge.registry_branches {
            let (root, subkey) = parse_registry_key(branch)?;
            advance_gui_progress_step(
                gui_progress,
                &mut progress_step,
                uninstall_total,
                &format!("Purging registry branch {}", branch),
            )?;
            sys.delete_registry_tree(root, &subkey, &effective_logger)?;
        }
        for path in &journal.purge.paths {
            let resolved = resolver.resolve(path);
            advance_gui_progress_step(
                gui_progress,
                &mut progress_step,
                uninstall_total,
                &format!("Purging path {}", resolved.display()),
            )?;
            purge_path(&resolved, sys, &effective_logger)?;
        }

        for (root, subkey) in deferred_uninstall_registry {
            advance_gui_progress_step(
                gui_progress,
                &mut progress_step,
                uninstall_total,
                &format!("Removing uninstall registration {}", subkey),
            )?;
            sys.delete_registry_tree(root, &subkey, &effective_logger)?;
        }

        if let Some(path) = deferred_self_delete {
            if !is_rollback {
                // Never wait for the window here: the cleanup helper must be
                // spawned promptly so self-deletion is not deferred on the user.
                finish_gui_progress(
                    gui_progress,
                    &format!("Finalizing removal of {}", journal.app_name),
                    false,
                )?;
            }
            sys.spawn_cleanup_helper(
                &path,
                path.parent(),
                &journal.app_name,
                ui_mode,
                automation,
                logger.json,
                &effective_logger,
            )?;
        } else if !is_rollback {
            // During a rollback the install error handler owns the single
            // terminal message on the shared progress session.
            finish_gui_progress(
                gui_progress,
                &format!("{} uninstalled successfully!", journal.app_name),
                false,
            )?;
            if ui_mode == UiMode::Gui && !automation {
                sys.ui_report_uninstall_success(&journal.app_name)?;
            }
        }

        effective_logger.result("ok", json!({"journal":journal_path}));
        let _ = fs::remove_file(journal_path);
        trace_event(
            if is_rollback {
                "rollback_ok"
            } else {
                "uninstall_ok"
            },
            json!({"journal": journal_path}),
        );
        Ok(())
    })();

    if let Err(err) = &result {
        trace_event(
            if is_rollback {
                "rollback_error"
            } else {
                "uninstall_error"
            },
            json!({"app_name": app_name, "error": err.to_string()}),
        );
        // Rollback errors bubble up so the install error handler can compose
        // the single combined failure message on the shared session.
        if !is_rollback {
            let _ = fail_gui_progress(
                gui_progress,
                &app_name,
                "uninstall",
                err,
                journal.support_contact.as_deref(),
                !automation,
            );
        }
    }

    result
}

fn cleanup(
    target_exe: PathBuf,
    install_root: Option<PathBuf>,
    app_name: String,
    ui_mode: UiMode,
    automation: bool,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<(), AppError> {
    trace_event(
        "cleanup_start",
        json!({"target_exe": &target_exe, "install_root": &install_root, "app_name": &app_name}),
    );
    let effective_logger = if ui_mode == UiMode::Tui {
        logger.quiet_clone()
    } else {
        logger.clone()
    };
    let mut reboot_required = false;
    for _ in 0..50 {
        if !target_exe.exists() {
            break;
        }
        if fs::remove_file(&target_exe).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if target_exe.exists() {
        sys.remove_file_with_fallback(&target_exe, &effective_logger)?;
        reboot_required = target_exe.exists();
    }
    if let Some(install_root) = install_root {
        if install_root.exists() && fs::read_dir(&install_root)?.next().is_none() {
            win::remove_directory_if_exists(&install_root, &effective_logger)?;
        }
    }
    reboot_required |= sys.schedule_helper_self_cleanup(&effective_logger)?;
    if ui_mode == UiMode::Gui && !automation {
        if reboot_required {
            if sys.ui_prompt_uninstall_reboot(&app_name)? {
                sys.spawn_reboot(&effective_logger)?;
            }
        } else {
            sys.ui_report_uninstall_success(&app_name)?;
        }
    } else if ui_mode == UiMode::Tui {
        if reboot_required {
            println!(
                "{app_name} uninstalled successfully! Some files from the program still remain on your computer. To complete removal of these files, restart your computer now."
            );
            if sys.prompt_reboot_tui()? {
                sys.spawn_reboot(&effective_logger)?;
            }
        } else {
            println!("{app_name} uninstalled successfully!");
        }
    }
    Ok(())
}

fn ensure_elevation_if_needed(
    required: bool,
    relaunch: bool,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<(), AppError> {
    if !required || sys.is_elevated(logger)? {
        trace_event(
            "elevation_ok",
            json!({"required": required, "relaunch": relaunch}),
        );
        return Ok(());
    }
    if relaunch {
        trace_event("elevation_relaunch", json!({}));
        sys.relaunch_as_admin(logger)?;
        return Err(AppError::Message("__elevated_relaunch__".into()));
    }
    trace_event("elevation_required_error", json!({}));
    Err(AppError::Message(
        "Elevation required for requested operation".into(),
    ))
}

fn build_install_runtime(
    manifest: &InstallManifest,
    manifest_path: &Path,
    journal_path: Option<PathBuf>,
    requires_admin: bool,
    resolver: &win::PathResolver,
) -> Result<InstallRuntime, AppError> {
    let install_root = infer_install_root(manifest, resolver);
    let journal_path = journal_path.unwrap_or_else(|| {
        install_root
            .clone()
            .unwrap_or_else(|| {
                manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            })
            .join("journal.json")
    });
    let uninstall_exe_path = install_root
        .clone()
        .map(|root| root.join("covenant-setup-uninstall.exe"));
    let uninstall_registry_root = if requires_admin {
        RegistryRoot::Hklm
    } else {
        RegistryRoot::Hkcu
    };
    let uninstall_registry_key = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
        sanitize_registry_component(&manifest.app_name)
    );
    Ok(InstallRuntime {
        journal_path,
        install_root,
        uninstall_exe_path,
        uninstall_registry_root,
        uninstall_registry_key,
    })
}

fn infer_install_root(manifest: &InstallManifest, resolver: &win::PathResolver) -> Option<PathBuf> {
    if let Some(path) = manifest.purge.paths.first() {
        return Some(resolver.resolve(path));
    }
    if let Some(directory) = manifest.directories.first() {
        return Some(resolver.resolve(directory));
    }
    if let Some(file) = manifest.files.first() {
        return resolver
            .resolve(&file.destination)
            .parent()
            .map(Path::to_path_buf);
    }
    None
}

fn install_uninstaller(uninstall_exe_path: &Path, logger: &Logger) -> Result<(), AppError> {
    if let Some(parent) = uninstall_exe_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let current_exe = std::env::current_exe()?;
    logger.info(
        "install_uninstaller",
        json!({"source":current_exe,"destination":uninstall_exe_path}),
    );
    fs::copy(current_exe, uninstall_exe_path)?;
    Ok(())
}

fn register_uninstall_entry(
    manifest: &InstallManifest,
    runtime: &InstallRuntime,
    install_root: &Path,
    uninstall_exe_path: &Path,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<(), AppError> {
    let uninstall_command = format!(
        "\"{}\" uninstall \"{}\" --elevate",
        uninstall_exe_path.display(),
        runtime.journal_path.display()
    );
    let values = [
        ("DisplayName", manifest.app_name.clone()),
        ("Publisher", "covenant-setup".to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        (
            "InstallLocation",
            install_root.to_string_lossy().to_string(),
        ),
        (
            "DisplayIcon",
            uninstall_exe_path.to_string_lossy().to_string(),
        ),
        ("UninstallString", uninstall_command.clone()),
        ("QuietUninstallString", uninstall_command),
    ];
    for (name, value) in values {
        logger.info(
            "register_uninstall_value",
            json!({"key":runtime.uninstall_registry_key,"name":name,"value":value}),
        );
        sys.set_registry_string(
            runtime.uninstall_registry_root,
            &runtime.uninstall_registry_key,
            name,
            &value,
            logger,
        )?;
    }
    Ok(())
}

pub(crate) fn spawn_cleanup_helper(
    target_exe: &Path,
    install_root: Option<&Path>,
    app_name: &str,
    ui_mode: UiMode,
    automation: bool,
    json: bool,
    logger: &Logger,
) -> Result<(), AppError> {
    let current_exe = std::env::current_exe()?;
    let helper_path = std::env::temp_dir().join(format!(
        "covenant-setup-cleanup-{}-{}.exe",
        process::id(),
        unique_ticks()
    ));
    logger.info(
        "spawn_cleanup_helper",
        json!({"helper":helper_path,"target_exe":target_exe,"install_root":install_root}),
    );
    fs::copy(&current_exe, &helper_path)?;

    let mut command = Command::new(&helper_path);
    command.creation_flags(CREATE_NO_WINDOW);
    if json {
        command.arg("--json");
    }
    if ui_mode == UiMode::Tui {
        command.arg("--headless");
    } else if ui_mode == UiMode::Gui {
        command.arg("--headed");
    }
    if automation {
        command.arg("--automation");
    }
    command.arg("cleanup");
    command.arg("--target-exe");
    command.arg(target_exe);
    if let Some(install_root) = install_root {
        command.arg("--install-root");
        command.arg(install_root);
    }
    command.arg("--app-name");
    command.arg(app_name);
    command.spawn()?;
    Ok(())
}

fn start_tui_progress(ui_mode: UiMode, label: String) -> Option<TuiProgress> {
    if ui_mode == UiMode::Tui {
        Some(TuiProgress::start(label))
    } else {
        None
    }
}

fn ui_preferences_from_cli(cli: &Cli) -> UiPreferences {
    UiPreferences {
        json: cli.json,
        headless: cli.headless,
        headed: cli.headed,
        automation: cli.automation,
    }
}

fn select_ui(
    preferences: UiPreferences,
    sys: &dyn Sys,
    logger: &Logger,
) -> Result<UiMode, AppError> {
    if preferences.json {
        return Ok(UiMode::None);
    }
    if preferences.headless && preferences.headed {
        return Err(AppError::Message(
            "Pass either --headed or --headless, not both".into(),
        ));
    }
    if preferences.headless {
        return Ok(UiMode::Tui);
    }
    if preferences.headed {
        if !sys.ui_available() {
            logger.info("gui_unavailable_fallback", json!({"fallback": "headless"}));
            return Ok(UiMode::Tui);
        }
        return Ok(UiMode::Gui);
    }
    Err(AppError::Message(
        "UI mode is required. Pass --headed for the WinForms UI, --headless for terminal progress, or --json for machine-readable output".into(),
    ))
}

fn ui_mode_name(ui_mode: UiMode) -> &'static str {
    match ui_mode {
        UiMode::None => "none",
        UiMode::Gui => "gui",
        UiMode::Tui => "tui",
    }
}

fn start_gui_progress(
    ui_mode: UiMode,
    sys: &dyn Sys,
    title: &str,
    app_name: Option<&str>,
    install_root: Option<&Path>,
    total_steps: usize,
    automation: bool,
    logger: &Logger,
) -> Result<Option<Box<dyn ProgressSink>>, AppError> {
    trace_event(
        "gui_progress_start",
        json!({
            "ui_mode": ui_mode_name(ui_mode),
            "title": title,
            "total_steps": total_steps.max(1)
        }),
    );
    if let Some(sink) = sys.start_progress(ui_mode, title, total_steps.max(1))? {
        return Ok(Some(sink));
    }
    if ui_mode == UiMode::Gui {
        Ok(Some(Box::new(ui::GuiProgress::start(
            title,
            app_name,
            install_root
                .map(|p| p.to_string_lossy().into_owned())
                .as_deref(),
            total_steps.max(1),
            automation,
            logger,
        )?)))
    } else {
        Ok(None)
    }
}

fn advance_gui_progress(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    current_step: usize,
    total_steps: usize,
    message: &str,
) -> Result<(), AppError> {
    if gui_progress
        .as_mut()
        .is_some_and(|progress| progress.cancel_requested())
    {
        trace_event(
            "cancel_request_detected",
            json!({"current_step": current_step}),
        );
        return Err(AppError::Cancelled);
    }
    trace_event(
        "progress",
        json!({"current_step": current_step, "message": message}),
    );
    if let Some(progress) = gui_progress.as_mut() {
        progress.advance(current_step, total_steps, message)?;
    }
    Ok(())
}

fn advance_gui_progress_step(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    current_step: &mut usize,
    total_steps: usize,
    message: &str,
) -> Result<(), AppError> {
    *current_step += 1;
    advance_gui_progress(gui_progress, *current_step, total_steps, message)
}

fn finish_gui_progress(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    message: &str,
    wait_for_close: bool,
) -> Result<(), AppError> {
    trace_event("progress_finish", json!({"message": message}));
    if let Some(progress) = gui_progress.as_mut() {
        progress.finish(message, wait_for_close)?;
    }
    Ok(())
}

fn fail_gui_progress(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    app_name: &str,
    operation: &str,
    err: &AppError,
    support_contact: Option<&str>,
    wait_for_close: bool,
) -> Result<(), AppError> {
    let message = format!("Error: program {app_name} failed to {operation} completely!");
    trace_event(
        "progress_fail",
        json!({"app_name": app_name, "operation": operation, "message": message, "error": err.to_string()}),
    );
    if let Some(progress) = gui_progress.as_mut() {
        progress.fail(
            app_name,
            operation,
            &message,
            &err.to_string(),
            error_errata(app_name, operation, err, support_contact),
            wait_for_close,
        )?;
        mark_failure_ux_shown();
    }
    Ok(())
}

fn mark_failure_ux_shown() {
    FAILURE_UX_SHOWN.store(true, Ordering::Relaxed);
}

fn failure_ux_shown() -> bool {
    FAILURE_UX_SHOWN.load(Ordering::Relaxed)
}

fn error_errata(
    app_name: &str,
    operation: &str,
    err: &AppError,
    support_contact: Option<&str>,
) -> serde_json::Value {
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis());
    json!({
        "schema": "covenant_setup_errata_v1",
        "app_name": app_name,
        "operation": operation,
        "timestamp_unix_ms": timestamp_unix_ms,
        "support_contact": support_contact,
        "error": {
            "message": err.to_string(),
            "debug": format!("{err:?}"),
        },
        "process": {
            "pid": process::id(),
            "exe": std::env::current_exe().ok().map(|path| path.display().to_string()),
            "current_dir": std::env::current_dir().ok().map(|path| path.display().to_string()),
            "args": std::env::args_os()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            "trace_dir": std::env::var_os("COVENANT_SETUP_TRACE_DIR")
                .map(|path| path.to_string_lossy().to_string()),
        }
    })
}

fn append_gui_shell_output(
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
    bytes: &[u8],
) -> Result<(), AppError> {
    if bytes.is_empty() {
        return Ok(());
    }
    if let Some(progress) = gui_progress.as_mut() {
        let text = String::from_utf8_lossy(bytes);
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            trace_event("script_output", json!({"line": line}));
            progress.log(line)?;
        }
    }
    Ok(())
}

fn total_install_steps(manifest: &InstallManifest) -> usize {
    manifest.directories.len()
        + manifest.files.len()
        + manifest.registry.len()
        + manifest.shortcuts.len()
        + manifest.scripts.len()
        + 2
}

fn total_uninstall_steps(journal: &Journal) -> usize {
    journal.actions.len() + journal.purge.registry_branches.len() + journal.purge.paths.len() + 2
}

pub(crate) fn schedule_helper_self_cleanup(logger: &Logger) -> Result<bool, AppError> {
    let self_exe = std::env::current_exe()?;
    logger.info("schedule_helper_self_cleanup", json!({"path":self_exe}));
    let delete_command = format!(
        "Start-Sleep -Seconds 2; Remove-Item -LiteralPath '{}' -Force -ErrorAction SilentlyContinue",
        powershell_single_quote(&self_exe.to_string_lossy())
    );
    let mut command = Command::new("powershell.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    command.arg("-NoProfile");
    command.arg("-WindowStyle");
    command.arg("Hidden");
    command.arg("-Command");
    command.arg(OsString::from(delete_command));
    if command.spawn().is_ok() {
        return Ok(false);
    }
    win::remove_file_with_fallback(&self_exe, logger)?;
    Ok(true)
}

pub(crate) fn trace_event(phase: &str, detail: impl Serialize) {
    let Ok(root) = std::env::var("COVENANT_SETUP_TRACE_DIR") else {
        return;
    };
    if root.trim().is_empty() {
        return;
    }

    let root = PathBuf::from(root);
    if fs::create_dir_all(&root).is_err() {
        return;
    }
    let path = root.join(format!("installer-heartbeat-{}.jsonl", process::id()));
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let event = json!({
        "time_unix_ms": timestamp,
        "pid": process::id(),
        "phase": phase,
        "detail": detail
    });
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{event}");
    }
}

fn unique_ticks() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn powershell_single_quote(value: &str) -> String {
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

fn sanitize_registry_component(value: &str) -> String {
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

fn manifest_requires_admin(
    manifest: &InstallManifest,
    resolver: &win::PathResolver,
) -> Result<bool, AppError> {
    for directory in &manifest.directories {
        if resolver.requires_admin(&resolver.resolve(directory)) {
            return Ok(true);
        }
    }
    for file in &manifest.files {
        if resolver.requires_admin(&resolver.resolve(&file.destination)) {
            return Ok(true);
        }
    }
    for shortcut in &manifest.shortcuts {
        if resolver.requires_admin(&resolver.resolve(&shortcut.path)) {
            return Ok(true);
        }
    }
    for key in &manifest.registry {
        let (root, _) = parse_registry_key(&key.key)?;
        if matches!(root, RegistryRoot::Hklm) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn journal_requires_admin(
    journal: &Journal,
    resolver: &win::PathResolver,
) -> Result<bool, AppError> {
    for action in &journal.actions {
        match action {
            JournalAction::CreateDirectory { path }
            | JournalAction::CopyFile {
                destination: path, ..
            }
            | JournalAction::CreateShortcut { path } => {
                if resolver.requires_admin(path) {
                    return Ok(true);
                }
            }
            JournalAction::WriteRegistry { root, .. } if matches!(root, RegistryRoot::Hklm) => {
                return Ok(true);
            }
            _ => {}
        }
    }
    for branch in &journal.purge.registry_branches {
        let (root, _) = parse_registry_key(branch)?;
        if matches!(root, RegistryRoot::Hklm) {
            return Ok(true);
        }
    }
    for path in &journal.purge.paths {
        if resolver.requires_admin(&resolver.resolve(path)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn execute_script(
    script: &ScriptSpec,
    manifest_dir: Option<&Path>,
    working_directory: Option<&Path>,
    gui_progress: &mut Option<Box<dyn ProgressSink>>,
) -> Result<(), AppError> {
    trace_event(
        "script_start",
        json!({"command": &script.command, "args": &script.args, "working_directory": working_directory}),
    );
    let command_path = absolutize(manifest_dir, &script.command);
    let command = if command_path.exists() {
        command_path
    } else {
        PathBuf::from(&script.command)
    };
    let mut process = Command::new(command);
    process.creation_flags(CREATE_NO_WINDOW);
    process.args(&script.args);
    if let Some(dir) = working_directory {
        process.current_dir(dir);
    }
    let output = process.output()?;
    append_gui_shell_output(gui_progress, &output.stdout)?;
    append_gui_shell_output(gui_progress, &output.stderr)?;
    let status = output.status;
    trace_event(
        "script_exit",
        json!({"command": &script.command, "status": status.code()}),
    );
    if !status.success() {
        return Err(AppError::Message(format!(
            "Script failed: {} ({status})",
            script.command
        )));
    }
    Ok(())
}

fn purge_path(path: &Path, sys: &dyn Sys, logger: &Logger) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_file() {
        return sys.remove_file_with_fallback(path, logger);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            purge_path(&child, sys, logger)?;
        } else {
            sys.remove_file_with_fallback(&child, logger)?;
        }
    }
    win::remove_directory_if_exists(path, logger)
}

fn parse_registry_key(input: &str) -> Result<(RegistryRoot, String), AppError> {
    if let Some(rest) = input.strip_prefix("HKCU\\") {
        return Ok((RegistryRoot::Hkcu, rest.to_string()));
    }
    if let Some(rest) = input.strip_prefix("HKLM\\") {
        return Ok((RegistryRoot::Hklm, rest.to_string()));
    }
    Err(AppError::Message(format!(
        "Unsupported registry root: {input}"
    )))
}

fn is_uninstall_registry_key(subkey: &str) -> bool {
    subkey.starts_with("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\")
}

fn push_unique_registry_branch(
    branches: &mut Vec<(RegistryRoot, String)>,
    root: RegistryRoot,
    subkey: String,
) {
    if !branches.iter().any(|(existing_root, existing_subkey)| {
        *existing_root == root && *existing_subkey == subkey
    }) {
        branches.push((root, subkey));
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    if let (Ok(left), Ok(right)) = (fs::canonicalize(left), fs::canonicalize(right)) {
        return normalize_path_for_compare(&left) == normalize_path_for_compare(&right);
    }
    normalize_path_for_compare(left) == normalize_path_for_compare(right)
}

fn normalize_path_for_compare(path: &Path) -> String {
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

fn is_windows_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() == 3 && bytes[1] == b':' && bytes[2] == b'\\')
        || (path.starts_with("\\\\") && path[2..].matches('\\').count() <= 1)
}

fn absolutize(base: Option<&Path>, value: &str) -> PathBuf {
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

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "covenant-setup-test-{name}-{}-{unique}",
                process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

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
            &[generated_installer.clone()],
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
    fn declared_tracker_records_actions_and_finishes_journal() {
        let mut tracker = DeclaredTracker::new();
        tracker.record(JournalAction::CreateDirectory {
            path: PathBuf::from("C:\\Apps\\Tracked"),
        });

        let journal = tracker.finish(
            "Tracked App".to_string(),
            Some(PathBuf::from("install.toml")),
            PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\Tracked".to_string()],
                paths: vec!["C:\\Apps\\Tracked".to_string()],
            },
            None,
        );

        assert_eq!(journal.app_name, "Tracked App");
        assert_eq!(journal.actions.len(), 1);
        assert_eq!(journal.purge.paths, vec!["C:\\Apps\\Tracked"]);
    }

    #[test]
    fn journal_serde_round_trips_all_action_variants() {
        let journal = Journal {
            app_name: "Serde App".to_string(),
            manifest_path: Some(PathBuf::from("C:\\install\\app.toml")),
            actions: vec![
                JournalAction::CreateDirectory {
                    path: PathBuf::from("C:\\Apps\\Serde"),
                },
                JournalAction::CopyFile {
                    source: PathBuf::from("payload\\app.exe"),
                    destination: PathBuf::from("C:\\Apps\\Serde\\app.exe"),
                },
                JournalAction::WriteRegistry {
                    root: RegistryRoot::Hkcu,
                    subkey: "Software\\SerdeApp".to_string(),
                    name: "InstallLocation".to_string(),
                },
                JournalAction::CreateShortcut {
                    path: PathBuf::from("C:\\Users\\Public\\Desktop\\Serde.lnk"),
                },
                JournalAction::ExecuteScript {
                    command: "powershell.exe".to_string(),
                    args: vec!["-NoProfile".to_string(), "-File".to_string()],
                    working_directory: Some(PathBuf::from("C:\\Apps\\Serde")),
                },
            ],
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\Serde".to_string()],
                paths: vec!["C:\\Apps\\Serde\\cache".to_string()],
            },
            support_contact: None,
        };

        let serialized = serde_json::to_string_pretty(&journal).unwrap();
        let decoded: Journal = serde_json::from_str(&serialized).unwrap();

        assert_eq!(decoded, journal);
        assert!(serialized.contains("\"type\": \"create_directory\""));
        assert!(serialized.contains("\"root\": \"hkcu\""));
    }

    #[test]
    fn manifest_deserializes_grouped_directories() {
        let manifest: InstallManifest = toml::from_str(
            r#"
app_name = 'Grouped App'

[directories]
paths = [
  '{LocalAppData}\Grouped',
  '{LocalAppData}\Grouped\bin',
]
"#,
        )
        .unwrap();

        assert_eq!(manifest.directories.len(), 2);
        assert_eq!(manifest.directories[0], r"{LocalAppData}\Grouped");
        assert_eq!(manifest.directories[1], r"{LocalAppData}\Grouped\bin");
    }

    #[test]
    fn manifest_rejects_legacy_directory_tables() {
        let result = toml::from_str::<InstallManifest>(
            r#"
app_name = 'Legacy App'

[[directories]]
path = '{LocalAppData}\Legacy'

[[directories]]
path = '{LocalAppData}\Legacy\bin'
"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn parse_registry_key_accepts_supported_roots_and_rejects_unknown_roots() {
        assert_eq!(
            parse_registry_key("HKCU\\Software\\Example").unwrap(),
            (RegistryRoot::Hkcu, "Software\\Example".to_string())
        );
        assert_eq!(
            parse_registry_key("HKLM\\Software\\Example").unwrap(),
            (RegistryRoot::Hklm, "Software\\Example".to_string())
        );
        assert!(parse_registry_key("HKCR\\Software\\Example").is_err());
    }

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
    fn manifest_file_name_must_match_app_name_convention() {
        let manifest = InstallManifest {
            app_name: "Sample App".to_string(),
            directories: Vec::new(),
            files: Vec::new(),
            registry: Vec::new(),
            shortcuts: Vec::new(),
            scripts: Vec::new(),
            purge: PurgeSpec::default(),
            support_contact: None,
        };

        enforce_manifest_file_name(Path::new("SampleApp-install.toml"), &manifest).unwrap();

        let err = enforce_manifest_file_name(Path::new("install.toml"), &manifest)
            .unwrap_err()
            .to_string();
        assert!(err.contains("SampleApp-install.toml"));

        let err = enforce_manifest_file_name(
            Path::new("manifest folder\\SampleApp-install.toml"),
            &manifest,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("cannot contain spaces"));
    }

    #[test]
    fn manifest_field_spacing_allows_only_app_name_and_description_spaces() {
        let mut manifest = InstallManifest {
            app_name: "Display Name With Spaces".to_string(),
            directories: vec!["{LocalAppData}\\NoSpaces".to_string()],
            files: vec![FileSpec {
                source: "payload\\app.exe".to_string(),
                destination: "{LocalAppData}\\NoSpaces\\app.exe".to_string(),
            }],
            registry: vec![RegistrySpec {
                key: "HKCU\\Software\\NoSpaces".to_string(),
                name: "InstallRoot".to_string(),
                value: "{LocalAppData}\\NoSpaces".to_string(),
            }],
            shortcuts: vec![ShortcutSpec {
                path: "{Desktop}\\NoSpaces.lnk".to_string(),
                target: "{LocalAppData}\\NoSpaces\\app.exe".to_string(),
                arguments: Some("--profile=default".to_string()),
                working_directory: Some("{LocalAppData}\\NoSpaces".to_string()),
                description: Some("Description can have spaces".to_string()),
            }],
            scripts: vec![ScriptSpec {
                command: "powershell.exe".to_string(),
                args: vec![
                    "-ExecutionPolicy".to_string(),
                    "Bypass".to_string(),
                    "-File".to_string(),
                    "payload\\post_install.ps1".to_string(),
                ],
                working_directory: Some("{LocalAppData}\\NoSpaces".to_string()),
            }],
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\NoSpaces".to_string()],
                paths: vec!["{LocalAppData}\\NoSpaces".to_string()],
            },
            support_contact: None,
        };

        enforce_manifest_field_spacing(&manifest).unwrap();

        manifest.scripts[0].args[3] = "payload\\post install.ps1".to_string();
        let err = enforce_manifest_field_spacing(&manifest)
            .unwrap_err()
            .to_string();
        assert!(err.contains("scripts[0].args[3]"));
        assert!(err.contains("cannot contain spaces"));
    }

    #[test]
    fn read_install_manifest_rejects_space_in_manifest_field() {
        let temp = TestDir::new("manifest-field-spaces");
        let manifest_path = temp.path().join("SpaceAllowed-install.toml");
        fs::write(
            &manifest_path,
            r#"
app_name = 'Space Allowed'

[directories]
paths = ['{LocalAppData}\Bad Path']

[[shortcuts]]
path = '{Desktop}\NoSpaces.lnk'
target = '{LocalAppData}\NoSpaces\app.exe'
description = 'Description can have spaces'
"#,
        )
        .unwrap();

        let err = read_install_manifest(&manifest_path)
            .unwrap_err()
            .to_string();

        assert!(err.contains("directories.paths[0]"));
        assert!(err.contains("cannot contain spaces"));
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
    fn cli_parses_bundled_flags_without_manual_preparse() {
        let bundled = Cli::try_parse_from(["setup.exe", "--headed", "--automation"]).unwrap();
        assert!(bundled.command.is_none());
        assert!(bundled.headed);
        assert!(bundled.automation);

        let direct =
            Cli::try_parse_from(["setup.exe", "--headless", "install", "manifest.toml"]).unwrap();
        assert!(matches!(direct.command, Some(Commands::Install { .. })));
        assert!(direct.headless);
    }

    #[test]
    fn run_without_command_reports_missing_command() {
        let cli = Cli::try_parse_from(["setup.exe", "--headless"]).unwrap();
        let err = run(cli, &WinSys, &quiet_logger()).unwrap_err().to_string();

        assert!(err.contains("Missing command"));
    }

    #[test]
    fn run_package_command_creates_packaged_installer() {
        let temp = TestDir::new("run-package");
        let manifest = temp.path().join("RunPackage-install.toml");
        let output = temp.path().join("dist");
        fs::write(&manifest, "app_name = 'Run Package'\n").unwrap();

        let cli = Cli::try_parse_from([
            OsString::from("setup.exe"),
            OsString::from("--json"),
            OsString::from("package"),
            manifest.clone().into_os_string(),
            OsString::from("--output"),
            output.clone().into_os_string(),
        ])
        .unwrap();
        run(cli, &WinSys, &quiet_logger()).unwrap();

        let installer = output.join("covenant-setup-installer.exe");
        assert!(installer.is_file());
        assert_eq!(
            read_embedded_bundle(&installer)
                .unwrap()
                .unwrap()
                .metadata
                .app_name,
            "Run Package"
        );
    }

    #[test]
    fn ui_preferences_and_selection_cover_explicit_modes() {
        let json_cli = Cli::try_parse_from(["setup.exe", "--json", "--headed"]).unwrap();
        let json_preferences = ui_preferences_from_cli(&json_cli);
        assert!(json_preferences.json);
        assert_eq!(
            select_ui(json_preferences, &WinSys, &quiet_logger()).unwrap(),
            UiMode::None
        );

        let headless_cli = Cli::try_parse_from(["setup.exe", "--headless"]).unwrap();
        assert_eq!(
            select_ui(
                ui_preferences_from_cli(&headless_cli),
                &WinSys,
                &quiet_logger()
            )
            .unwrap(),
            UiMode::Tui
        );

        let missing = UiPreferences {
            json: false,
            headless: false,
            headed: false,
            automation: false,
        };
        assert!(select_ui(missing, &WinSys, &quiet_logger()).is_err());

        let conflict = UiPreferences {
            json: false,
            headless: true,
            headed: true,
            automation: false,
        };
        assert!(select_ui(conflict, &WinSys, &quiet_logger()).is_err());
    }

    #[test]
    fn headed_selection_uses_gui_or_documented_fallback() {
        let mode = select_ui(
            UiPreferences {
                json: false,
                headless: false,
                headed: true,
                automation: false,
            },
            &WinSys,
            &quiet_logger(),
        )
        .unwrap();

        assert!(matches!(mode, UiMode::Gui | UiMode::Tui));
    }

    #[test]
    fn ui_mode_names_are_stable() {
        assert_eq!(ui_mode_name(UiMode::None), "none");
        assert_eq!(ui_mode_name(UiMode::Gui), "gui");
        assert_eq!(ui_mode_name(UiMode::Tui), "tui");
    }

    #[test]
    fn logger_methods_and_quiet_clone_are_callable() {
        for json_output in [false, true] {
            let logger = Logger {
                json: json_output,
                quiet: false,
            };
            logger.info("test_event", json!({"value": 1}));
            logger.result("ok", json!({"value": 2}));
            logger.error("boom", 7);

            let quiet = logger.quiet_clone();
            assert!(quiet.quiet);
            assert_eq!(quiet.json, json_output);
            quiet.info("hidden", json!({}));
            quiet.result("hidden", json!({}));
        }
    }

    #[test]
    fn tui_progress_can_start_and_stop() {
        let progress = start_tui_progress(UiMode::Tui, "Testing ".to_string());
        assert!(progress.is_some());
        drop(progress);
        assert!(start_tui_progress(UiMode::None, "Testing ".to_string()).is_none());
    }

    #[test]
    fn progress_helpers_are_noops_without_gui_progress() {
        let mut progress: Option<Box<dyn ProgressSink>> = None;
        let mut step = 0;

        assert!(
            start_gui_progress(
                UiMode::None,
                &WinSys,
                "No UI",
                None,
                None,
                0,
                false,
                &quiet_logger()
            )
            .unwrap()
            .is_none()
        );
        advance_gui_progress(&mut progress, 1, 3, "step").unwrap();
        advance_gui_progress_step(&mut progress, &mut step, 3, "next").unwrap();
        finish_gui_progress(&mut progress, "done", false).unwrap();
        fail_gui_progress(
            &mut progress,
            "App",
            "install",
            &AppError::Message("boom".to_string()),
            None,
            false,
        )
        .unwrap();
        append_gui_shell_output(&mut progress, b"").unwrap();
        append_gui_shell_output(&mut progress, b"line 1\n\nline 2\n").unwrap();

        assert_eq!(step, 1);
    }

    #[test]
    fn failure_ux_marker_tracks_whether_failure_was_shown() {
        FAILURE_UX_SHOWN.store(false, Ordering::Relaxed);
        assert!(!failure_ux_shown());

        mark_failure_ux_shown();
        assert!(failure_ux_shown());

        FAILURE_UX_SHOWN.store(false, Ordering::Relaxed);
    }

    #[test]
    fn embedded_bundle_probe_is_false_for_test_binary() {
        assert!(!has_embedded_bundle());
    }

    #[test]
    fn elevation_not_required_short_circuits_without_admin_probe() {
        ensure_elevation_if_needed(false, false, &WinSys, &quiet_logger()).unwrap();
    }

    #[test]
    fn error_errata_contains_operation_error_and_process_context() {
        let errata = error_errata(
            "Errata App",
            "install",
            &AppError::Message("boom".into()),
            Some("support@example.com"),
        );

        assert_eq!(errata["schema"], "covenant_setup_errata_v1");
        assert_eq!(errata["app_name"], "Errata App");
        assert_eq!(errata["operation"], "install");
        assert_eq!(errata["error"]["message"], "boom");
        assert_eq!(errata["support_contact"], "support@example.com");
        assert!(errata["process"]["pid"].as_u64().is_some());
        assert!(errata["process"]["args"].as_array().is_some());
    }

    #[test]
    fn total_step_helpers_count_manifest_and_journal_work() {
        let manifest = sample_manifest();
        assert_eq!(total_install_steps(&manifest), 7);

        let journal = Journal {
            app_name: "Steps".to_string(),
            manifest_path: None,
            actions: vec![
                JournalAction::CreateDirectory {
                    path: PathBuf::from("C:\\Apps\\Steps"),
                },
                JournalAction::CreateShortcut {
                    path: PathBuf::from("C:\\Users\\Public\\Desktop\\Steps.lnk"),
                },
            ],
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\Steps".to_string()],
                paths: vec!["C:\\Apps\\Steps\\Cache".to_string()],
            },
            support_contact: None,
        };
        assert_eq!(total_uninstall_steps(&journal), 6);
    }

    #[test]
    fn install_empty_manifest_writes_minimal_journal() {
        let temp = TestDir::new("install-empty");
        let manifest_path = temp.path().join("EmptyApp-install.toml");
        fs::write(&manifest_path, "app_name = 'Empty App'\n").unwrap();

        install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true,
            &WinSys,
            None,
            &quiet_logger(),
        )
        .unwrap();

        let journal_path = temp.path().join("journal.json");
        let journal: Journal =
            serde_json::from_str(&fs::read_to_string(journal_path).unwrap()).unwrap();
        assert_eq!(journal.app_name, "Empty App");
        assert_eq!(journal.manifest_path, Some(manifest_path));
        assert!(journal.actions.is_empty());
        assert_eq!(journal.purge, PurgeSpec::default());
    }

    #[test]
    fn uninstall_empty_journal_succeeds_without_actions() {
        let temp = TestDir::new("uninstall-empty");
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "Empty App".to_string(),
            manifest_path: None,
            actions: Vec::new(),
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &WinSys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
    }

    #[test]
    fn execute_script_reports_success_and_failure_status() {
        let mut progress = None;
        let ok = ScriptSpec {
            command: "cmd.exe".to_string(),
            args: vec!["/C".to_string(), "exit 0".to_string()],
            working_directory: None,
        };
        execute_script(&ok, None, None, &mut progress).unwrap();

        let failing = ScriptSpec {
            command: "cmd.exe".to_string(),
            args: vec!["/C".to_string(), "exit 7".to_string()],
            working_directory: None,
        };
        let err = execute_script(&failing, None, None, &mut progress)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Script failed: cmd.exe"));
    }

    #[test]
    fn purge_path_removes_nested_temp_tree_and_noops_when_missing() {
        let temp = TestDir::new("purge-path");
        let root = temp.path().join("root");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("payload.txt"), b"payload").unwrap();

        purge_path(&root, &WinSys, &quiet_logger()).unwrap();
        purge_path(&root, &WinSys, &quiet_logger()).unwrap();

        assert!(!root.exists());
    }

    #[test]
    fn purge_path_removes_single_file() {
        let temp = TestDir::new("purge-file");
        let file = temp.path().join("payload.txt");
        fs::write(&file, b"payload").unwrap();

        purge_path(&file, &WinSys, &quiet_logger()).unwrap();

        assert!(!file.exists());
    }

    #[test]
    fn install_uninstaller_copies_current_exe_to_target() {
        let temp = TestDir::new("install-uninstaller");
        let target = temp.path().join("bin").join("covenant-setup-uninstall.exe");

        install_uninstaller(&target, &quiet_logger()).unwrap();

        assert!(target.is_file());
        assert!(fs::metadata(target).unwrap().len() > 0);
    }

    #[test]
    fn execute_script_uses_manifest_relative_command_and_working_directory() {
        let temp = TestDir::new("script-relative");
        let script = temp.path().join("ok.cmd");
        let working_directory = temp.path().join("wd");
        fs::create_dir_all(&working_directory).unwrap();
        fs::write(&script, "@echo off\r\ncd\r\nexit /B 0\r\n").unwrap();

        let spec = ScriptSpec {
            command: "ok.cmd".to_string(),
            args: Vec::new(),
            working_directory: None,
        };
        let mut progress = None;

        execute_script(
            &spec,
            Some(temp.path()),
            Some(&working_directory),
            &mut progress,
        )
        .unwrap();
    }

    #[test]
    fn install_runtime_uses_inferred_or_explicit_journal_paths() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let manifest = sample_manifest();
        let runtime = build_install_runtime(
            &manifest,
            Path::new("C:\\source\\install.toml"),
            None,
            true,
            &resolver,
        )
        .unwrap();

        assert_eq!(
            runtime.journal_path,
            PathBuf::from("C:\\Apps\\Sample\\journal.json")
        );
        assert_eq!(
            runtime.uninstall_exe_path,
            Some(PathBuf::from(
                "C:\\Apps\\Sample\\covenant-setup-uninstall.exe"
            ))
        );
        assert_eq!(runtime.uninstall_registry_root, RegistryRoot::Hklm);
        assert!(runtime.uninstall_registry_key.ends_with("Sample_App"));

        let explicit = build_install_runtime(
            &manifest,
            Path::new("C:\\source\\install.toml"),
            Some(PathBuf::from("D:\\journal.json")),
            false,
            &resolver,
        )
        .unwrap();
        assert_eq!(explicit.journal_path, PathBuf::from("D:\\journal.json"));
        assert_eq!(explicit.uninstall_registry_root, RegistryRoot::Hkcu);
    }

    #[test]
    fn infer_install_root_prefers_purge_then_directory_then_file_parent() {
        let resolver = win::PathResolver::with_roots_for_test(vec![]);
        let mut manifest = sample_manifest();

        assert_eq!(
            infer_install_root(&manifest, &resolver),
            Some(PathBuf::from("C:\\Apps\\Sample"))
        );

        manifest.purge.paths.clear();
        assert_eq!(
            infer_install_root(&manifest, &resolver),
            Some(PathBuf::from("C:\\Apps\\Sample\\bin"))
        );

        manifest.directories.clear();
        assert_eq!(
            infer_install_root(&manifest, &resolver),
            Some(PathBuf::from("C:\\Apps\\Sample"))
        );

        manifest.files.clear();
        assert_eq!(infer_install_root(&manifest, &resolver), None);
    }

    #[test]
    fn manifest_and_journal_admin_checks_use_resolved_paths_and_hklm() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let mut manifest = sample_manifest();
        manifest.purge.paths = vec!["C:\\Users\\Alice\\App".to_string()];
        manifest.directories = vec!["C:\\Program Files\\Sample".to_string()];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        manifest.directories.clear();
        manifest.files.clear();
        manifest.shortcuts.clear();
        manifest.registry = vec![RegistrySpec {
            key: "HKLM\\Software\\Sample".to_string(),
            name: "Value".to_string(),
            value: "Data".to_string(),
        }];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CopyFile {
                source: PathBuf::from("payload.exe"),
                destination: PathBuf::from("C:\\Program Files\\Sample\\payload.exe"),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: vec![],
            purge: PurgeSpec {
                registry_branches: vec!["HKLM\\Software\\Sample".to_string()],
                paths: vec![],
            },
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());
    }

    #[test]
    fn manifest_and_journal_admin_checks_return_false_for_user_scope_work() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let manifest = InstallManifest {
            app_name: "User App".to_string(),
            directories: vec!["C:\\Users\\Alice\\AppData\\Local\\UserApp".to_string()],
            files: vec![FileSpec {
                source: "app.exe".to_string(),
                destination: "C:\\Users\\Alice\\AppData\\Local\\UserApp\\app.exe".to_string(),
            }],
            registry: vec![RegistrySpec {
                key: "HKCU\\Software\\UserApp".to_string(),
                name: "InstallLocation".to_string(),
                value: "C:\\Users\\Alice\\AppData\\Local\\UserApp".to_string(),
            }],
            shortcuts: vec![ShortcutSpec {
                path: "C:\\Users\\Alice\\Desktop\\UserApp.lnk".to_string(),
                target: "C:\\Users\\Alice\\AppData\\Local\\UserApp\\app.exe".to_string(),
                arguments: None,
                working_directory: None,
                description: None,
            }],
            scripts: Vec::new(),
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\UserApp".to_string()],
                paths: vec!["C:\\Users\\Alice\\AppData\\Local\\UserApp".to_string()],
            },
            support_contact: None,
        };
        assert!(!manifest_requires_admin(&manifest, &resolver).unwrap());

        let journal = Journal {
            app_name: "User App".to_string(),
            manifest_path: None,
            actions: vec![
                JournalAction::CreateDirectory {
                    path: PathBuf::from("C:\\Users\\Alice\\AppData\\Local\\UserApp"),
                },
                JournalAction::WriteRegistry {
                    root: RegistryRoot::Hkcu,
                    subkey: "Software\\UserApp".to_string(),
                    name: "InstallLocation".to_string(),
                },
            ],
            purge: manifest.purge,
            support_contact: None,
        };
        assert!(!journal_requires_admin(&journal, &resolver).unwrap());
    }

    #[test]
    fn admin_checks_detect_file_shortcut_and_purge_paths() {
        let resolver =
            win::PathResolver::with_roots_for_test(vec![PathBuf::from("C:\\Program Files")]);
        let mut manifest = sample_manifest();
        manifest.directories.clear();
        manifest.registry.clear();
        manifest.files = vec![FileSpec {
            source: "payload.exe".to_string(),
            destination: "C:\\Program Files\\Sample\\payload.exe".to_string(),
        }];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        manifest.files.clear();
        manifest.shortcuts = vec![ShortcutSpec {
            path: "C:\\Program Files\\Sample\\Sample.lnk".to_string(),
            target: "C:\\Users\\Alice\\App\\app.exe".to_string(),
            arguments: None,
            working_directory: None,
            description: None,
        }];
        assert!(manifest_requires_admin(&manifest, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CreateShortcut {
                path: PathBuf::from("C:\\Program Files\\Sample\\Sample.lnk"),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());

        let journal = Journal {
            app_name: "Admin".to_string(),
            manifest_path: None,
            actions: Vec::new(),
            purge: PurgeSpec {
                registry_branches: Vec::new(),
                paths: vec!["C:\\Program Files\\Sample".to_string()],
            },
            support_contact: None,
        };
        assert!(journal_requires_admin(&journal, &resolver).unwrap());
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

    #[test]
    fn uninstall_registry_key_detection_matches_only_uninstall_branch() {
        assert!(is_uninstall_registry_key(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sample"
        ));
        assert!(!is_uninstall_registry_key("Software\\Sample"));
    }

    #[test]
    fn push_unique_registry_branch_deduplicates_root_and_subkey() {
        let mut branches = Vec::new();
        push_unique_registry_branch(
            &mut branches,
            RegistryRoot::Hkcu,
            "Software\\App".to_string(),
        );
        push_unique_registry_branch(
            &mut branches,
            RegistryRoot::Hkcu,
            "Software\\App".to_string(),
        );
        push_unique_registry_branch(
            &mut branches,
            RegistryRoot::Hklm,
            "Software\\App".to_string(),
        );

        assert_eq!(branches.len(), 2);
    }

    // -------------------------------------------------------------------------
    // Mock infrastructure for the Sys trait + ProgressSink trait. These mocks
    // record every boundary call the orchestration code makes so tests can
    // assert on exact sequences without spawning Win32 / process / GUI side
    // effects.
    // -------------------------------------------------------------------------

    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SysCall {
        IsElevated,
        RelaunchAsAdmin,
        SpawnReboot,
        PromptRebootTui,
        SpawnCleanupHelper {
            target_exe: PathBuf,
            install_root: Option<PathBuf>,
            app_name: String,
            ui_mode: UiMode,
            automation: bool,
            json: bool,
        },
        ScheduleHelperSelfCleanup,
        SetRegistryString {
            root: RegistryRoot,
            subkey: String,
            name: String,
            value: String,
        },
        DeleteRegistryTree {
            root: RegistryRoot,
            subkey: String,
        },
        HasEmbeddedBundle,
        UiAvailable,
        UiReportError(String),
        UiReportUninstallSuccess(String),
        UiPromptUninstallReboot(String),
        RemoveFileWithFallback(PathBuf),
        StartProgress {
            ui_mode: UiMode,
            title: String,
            total_steps: usize,
        },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SinkCall {
        Advance {
            current_step: usize,
            total_steps: usize,
            message: String,
        },
        Log(String),
        Finish {
            message: String,
            wait_for_close: bool,
        },
        Fail {
            app_name: String,
            operation: String,
            message: String,
            error: String,
            wait_for_close: bool,
        },
    }

    #[derive(Default)]
    struct MockProgressSink {
        calls: Arc<Mutex<Vec<SinkCall>>>,
        // When Some(n), cancel_requested() reports true once n Advance calls
        // have been recorded; clear_cancel_request() resets it to None.
        cancel_after_advances: Option<usize>,
    }

    impl MockProgressSink {
        fn new() -> Self {
            Self::default()
        }

        fn handle(&self) -> Arc<Mutex<Vec<SinkCall>>> {
            self.calls.clone()
        }

        fn recorded_advances(&self) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, SinkCall::Advance { .. }))
                .count()
        }
    }

    impl ProgressSink for MockProgressSink {
        fn advance(
            &mut self,
            current_step: usize,
            total_steps: usize,
            message: &str,
        ) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(SinkCall::Advance {
                current_step,
                total_steps,
                message: message.to_string(),
            });
            Ok(())
        }

        fn log(&mut self, message: &str) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SinkCall::Log(message.to_string()));
            Ok(())
        }

        fn finish(&mut self, message: &str, wait_for_close: bool) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(SinkCall::Finish {
                message: message.to_string(),
                wait_for_close,
            });
            Ok(())
        }

        fn cancel_requested(&mut self) -> bool {
            self.cancel_after_advances
                .is_some_and(|threshold| self.recorded_advances() >= threshold)
        }

        fn clear_cancel_request(&mut self) {
            self.cancel_after_advances = None;
        }

        fn fail(
            &mut self,
            app_name: &str,
            operation: &str,
            message: &str,
            error: &str,
            _errata: serde_json::Value,
            wait_for_close: bool,
        ) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(SinkCall::Fail {
                app_name: app_name.to_string(),
                operation: operation.to_string(),
                message: message.to_string(),
                error: error.to_string(),
                wait_for_close,
            });
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockSys {
        calls: Mutex<Vec<SysCall>>,
        is_elevated: Mutex<bool>,
        ui_available: Mutex<bool>,
        ui_prompt_uninstall_reboot: Mutex<bool>,
        schedule_helper_self_cleanup: Mutex<bool>,
        prompt_reboot_tui: Mutex<bool>,
        has_embedded_bundle: Mutex<bool>,
        progress_sink_calls: Mutex<Option<Arc<Mutex<Vec<SinkCall>>>>>,
        progress_sink_cancel_after: Mutex<Option<usize>>,
    }

    #[allow(dead_code)]
    impl MockSys {
        fn new() -> Self {
            Self::default()
        }

        fn recorded(&self) -> Vec<SysCall> {
            self.calls.lock().unwrap().clone()
        }

        fn set_is_elevated(&self, value: bool) {
            *self.is_elevated.lock().unwrap() = value;
        }

        fn set_ui_available(&self, value: bool) {
            *self.ui_available.lock().unwrap() = value;
        }

        fn set_ui_prompt_uninstall_reboot(&self, value: bool) {
            *self.ui_prompt_uninstall_reboot.lock().unwrap() = value;
        }

        fn set_schedule_helper_self_cleanup(&self, value: bool) {
            *self.schedule_helper_self_cleanup.lock().unwrap() = value;
        }

        fn set_prompt_reboot_tui(&self, value: bool) {
            *self.prompt_reboot_tui.lock().unwrap() = value;
        }

        fn install_progress_sink(&self) -> Arc<Mutex<Vec<SinkCall>>> {
            let sink = MockProgressSink::new();
            let handle = sink.handle();
            *self.progress_sink_calls.lock().unwrap() = Some(handle.clone());
            // Box and stash a fresh sink each call to start_progress; use the
            // shared handle so tests can read the recorded calls.
            handle
        }

        fn set_cancel_after_advances(&self, threshold: usize) {
            *self.progress_sink_cancel_after.lock().unwrap() = Some(threshold);
        }
    }

    impl Sys for MockSys {
        fn is_elevated(&self, _logger: &Logger) -> Result<bool, AppError> {
            self.calls.lock().unwrap().push(SysCall::IsElevated);
            Ok(*self.is_elevated.lock().unwrap())
        }

        fn relaunch_as_admin(&self, _logger: &Logger) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(SysCall::RelaunchAsAdmin);
            Ok(())
        }

        fn spawn_reboot(&self, _logger: &Logger) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(SysCall::SpawnReboot);
            Ok(())
        }

        fn prompt_reboot_tui(&self) -> Result<bool, AppError> {
            self.calls.lock().unwrap().push(SysCall::PromptRebootTui);
            Ok(*self.prompt_reboot_tui.lock().unwrap())
        }

        fn spawn_cleanup_helper(
            &self,
            target_exe: &Path,
            install_root: Option<&Path>,
            app_name: &str,
            ui_mode: UiMode,
            automation: bool,
            json: bool,
            _logger: &Logger,
        ) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SysCall::SpawnCleanupHelper {
                    target_exe: target_exe.to_path_buf(),
                    install_root: install_root.map(Path::to_path_buf),
                    app_name: app_name.to_string(),
                    ui_mode,
                    automation,
                    json,
                });
            Ok(())
        }

        fn schedule_helper_self_cleanup(&self, _logger: &Logger) -> Result<bool, AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SysCall::ScheduleHelperSelfCleanup);
            Ok(*self.schedule_helper_self_cleanup.lock().unwrap())
        }

        fn set_registry_string(
            &self,
            root: RegistryRoot,
            subkey: &str,
            name: &str,
            value: &str,
            _logger: &Logger,
        ) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(SysCall::SetRegistryString {
                root,
                subkey: subkey.to_string(),
                name: name.to_string(),
                value: value.to_string(),
            });
            Ok(())
        }

        fn delete_registry_tree(
            &self,
            root: RegistryRoot,
            subkey: &str,
            _logger: &Logger,
        ) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SysCall::DeleteRegistryTree {
                    root,
                    subkey: subkey.to_string(),
                });
            Ok(())
        }

        fn has_embedded_bundle(&self) -> bool {
            self.calls.lock().unwrap().push(SysCall::HasEmbeddedBundle);
            *self.has_embedded_bundle.lock().unwrap()
        }

        fn ui_available(&self) -> bool {
            self.calls.lock().unwrap().push(SysCall::UiAvailable);
            *self.ui_available.lock().unwrap()
        }

        fn ui_report_error(&self, message: &str) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SysCall::UiReportError(message.to_string()));
            Ok(())
        }

        fn ui_report_uninstall_success(&self, app_name: &str) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SysCall::UiReportUninstallSuccess(app_name.to_string()));
            Ok(())
        }

        fn ui_prompt_uninstall_reboot(&self, app_name: &str) -> Result<bool, AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SysCall::UiPromptUninstallReboot(app_name.to_string()));
            Ok(*self.ui_prompt_uninstall_reboot.lock().unwrap())
        }

        fn remove_file_with_fallback(&self, path: &Path, _logger: &Logger) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(SysCall::RemoveFileWithFallback(path.to_path_buf()));
            // Best-effort delete the real file so subsequent fs::read_dir checks
            // in cleanup() see the directory as empty.
            let _ = std::fs::remove_file(path);
            Ok(())
        }

        fn start_progress(
            &self,
            ui_mode: UiMode,
            title: &str,
            total_steps: usize,
        ) -> Result<Option<Box<dyn ProgressSink>>, AppError> {
            self.calls.lock().unwrap().push(SysCall::StartProgress {
                ui_mode,
                title: title.to_string(),
                total_steps,
            });
            // Only inject a recording sink when test code explicitly opted in.
            if let Some(handle) = self.progress_sink_calls.lock().unwrap().clone() {
                let sink = MockProgressSink {
                    calls: handle.clone(),
                    cancel_after_advances: *self.progress_sink_cancel_after.lock().unwrap(),
                };
                Ok(Some(Box::new(sink) as Box<dyn ProgressSink>))
            } else {
                Ok(None)
            }
        }
    }

    // (a)
    #[test]
    fn ensure_elevation_if_needed_relaunches_when_required_and_relaunch_flag_set() {
        let sys = MockSys::new();
        sys.set_is_elevated(false);
        let err = ensure_elevation_if_needed(true, true, &sys, &quiet_logger()).unwrap_err();
        assert_eq!(err.to_string(), "__elevated_relaunch__");
        let calls = sys.recorded();
        assert!(matches!(calls[0], SysCall::IsElevated));
        assert!(matches!(calls[1], SysCall::RelaunchAsAdmin));
    }

    // (b)
    #[test]
    fn ensure_elevation_if_needed_errors_when_required_and_no_relaunch() {
        let sys = MockSys::new();
        sys.set_is_elevated(false);
        let err = ensure_elevation_if_needed(true, false, &sys, &quiet_logger()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Elevation required"));
        let calls = sys.recorded();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0], SysCall::IsElevated));
    }

    // (c)
    #[test]
    fn ensure_elevation_if_needed_passes_when_already_elevated() {
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        ensure_elevation_if_needed(true, true, &sys, &quiet_logger()).unwrap();
        let calls = sys.recorded();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0], SysCall::IsElevated));
    }

    // (d)
    #[test]
    fn cleanup_prompts_and_spawns_reboot_when_required_in_gui_mode() {
        let temp = TestDir::new("cleanup-gui-reboot");
        let target_exe = temp.path().join("ghost.exe");
        // Don't create the file — cleanup() short-circuits on !exists().
        let sys = MockSys::new();
        sys.set_schedule_helper_self_cleanup(true);
        sys.set_ui_prompt_uninstall_reboot(true);
        cleanup(
            target_exe,
            None,
            "Sample".to_string(),
            UiMode::Gui,
            false,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let calls = sys.recorded();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SysCall::ScheduleHelperSelfCleanup))
        );
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SysCall::UiPromptUninstallReboot(name) if name == "Sample"))
        );
        assert!(calls.iter().any(|c| matches!(c, SysCall::SpawnReboot)));
    }

    // (e)
    #[test]
    fn cleanup_skips_reboot_when_user_declines() {
        let temp = TestDir::new("cleanup-decline");
        let target_exe = temp.path().join("ghost.exe");
        let sys = MockSys::new();
        sys.set_schedule_helper_self_cleanup(true);
        sys.set_ui_prompt_uninstall_reboot(false);
        cleanup(
            target_exe,
            None,
            "Sample".to_string(),
            UiMode::Gui,
            false,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let calls = sys.recorded();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SysCall::UiPromptUninstallReboot(_)))
        );
        assert!(!calls.iter().any(|c| matches!(c, SysCall::SpawnReboot)));
    }

    // (f)
    #[test]
    fn cleanup_tui_path_skips_prompt_when_no_reboot_needed() {
        let temp = TestDir::new("cleanup-tui-no-reboot");
        let target_exe = temp.path().join("ghost.exe");
        let sys = MockSys::new();
        sys.set_schedule_helper_self_cleanup(false);
        cleanup(
            target_exe,
            None,
            "Sample".to_string(),
            UiMode::Tui,
            false,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let calls = sys.recorded();
        assert!(
            !calls
                .iter()
                .any(|c| matches!(c, SysCall::UiPromptUninstallReboot(_)))
        );
        assert!(!calls.iter().any(|c| matches!(c, SysCall::PromptRebootTui)));
        assert!(!calls.iter().any(|c| matches!(c, SysCall::SpawnReboot)));
    }

    // (g)
    #[test]
    fn register_uninstall_entry_writes_all_seven_values() {
        let manifest = InstallManifest {
            app_name: "Sample".to_string(),
            directories: Vec::new(),
            files: Vec::new(),
            registry: Vec::new(),
            shortcuts: Vec::new(),
            scripts: Vec::new(),
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        let runtime = InstallRuntime {
            journal_path: PathBuf::from("C:\\fake\\journal.json"),
            install_root: Some(PathBuf::from("C:\\Apps\\Sample")),
            uninstall_exe_path: Some(PathBuf::from("C:\\Apps\\Sample\\uninstall.exe")),
            uninstall_registry_root: RegistryRoot::Hkcu,
            uninstall_registry_key:
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sample".to_string(),
        };
        let install_root = PathBuf::from("C:\\Apps\\Sample");
        let uninstall_exe = PathBuf::from("C:\\Apps\\Sample\\uninstall.exe");
        let sys = MockSys::new();
        register_uninstall_entry(
            &manifest,
            &runtime,
            &install_root,
            &uninstall_exe,
            &sys,
            &quiet_logger(),
        )
        .unwrap();
        let writes: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::SetRegistryString { name, .. } => Some(name),
                _ => None,
            })
            .collect();
        assert_eq!(
            writes,
            vec![
                "DisplayName",
                "Publisher",
                "DisplayVersion",
                "InstallLocation",
                "DisplayIcon",
                "UninstallString",
                "QuietUninstallString",
            ]
        );
    }

    // (h) — substitute: run() dispatches Install subcommand to install() through Sys.
    #[test]
    fn run_install_subcommand_uses_sys_for_registry_writes() {
        let temp = TestDir::new("run-install-sys");
        let manifest_path = temp.path().join("Mocked-install.toml");
        fs::write(
            &manifest_path,
            "app_name = 'Mocked'\n[[registry]]\nkey = 'HKCU\\\\Software\\\\Mocked'\nname = 'Foo'\nvalue = 'Bar'\n",
        )
        .unwrap();
        let journal_path = temp.path().join("journal.json");
        let cli = Cli {
            json: true,
            headed: false,
            headless: false,
            automation: true,
            elevate: false,
            command: Some(Commands::Install {
                manifest: manifest_path.clone(),
                journal: Some(journal_path),
            }),
        };
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        run(cli, &sys, &quiet_logger()).unwrap();
        let writes: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::SetRegistryString { name, value, .. } => Some((name, value)),
                _ => None,
            })
            .collect();
        assert!(writes.iter().any(|(n, v)| n == "Foo" && v == "Bar"));
    }

    // (i) — substitute: install error path emits ui_report_error via run_bundled_installer
    // is exercised at the install layer: a missing manifest yields AppError::Io and the
    // automation flag suppresses the GUI fail UX. We assert the error propagates.
    #[test]
    fn install_with_missing_manifest_propagates_error_without_success_ui() {
        let sys = MockSys::new();
        let err = install(
            Path::new("C:\\does\\not\\exist\\install.toml"),
            None,
            false,
            UiMode::None,
            true,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Io(_) | AppError::Message(_)));
    }

    #[test]
    fn install_failure_triggers_partial_rollback() {
        let temp = TestDir::new("install-rollback-test");
        let manifest_path = temp.path().join("RollbackApp-install.toml");
        let dir_to_create = temp.path().join("CreatedDir");

        // Manifest specifies creating a directory, then copying a non-existent file (which will fail)
        fs::write(
            &manifest_path,
            format!(
                "app_name = 'RollbackApp'\ndirectories = {{ paths = ['{}'] }}\n[[files]]\nsource = 'nonexistent.txt'\ndestination = '{}\\\\file.txt'\n",
                dir_to_create.display().to_string().replace("\\", "\\\\"),
                dir_to_create.display().to_string().replace("\\", "\\\\")
            ),
        )
        .unwrap();

        let sys = MockSys::new();
        sys.set_is_elevated(true);

        let err = install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true, // automation = true
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();

        // The installation must have failed
        assert!(
            err.to_string().contains("nonexistent.txt")
                || err.to_string().contains("cannot find")
                || err.to_string().contains("system cannot find")
        );

        // The directory created during the partial install must have been rolled back (deleted)
        assert!(!dir_to_create.exists());
    }

    #[test]
    fn install_cancel_triggers_rollback_and_returns_cancelled() {
        let temp = TestDir::new("install-cancel-test");
        let manifest_path = temp.path().join("CancelApp-install.toml");
        let dir_to_create = temp.path().join("CreatedDir");
        let payload = temp.path().join("payload.txt");
        fs::write(&payload, b"data").unwrap();

        fs::write(
            &manifest_path,
            format!(
                "app_name = 'CancelApp'\ndirectories = {{ paths = ['{}'] }}\n[[files]]\nsource = '{}'\ndestination = '{}\\\\file.txt'\n",
                dir_to_create.display().to_string().replace("\\", "\\\\"),
                payload.display().to_string().replace("\\", "\\\\"),
                dir_to_create.display().to_string().replace("\\", "\\\\")
            ),
        )
        .unwrap();

        let sys = MockSys::new();
        sys.set_is_elevated(true);
        let sink_calls = sys.install_progress_sink();
        // Trip the cancel flag once the first mutation step has been reported,
        // as if the user clicked Cancel between steps.
        sys.set_cancel_after_advances(1);

        let err = install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            false,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();

        assert!(matches!(err, AppError::Cancelled));
        assert!(!dir_to_create.exists());

        let calls = sink_calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SinkCall::Log(m) if m.contains("Cancel requested")))
        );
        assert!(calls.iter().any(
            |c| matches!(c, SinkCall::Advance { message, .. } if message.contains("Removing directory"))
        ));
        let finishes: Vec<_> = calls
            .iter()
            .filter_map(|c| match c {
                SinkCall::Finish {
                    message,
                    wait_for_close,
                } => Some((message.clone(), *wait_for_close)),
                _ => None,
            })
            .collect();
        assert_eq!(finishes.len(), 1);
        assert!(finishes[0].0.contains("cancelled"));
        assert!(finishes[0].1, "non-automation finish must wait for close");
        assert!(!calls.iter().any(|c| matches!(c, SinkCall::Fail { .. })));
    }

    #[test]
    fn install_failure_rolls_back_through_same_sink_without_second_session() {
        let temp = TestDir::new("install-rollback-same-sink");
        let manifest_path = temp.path().join("SameSinkApp-install.toml");
        let dir_to_create = temp.path().join("CreatedDir");

        fs::write(
            &manifest_path,
            format!(
                "app_name = 'SameSinkApp'\ndirectories = {{ paths = ['{}'] }}\n[[files]]\nsource = 'nonexistent.txt'\ndestination = '{}\\\\file.txt'\n",
                dir_to_create.display().to_string().replace("\\", "\\\\"),
                dir_to_create.display().to_string().replace("\\", "\\\\")
            ),
        )
        .unwrap();

        let sys = MockSys::new();
        sys.set_is_elevated(true);
        let sink_calls = sys.install_progress_sink();

        let err = install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap_err();
        assert!(!matches!(err, AppError::Cancelled));
        assert!(!dir_to_create.exists());

        // The rollback must reuse the live session instead of starting a second one.
        let start_progress_calls = sys
            .recorded()
            .iter()
            .filter(|c| matches!(c, SysCall::StartProgress { .. }))
            .count();
        assert_eq!(start_progress_calls, 1);

        let calls = sink_calls.lock().unwrap().clone();
        assert!(calls.iter().any(
            |c| matches!(c, SinkCall::Advance { message, .. } if message.contains("Removing directory"))
        ));
        let fails = calls
            .iter()
            .filter(|c| matches!(c, SinkCall::Fail { .. }))
            .count();
        assert_eq!(fails, 1, "exactly one terminal fail message");
        assert!(!calls.iter().any(|c| matches!(c, SinkCall::Finish { .. })));
    }

    #[test]
    fn install_success_finish_wait_for_close_follows_automation_flag() {
        for automation in [false, true] {
            let temp = TestDir::new(if automation {
                "install-finish-wait-auto"
            } else {
                "install-finish-wait-manual"
            });
            let manifest_path = temp.path().join("FinishApp-install.toml");
            let dir_to_create = temp.path().join("CreatedDir");
            fs::write(
                &manifest_path,
                format!(
                    "app_name = 'FinishApp'\ndirectories = {{ paths = ['{}'] }}\n",
                    dir_to_create.display().to_string().replace("\\", "\\\\")
                ),
            )
            .unwrap();

            let sys = MockSys::new();
            sys.set_is_elevated(true);
            let sink_calls = sys.install_progress_sink();

            install(
                &manifest_path,
                None,
                false,
                UiMode::None,
                automation,
                &sys,
                None,
                &quiet_logger(),
            )
            .unwrap();

            let calls = sink_calls.lock().unwrap().clone();
            let wait_for_close = calls
                .iter()
                .find_map(|c| match c {
                    SinkCall::Finish { wait_for_close, .. } => Some(*wait_for_close),
                    _ => None,
                })
                .expect("finish recorded");
            assert_eq!(wait_for_close, !automation);
        }
    }

    #[test]
    fn error_exit_code_maps_cancelled_elevation_and_default() {
        assert_eq!(error_exit_code(&AppError::Cancelled), EXIT_CANCELLED);
        assert_eq!(
            error_exit_code(&AppError::Message(
                "Elevation required to write to HKLM".into()
            )),
            EXIT_ELEVATION_REQUIRED
        );
        assert_eq!(
            error_exit_code(&AppError::Message("boom".into())),
            EXIT_OPERATION_FAILED
        );
    }

    #[test]
    fn advance_gui_progress_returns_cancelled_when_sink_requests() {
        let handle = Arc::new(Mutex::new(Vec::new()));
        let sink = MockProgressSink {
            calls: handle.clone(),
            cancel_after_advances: Some(0),
        };
        let mut progress: Option<Box<dyn ProgressSink>> = Some(Box::new(sink));

        let err = advance_gui_progress(&mut progress, 1, 3, "step").unwrap_err();
        assert!(matches!(err, AppError::Cancelled));
        assert!(handle.lock().unwrap().is_empty());

        progress.as_mut().unwrap().clear_cancel_request();
        advance_gui_progress(&mut progress, 1, 3, "step").unwrap();
        assert_eq!(handle.lock().unwrap().len(), 1);
    }

    #[test]
    fn uninstall_cancel_mid_run_keeps_journal_and_reports_uninstall_failure() {
        let temp = TestDir::new("uninstall-cancel");
        let copied = temp.path().join("copied.bin");
        fs::write(&copied, b"x").unwrap();
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "CancelApp".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CopyFile {
                source: temp.path().join("source.bin"),
                destination: copied.clone(),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();

        let handle = Arc::new(Mutex::new(Vec::new()));
        let sink = MockProgressSink {
            calls: handle.clone(),
            cancel_after_advances: Some(0),
        };
        let mut progress: Option<Box<dyn ProgressSink>> = Some(Box::new(sink));
        let sys = MockSys::new();
        sys.set_is_elevated(true);

        let err = uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut progress,
            &quiet_logger(),
            false,
        )
        .unwrap_err();

        assert!(matches!(err, AppError::Cancelled));
        assert!(
            journal_path.exists(),
            "journal must survive a cancelled uninstall"
        );
        assert!(copied.exists(), "no actions ran before the cancel tripped");
        let calls = handle.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, SinkCall::Fail { operation, .. } if operation == "uninstall"))
        );
    }

    // (j)
    #[test]
    fn install_emits_set_registry_string_calls_for_each_registry_spec() {
        let temp = TestDir::new("install-registry-mock");
        let manifest_path = temp.path().join("RegApp-install.toml");
        fs::write(
            &manifest_path,
            "app_name = 'RegApp'\n[[registry]]\nkey = 'HKCU\\\\Software\\\\RegApp'\nname = 'Alpha'\nvalue = 'A'\n[[registry]]\nkey = 'HKCU\\\\Software\\\\RegApp'\nname = 'Beta'\nvalue = 'B'\n",
        )
        .unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        install(
            &manifest_path,
            None,
            false,
            UiMode::None,
            true,
            &sys,
            None,
            &quiet_logger(),
        )
        .unwrap();
        let manifest_writes: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::SetRegistryString { name, value, .. }
                    if name == "Alpha" || name == "Beta" =>
                {
                    Some((name, value))
                }
                _ => None,
            })
            .collect();
        assert_eq!(manifest_writes.len(), 2);
        assert!(
            manifest_writes
                .iter()
                .any(|(n, v)| n == "Alpha" && v == "A")
        );
        assert!(manifest_writes.iter().any(|(n, v)| n == "Beta" && v == "B"));
    }

    // (k)
    #[test]
    fn uninstall_calls_delete_registry_tree_for_recorded_actions_and_purge() {
        let temp = TestDir::new("uninstall-deltree");
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "RegApp".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::WriteRegistry {
                root: RegistryRoot::Hkcu,
                subkey: "Software\\RegApp".to_string(),
                name: "Alpha".to_string(),
            }],
            purge: PurgeSpec {
                registry_branches: vec!["HKCU\\Software\\Purged".to_string()],
                paths: vec![],
            },
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
        let trees: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::DeleteRegistryTree { subkey, .. } => Some(subkey),
                _ => None,
            })
            .collect();
        assert!(trees.iter().any(|s| s == "Software\\RegApp"));
        assert!(trees.iter().any(|s| s == "Software\\Purged"));
    }

    // (l)
    #[test]
    fn uninstall_calls_remove_file_with_fallback_for_copy_actions_and_shortcuts() {
        let temp = TestDir::new("uninstall-rmfile");
        let copied = temp.path().join("copied.bin");
        let shortcut = temp.path().join("Shortcut.lnk");
        fs::write(&copied, b"x").unwrap();
        fs::write(&shortcut, b"x").unwrap();
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "FileApp".to_string(),
            manifest_path: None,
            actions: vec![
                JournalAction::CopyFile {
                    source: temp.path().join("source.bin"),
                    destination: copied.clone(),
                },
                JournalAction::CreateShortcut {
                    path: shortcut.clone(),
                },
            ],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
        let removed: Vec<_> = sys
            .recorded()
            .into_iter()
            .filter_map(|c| match c {
                SysCall::RemoveFileWithFallback(p) => Some(p),
                _ => None,
            })
            .collect();
        assert!(removed.iter().any(|p| p == &copied));
        assert!(removed.iter().any(|p| p == &shortcut));
    }

    // (m): the self-delete branch is triggered when a CopyFile destination
    // matches the current exe — uninstall() then calls spawn_cleanup_helper.
    #[test]
    fn uninstall_defers_self_delete_to_spawn_cleanup_helper() {
        let temp = TestDir::new("uninstall-self");
        let current_exe = std::env::current_exe().unwrap();
        let journal_path = temp.path().join("journal.json");
        let journal = Journal {
            app_name: "SelfApp".to_string(),
            manifest_path: None,
            actions: vec![JournalAction::CopyFile {
                source: temp.path().join("source.exe"),
                destination: current_exe.clone(),
            }],
            purge: PurgeSpec::default(),
            support_contact: None,
        };
        fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();
        let sys = MockSys::new();
        sys.set_is_elevated(true);
        uninstall(
            &journal_path,
            false,
            UiMode::None,
            true,
            &sys,
            &mut None,
            &quiet_logger(),
            false,
        )
        .unwrap();
        let helper = sys
            .recorded()
            .into_iter()
            .find(|c| matches!(c, SysCall::SpawnCleanupHelper { .. }))
            .expect("expected SpawnCleanupHelper recorded");
        match helper {
            SysCall::SpawnCleanupHelper {
                target_exe,
                app_name,
                ..
            } => {
                assert!(same_path(&target_exe, &current_exe));
                assert_eq!(app_name, "SelfApp");
            }
            _ => unreachable!(),
        }
    }

    // (n)
    #[test]
    fn progress_sink_mock_records_calls_through_advance_log_finish_fail() {
        let recorder = MockProgressSink::new();
        let handle = recorder.handle();
        let mut sink: Box<dyn ProgressSink> = Box::new(recorder);
        sink.advance(2, 5, "step 2").unwrap();
        sink.log("note").unwrap();
        sink.finish("done", true).unwrap();
        sink.fail(
            "App",
            "install",
            "boom",
            "io error",
            serde_json::json!({"k":"v"}),
            true,
        )
        .unwrap();
        let calls = handle.lock().unwrap().clone();
        assert_eq!(calls.len(), 4);
        assert!(matches!(
            &calls[0],
            SinkCall::Advance { current_step: 2, total_steps: 5, message } if message == "step 2"
        ));
        assert!(matches!(&calls[1], SinkCall::Log(s) if s == "note"));
        assert!(matches!(
            &calls[2],
            SinkCall::Finish { message, wait_for_close } if message == "done" && *wait_for_close
        ));
        assert!(matches!(
            &calls[3],
            SinkCall::Fail { app_name, operation, error, wait_for_close, .. }
                if app_name == "App" && operation == "install" && error == "io error" && *wait_for_close
        ));
    }

    fn sample_manifest() -> InstallManifest {
        InstallManifest {
            app_name: "Sample App".to_string(),
            directories: vec!["C:\\Apps\\Sample\\bin".to_string()],
            files: vec![FileSpec {
                source: "payload\\app.exe".to_string(),
                destination: "C:\\Apps\\Sample\\app.exe".to_string(),
            }],
            registry: vec![RegistrySpec {
                key: "HKCU\\Software\\Sample".to_string(),
                name: "InstallLocation".to_string(),
                value: "C:\\Apps\\Sample".to_string(),
            }],
            shortcuts: vec![ShortcutSpec {
                path: "C:\\Users\\Public\\Desktop\\Sample.lnk".to_string(),
                target: "C:\\Apps\\Sample\\app.exe".to_string(),
                arguments: None,
                working_directory: None,
                description: None,
            }],
            scripts: vec![ScriptSpec {
                command: "post-install.cmd".to_string(),
                args: vec!["--ok".to_string()],
                working_directory: None,
            }],
            purge: PurgeSpec {
                registry_branches: vec![],
                paths: vec!["C:\\Apps\\Sample".to_string()],
            },
            support_contact: None,
        }
    }

    fn write_embedded_payload(exe: &Path, payload: &[u8]) {
        let mut bytes = payload.to_vec();
        bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(EMBEDDED_MAGIC);
        fs::write(exe, bytes).unwrap();
    }

    fn quiet_logger() -> Logger {
        Logger {
            json: false,
            quiet: true,
        }
    }
}
