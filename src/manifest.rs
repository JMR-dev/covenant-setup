use crate::AppError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub(crate) struct InstallManifest {
    pub(crate) app_name: String,
    #[serde(default, deserialize_with = "deserialize_directory_paths")]
    pub(crate) directories: Vec<String>,
    #[serde(default)]
    pub(crate) files: Vec<FileSpec>,
    #[serde(default)]
    pub(crate) registry: Vec<RegistrySpec>,
    #[serde(default)]
    pub(crate) shortcuts: Vec<ShortcutSpec>,
    #[serde(default)]
    pub(crate) scripts: Vec<ScriptSpec>,
    #[serde(default)]
    pub(crate) purge: PurgeSpec,
    #[serde(default)]
    pub(crate) support_contact: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PurgeSpec {
    #[serde(default)]
    pub(crate) registry_branches: Vec<String>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
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
pub(crate) struct FileSpec {
    pub(crate) source: String,
    pub(crate) destination: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistrySpec {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ShortcutSpec {
    pub(crate) path: String,
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) arguments: Option<String>,
    #[serde(default)]
    pub(crate) working_directory: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ScriptSpec {
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) working_directory: Option<String>,
}

pub(crate) fn read_install_manifest(manifest_path: &Path) -> Result<InstallManifest, AppError> {
    let manifest: InstallManifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;
    enforce_manifest_file_name(manifest_path, &manifest)?;
    enforce_manifest_field_spacing(&manifest)?;
    Ok(manifest)
}

/// Packaging-time rule only: install must accept manifests from any
/// directory because bundled installs extract under %TEMP%, which contains
/// spaces for user profiles like `C:\Users\First Last`.
pub(crate) fn enforce_manifest_path_spacing(manifest_path: &Path) -> Result<(), AppError> {
    let manifest_path_text = manifest_path.to_string_lossy();
    if contains_whitespace(&manifest_path_text) {
        return Err(AppError::Message(format!(
            "Manifest path cannot contain spaces: {}",
            manifest_path.display()
        )));
    }

    Ok(())
}

pub(crate) fn enforce_manifest_file_name(
    manifest_path: &Path,
    manifest: &InstallManifest,
) -> Result<(), AppError> {
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

pub(crate) fn enforce_manifest_field_spacing(manifest: &InstallManifest) -> Result<(), AppError> {
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

pub(crate) fn expected_manifest_file_name(app_name: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

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

        // Only the file name is constrained at install time; the directory may
        // contain spaces (bundled installs extract under %TEMP%).
        enforce_manifest_file_name(
            Path::new("manifest folder\\SampleApp-install.toml"),
            &manifest,
        )
        .unwrap();
    }

    #[test]
    fn manifest_path_spacing_is_enforced_only_for_packaging() {
        enforce_manifest_path_spacing(Path::new("C:\\work\\SampleApp-install.toml")).unwrap();

        let err =
            enforce_manifest_path_spacing(Path::new("manifest folder\\SampleApp-install.toml"))
                .unwrap_err()
                .to_string();
        assert!(err.contains("cannot contain spaces"));
    }

    #[test]
    fn read_install_manifest_accepts_spaces_in_parent_directory() {
        let temp = TestDir::new("manifest-spaced-parent");
        let spaced_dir = temp.path().join("User Name");
        fs::create_dir_all(&spaced_dir).unwrap();
        let manifest_path = spaced_dir.join("SpacedDir-install.toml");
        fs::write(&manifest_path, "app_name = 'Spaced Dir'\n").unwrap();

        read_install_manifest(&manifest_path).unwrap();
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
}
