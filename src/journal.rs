use crate::AppError;
use crate::manifest::PurgeSpec;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Journal {
    pub(crate) app_name: String,
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) actions: Vec<JournalAction>,
    pub(crate) purge: PurgeSpec,
    #[serde(default)]
    pub(crate) support_contact: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum JournalAction {
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

pub(crate) trait MutationTracker {
    fn record(&mut self, action: JournalAction);
    fn finish(
        self,
        app_name: String,
        manifest_path: Option<PathBuf>,
        purge: PurgeSpec,
        support_contact: Option<String>,
    ) -> Journal;
}

pub(crate) struct DeclaredTracker {
    actions: Vec<JournalAction>,
}

impl DeclaredTracker {
    pub(crate) fn new() -> Self {
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
pub(crate) enum RegistryRoot {
    Hkcu,
    Hklm,
}

pub(crate) fn write_journal(path: &Path, journal: &Journal) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(journal)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

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
}
