use super::*;

pub struct PathResolver {
    pub program_files_x64: PathBuf,
    pub program_files_x86: PathBuf,
    pub local_app_data: PathBuf,
    pub desktop: PathBuf,
    pub(crate) admin_roots: Vec<String>,
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
            program_files_x86,
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
                .replace(
                    "{ProgramFilesX86}",
                    &self.program_files_x86.to_string_lossy(),
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
            program_files_x86: PathBuf::new(),
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
    let path_result = pwstr_to_path(raw, logger);
    logger.unsafe_enter("CoTaskMemFree", json!({}));
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    logger.unsafe_exit("CoTaskMemFree", json!({"ok":true}));
    path_result
}

pub(crate) fn pwstr_to_path(raw: PWSTR, logger: &Logger) -> Result<PathBuf, AppError> {
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
