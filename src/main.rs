#![windows_subsystem = "windows"]
mod ui;
mod win;

use clap::{ArgAction, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use thiserror::Error;
use ui::GuiProgress;

const EXIT_ELEVATION_REQUIRED: i32 = 33;
const EXIT_OPERATION_FAILED: i32 = 1;
const BUNDLE_MANIFEST: &str = "install.toml";
const EMBEDDED_MAGIC: &[u8] = b"COVENANT_SETUP_BUNDLE_V1";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
    command: Commands,
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
    #[serde(default)]
    directories: Vec<DirectorySpec>,
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PurgeSpec {
    #[serde(default)]
    registry_branches: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DirectorySpec {
    path: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    app_name: String,
    manifest_path: Option<PathBuf>,
    actions: Vec<JournalAction>,
    purge: PurgeSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
struct PackagedApp {
    app_name: String,
    manifest: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmbeddedFile {
    relative_path: String,
    data: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
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
    fn finish(self, app_name: String, manifest_path: Option<PathBuf>, purge: PurgeSpec) -> Journal;
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

    fn finish(self, app_name: String, manifest_path: Option<PathBuf>, purge: PurgeSpec) -> Journal {
        Journal {
            app_name,
            manifest_path,
            actions: self.actions,
            purge,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RegistryRoot {
    Hkcu,
    Hklm,
}

#[derive(Debug, Error)]
enum AppError {
    #[error("{0}")]
    Message(String),
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

enum RuntimeMode {
    Bundled,
}

#[derive(Clone, Copy)]
struct UiPreferences {
    headless: bool,
    headed: bool,
    automation: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UiMode {
    None,
    Gui,
    Tui,
}

#[derive(Clone, Copy)]
enum UiPhase {
    Install,
    Uninstall,
    Cleanup,
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
    if is_bundled_runtime_invocation(&args) {
        let logger = Logger {
            json: false,
            quiet: false,
        };
        if let Some(mode) = detect_runtime_mode() {
            let preferences = parse_ui_preferences(&args);
            let exit_code = match run_bundled_installer(mode, preferences, &logger) {
                Ok(()) => 0,
                Err(AppError::Message(ref message)) if message == "__elevated_relaunch__" => 0,
                Err(err) => {
                    let _ = ui::report_error(&err.to_string());
                    logger.error(err, EXIT_OPERATION_FAILED);
                    EXIT_OPERATION_FAILED
                }
            };
            process::exit(exit_code);
        }
    }

    let cli = Cli::parse();
    let logger = Logger {
        json: cli.json,
        quiet: false,
    };
    let exit_code = match run(cli, &logger) {
        Ok(()) => 0,
        Err(AppError::Message(message)) if message == "__elevated_relaunch__" => 0,
        Err(err) => {
            let code = if matches!(&err, AppError::Message(message) if message.contains("Elevation required"))
            {
                EXIT_ELEVATION_REQUIRED
            } else {
                EXIT_OPERATION_FAILED
            };
            logger.error(err, code);
            code
        }
    };
    process::exit(exit_code);
}

fn run(cli: Cli, logger: &Logger) -> Result<(), AppError> {
    let preferences = ui_preferences_from_cli(&cli);
    match cli.command {
        Commands::Package { manifest, output } => package(&manifest, &output, logger),
        Commands::Install { manifest, journal } => install(
            &manifest,
            journal,
            cli.elevate,
            select_ui(UiPhase::Install, preferences, logger)?,
            logger,
        ),
        Commands::Uninstall { journal } => uninstall(
            &journal,
            cli.elevate,
            select_ui(UiPhase::Uninstall, preferences, logger)?,
            preferences.automation,
            logger,
        ),
        Commands::Cleanup {
            target_exe,
            install_root,
            app_name,
        } => cleanup(
            target_exe,
            install_root,
            app_name,
            select_ui(UiPhase::Cleanup, preferences, logger)?,
            preferences.automation,
            logger,
        ),
    }
}

fn package(manifest_path: &Path, output_root: &Path, logger: &Logger) -> Result<(), AppError> {
    let manifest: InstallManifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;
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

fn build_packaged_installer(
    exe_target: &Path,
    current_exe: &Path,
    manifest_dir: &Path,
    manifest_path: &Path,
    manifest: &InstallManifest,
    logger: &Logger,
) -> Result<(), AppError> {
    fs::copy(current_exe, &exe_target)?;
    let bundle = EmbeddedBundle {
        metadata: PackagedApp {
            app_name: manifest.app_name.clone(),
            manifest: BUNDLE_MANIFEST.to_string(),
        },
        files: collect_bundle_files(manifest_dir, manifest_path)?,
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
) -> Result<Vec<EmbeddedFile>, AppError> {
    let mut files = Vec::new();
    collect_bundle_files_recursive(source_root, source_root, manifest_path, &mut files)?;
    Ok(files)
}

fn collect_bundle_files_recursive(
    source_root: &Path,
    current: &Path,
    manifest_path: &Path,
    files: &mut Vec<EmbeddedFile>,
) -> Result<(), AppError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_bundle_files_recursive(source_root, &path, manifest_path, files)?;
        } else {
            let relative = path
                .strip_prefix(source_root)
                .map_err(|_| AppError::Message("Failed to derive embedded file path".into()))?;
            let relative_path = if path == manifest_path {
                BUNDLE_MANIFEST.to_string()
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

fn detect_runtime_mode() -> Option<RuntimeMode> {
    let exe = std::env::current_exe().ok()?;
    if read_embedded_bundle(&exe).ok().flatten().is_none() {
        return None;
    }
    Some(RuntimeMode::Bundled)
}

fn run_bundled_installer(
    mode: RuntimeMode,
    preferences: UiPreferences,
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
    match mode {
        RuntimeMode::Bundled => {
            let ui_mode = select_ui(UiPhase::Install, preferences, logger)?;
            trace_event(
                "bundled_installer_ui_selected",
                json!({"ui_mode": ui_mode_name(ui_mode), "automation": preferences.automation}),
            );
            if ui_mode == UiMode::Gui
                && !preferences.automation
                && !ui::confirm_install(&metadata.app_name)?
            {
                return Ok(());
            }
            match install(&manifest_path, None, true, ui_mode, logger) {
                Ok(()) => {
                    trace_event("bundled_installer_install_ok", json!({}));
                    if ui_mode == UiMode::Gui && !preferences.automation {
                        ui::report_success(&metadata.app_name)?;
                    }
                    Ok(())
                }
                Err(err) => {
                    trace_event(
                        "bundled_installer_install_error",
                        json!({"error": err.to_string()}),
                    );
                    if ui_mode == UiMode::Gui && !preferences.automation {
                        ui::report_error(&err.to_string())?;
                    }
                    Err(err)
                }
            }
        }
    }
}

fn install(
    manifest_path: &Path,
    journal_path: Option<PathBuf>,
    elevate: bool,
    ui_mode: UiMode,
    logger: &Logger,
) -> Result<(), AppError> {
    let manifest: InstallManifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;
    trace_event(
        "install_start",
        json!({"manifest": manifest_path, "app_name": &manifest.app_name}),
    );
    let app_name = manifest.app_name.clone();
    let _progress = start_tui_progress(ui_mode, format!("Installing {} ", manifest.app_name));
    let mut gui_progress = start_gui_progress(
        ui_mode,
        &format!("Installing {}", manifest.app_name),
        &manifest.app_name,
        total_install_steps(&manifest),
    )?;
    let result = (|| -> Result<(), AppError> {
        let effective_logger = if ui_mode == UiMode::Tui {
            logger.quiet_clone()
        } else {
            logger.clone()
        };
        let resolver = win::PathResolver::new(&effective_logger)?;
        let requires_admin = manifest_requires_admin(&manifest, &resolver)?;
        ensure_elevation_if_needed(requires_admin, elevate, &effective_logger)?;
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
        let mut tracker = DeclaredTracker::new();

        let mut progress_step = 0usize;
        for directory in &manifest.directories {
            let path = resolver.resolve(&directory.path);
            effective_logger.info("create_directory", json!({"path":path}));
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
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
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
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
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
                &format!("Writing registry value {} in {}", entry.name, entry.key),
            )?;
            win::set_registry_string(
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
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
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
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
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
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
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
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
                &format!("Registering {} in Installed Apps", manifest.app_name),
            )?;
            register_uninstall_entry(
                &manifest,
                &runtime,
                install_root,
                uninstall_exe_path,
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

        let journal = tracker.finish(
            manifest.app_name.clone(),
            Some(manifest_path.to_path_buf()),
            manifest.purge,
        );
        if let Some(parent) = runtime.journal_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&runtime.journal_path, serde_json::to_vec_pretty(&journal)?)?;
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
        )?;
        if ui_mode == UiMode::Tui {
            println!("{} installation completed successfully", manifest.app_name);
        }
        Ok(())
    })();

    if let Err(err) = &result {
        trace_event(
            "install_error",
            json!({"app_name": app_name, "error": err.to_string()}),
        );
        let _ = fail_gui_progress(
            &mut gui_progress,
            &format!("{app_name} installation failed: {err}"),
        );
    }

    result
}

fn uninstall(
    journal_path: &Path,
    elevate: bool,
    ui_mode: UiMode,
    automation: bool,
    logger: &Logger,
) -> Result<(), AppError> {
    let journal: Journal = serde_json::from_str(&fs::read_to_string(journal_path)?)?;
    trace_event(
        "uninstall_start",
        json!({"journal": journal_path, "app_name": &journal.app_name}),
    );
    let app_name = journal.app_name.clone();
    let _progress = start_tui_progress(ui_mode, format!("Uninstalling {} ", journal.app_name));
    let mut gui_progress = start_gui_progress(
        ui_mode,
        &format!("Uninstalling {}", journal.app_name),
        &journal.app_name,
        total_uninstall_steps(&journal),
    )?;
    let result = (|| -> Result<(), AppError> {
        let effective_logger = if ui_mode == UiMode::Tui {
            logger.quiet_clone()
        } else {
            logger.clone()
        };
        let resolver = win::PathResolver::new(&effective_logger)?;
        let requires_admin = journal_requires_admin(&journal, &resolver)?;
        ensure_elevation_if_needed(requires_admin, elevate, &effective_logger)?;
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
                    progress_step += 1;
                    advance_gui_progress(
                        &mut gui_progress,
                        progress_step,
                        &format!("Removing directory {}", path.display()),
                    )?;
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
                        progress_step += 1;
                        advance_gui_progress(
                            &mut gui_progress,
                            progress_step,
                            &format!("Removing file {}", destination.display()),
                        )?;
                        win::remove_file_with_fallback(destination, &effective_logger)?
                    }
                }
                JournalAction::WriteRegistry { root, subkey, .. } => {
                    if is_uninstall_registry_key(subkey) {
                        deferred_uninstall_registry.push((*root, subkey.clone()));
                    } else {
                        progress_step += 1;
                        advance_gui_progress(
                            &mut gui_progress,
                            progress_step,
                            &format!("Removing registry branch {}", subkey),
                        )?;
                        win::delete_registry_tree(*root, subkey, &effective_logger)?
                    }
                }
                JournalAction::CreateShortcut { path } => {
                    progress_step += 1;
                    advance_gui_progress(
                        &mut gui_progress,
                        progress_step,
                        &format!("Removing shortcut {}", path.display()),
                    )?;
                    win::remove_file_with_fallback(path, &effective_logger)?
                }
                JournalAction::ExecuteScript { .. } => {
                    effective_logger.info("skip_script_rollback", json!({}))
                }
            }
        }

        for branch in &journal.purge.registry_branches {
            let (root, subkey) = parse_registry_key(branch)?;
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
                &format!("Purging registry branch {}", branch),
            )?;
            win::delete_registry_tree(root, &subkey, &effective_logger)?;
        }
        for path in &journal.purge.paths {
            progress_step += 1;
            let resolved = resolver.resolve(path);
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
                &format!("Purging path {}", resolved.display()),
            )?;
            purge_path(&resolved, &effective_logger)?;
        }

        for (root, subkey) in deferred_uninstall_registry {
            progress_step += 1;
            advance_gui_progress(
                &mut gui_progress,
                progress_step,
                &format!("Removing uninstall registration {}", subkey),
            )?;
            win::delete_registry_tree(root, &subkey, &effective_logger)?;
        }

        if let Some(path) = deferred_self_delete {
            finish_gui_progress(
                &mut gui_progress,
                &format!("Finalizing removal of {}", journal.app_name),
            )?;
            spawn_cleanup_helper(
                &path,
                path.parent(),
                &journal.app_name,
                ui_mode,
                automation,
                &effective_logger,
            )?;
        } else {
            finish_gui_progress(
                &mut gui_progress,
                &format!("{} uninstalled successfully!", journal.app_name),
            )?;
            if ui_mode == UiMode::Gui && !automation {
                ui::report_uninstall_success(&journal.app_name)?;
            }
        }

        effective_logger.result("ok", json!({"journal":journal_path}));
        trace_event("uninstall_ok", json!({"journal": journal_path}));
        Ok(())
    })();

    if let Err(err) = &result {
        trace_event(
            "uninstall_error",
            json!({"app_name": app_name, "error": err.to_string()}),
        );
        let _ = fail_gui_progress(
            &mut gui_progress,
            &format!("{app_name} uninstall failed: {err}"),
        );
    }

    result
}

fn cleanup(
    target_exe: PathBuf,
    install_root: Option<PathBuf>,
    app_name: String,
    ui_mode: UiMode,
    automation: bool,
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
        win::remove_file_with_fallback(&target_exe, &effective_logger)?;
        reboot_required = target_exe.exists();
    }
    if let Some(install_root) = install_root {
        if install_root.exists() && fs::read_dir(&install_root)?.next().is_none() {
            win::remove_directory_if_exists(&install_root, &effective_logger)?;
        }
    }
    reboot_required |= schedule_helper_self_cleanup(&effective_logger)?;
    if ui_mode == UiMode::Gui && !automation {
        if reboot_required {
            if ui::prompt_uninstall_reboot(&app_name)? {
                spawn_reboot(&effective_logger)?;
            }
        } else {
            ui::report_uninstall_success(&app_name)?;
        }
    } else if ui_mode == UiMode::Tui {
        if reboot_required {
            println!(
                "{app_name} uninstalled sucessfully! Some files from the program still remain on your computer. To complete removal of these files, restart your computer now."
            );
            if prompt_reboot_tui()? {
                spawn_reboot(&effective_logger)?;
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
    logger: &Logger,
) -> Result<(), AppError> {
    if !required || win::is_elevated(logger)? {
        trace_event(
            "elevation_ok",
            json!({"required": required, "relaunch": relaunch}),
        );
        return Ok(());
    }
    if relaunch {
        trace_event("elevation_relaunch", json!({}));
        win::relaunch_as_admin(logger)?;
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
        return Some(resolver.resolve(&directory.path));
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
        win::set_registry_string(
            runtime.uninstall_registry_root,
            &runtime.uninstall_registry_key,
            name,
            &value,
            logger,
        )?;
    }
    Ok(())
}

fn spawn_cleanup_helper(
    target_exe: &Path,
    install_root: Option<&Path>,
    app_name: &str,
    ui_mode: UiMode,
    automation: bool,
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

fn parse_ui_preferences(args: &[OsString]) -> UiPreferences {
    let mut preferences = UiPreferences {
        headless: false,
        headed: false,
        automation: false,
    };
    for arg in args.iter().skip(1) {
        let value = arg.to_string_lossy();
        if value == "--headless" {
            preferences.headless = true;
        } else if value == "--headed" {
            preferences.headed = true;
        } else if value == "--automation" {
            preferences.automation = true;
        }
    }
    preferences
}

fn ui_preferences_from_cli(cli: &Cli) -> UiPreferences {
    UiPreferences {
        headless: cli.headless,
        headed: cli.headed,
        automation: cli.automation,
    }
}

fn is_bundled_runtime_invocation(args: &[OsString]) -> bool {
    let has_subcommand = args
        .iter()
        .skip(1)
        .map(|arg| arg.to_string_lossy().to_ascii_lowercase())
        .any(|arg| {
            matches!(
                arg.as_str(),
                "package" | "install" | "uninstall" | "cleanup"
            )
        });
    !has_subcommand
}

fn select_ui(
    phase: UiPhase,
    preferences: UiPreferences,
    logger: &Logger,
) -> Result<UiMode, AppError> {
    if preferences.headless {
        return Ok(UiMode::Tui);
    }
    if preferences.headed {
        return Ok(UiMode::Gui);
    }
    if io::stdout().is_terminal() && win::is_parent_powershell(logger)? {
        return Ok(UiMode::Tui);
    }
    if !io::stdout().is_terminal() {
        return Ok(UiMode::Gui);
    }
    Ok(match phase {
        UiPhase::Install => UiMode::None,
        UiPhase::Uninstall | UiPhase::Cleanup => UiMode::None,
    })
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
    title: &str,
    app_name: &str,
    total_steps: usize,
) -> Result<Option<GuiProgress>, AppError> {
    trace_event(
        "gui_progress_start",
        json!({
            "ui_mode": ui_mode_name(ui_mode),
            "title": title,
            "app_name": app_name,
            "total_steps": total_steps.max(1)
        }),
    );
    if ui_mode == UiMode::Gui {
        Ok(Some(ui::GuiProgress::start(
            title,
            &format!("{title}"),
            total_steps.max(1),
        )?))
    } else {
        let _ = app_name;
        Ok(None)
    }
}

fn advance_gui_progress(
    gui_progress: &mut Option<GuiProgress>,
    current_step: usize,
    message: &str,
) -> Result<(), AppError> {
    trace_event(
        "progress",
        json!({"current_step": current_step, "message": message}),
    );
    if let Some(progress) = gui_progress.as_mut() {
        progress.advance(current_step, message)?;
    }
    Ok(())
}

fn finish_gui_progress(
    gui_progress: &mut Option<GuiProgress>,
    message: &str,
) -> Result<(), AppError> {
    trace_event("progress_finish", json!({"message": message}));
    if let Some(progress) = gui_progress.as_mut() {
        progress.finish(message)?;
    }
    Ok(())
}

fn fail_gui_progress(
    gui_progress: &mut Option<GuiProgress>,
    message: &str,
) -> Result<(), AppError> {
    trace_event("progress_fail", json!({"message": message}));
    if let Some(progress) = gui_progress.as_mut() {
        progress.finish(message)?;
    }
    Ok(())
}

fn append_gui_shell_output(
    gui_progress: &mut Option<GuiProgress>,
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

fn schedule_helper_self_cleanup(logger: &Logger) -> Result<bool, AppError> {
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

fn spawn_reboot(logger: &Logger) -> Result<(), AppError> {
    logger.info("spawn_reboot", json!({}));
    let mut command = Command::new("shutdown.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    command.args(["/r", "/t", "0"]);
    command.spawn()?;
    Ok(())
}

fn prompt_reboot_tui() -> Result<bool, AppError> {
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
        if path_requires_admin(&resolver.resolve(&directory.path)) {
            return Ok(true);
        }
    }
    for file in &manifest.files {
        if path_requires_admin(&resolver.resolve(&file.destination)) {
            return Ok(true);
        }
    }
    for shortcut in &manifest.shortcuts {
        if path_requires_admin(&resolver.resolve(&shortcut.path)) {
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
                if path_requires_admin(path) {
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
        if path_requires_admin(&resolver.resolve(path)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn execute_script(
    script: &ScriptSpec,
    manifest_dir: Option<&Path>,
    working_directory: Option<&Path>,
    gui_progress: &mut Option<GuiProgress>,
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

fn purge_path(path: &Path, logger: &Logger) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_file() {
        return win::remove_file_with_fallback(path, logger);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            purge_path(&child, logger)?;
        } else {
            win::remove_file_with_fallback(&child, logger)?;
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

fn same_path(left: &Path, right: &Path) -> bool {
    normalize_path_for_compare(left) == normalize_path_for_compare(right)
}

fn normalize_path_for_compare(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn path_requires_admin(path: &Path) -> bool {
    let path = path.to_string_lossy().to_ascii_lowercase();
    path.starts_with("c:\\program files") || path.starts_with("c:\\windows")
}

fn absolutize(base: Option<&Path>, value: &str) -> PathBuf {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        candidate
    } else {
        base.unwrap_or_else(|| Path::new(".")).join(candidate)
    }
}
