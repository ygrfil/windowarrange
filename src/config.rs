use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{CandidateDisposition, WindowSignature};

const CONFIG_VERSION: u32 = 6;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationDefault {
    #[default]
    Ignored,
    FreeSpace,
    TopRight,
}

impl ApplicationDefault {
    #[must_use]
    pub const fn disposition(self) -> CandidateDisposition {
        match self {
            Self::Ignored => CandidateDisposition::Ignored,
            Self::FreeSpace => CandidateDisposition::FreeSpace,
            Self::TopRight => CandidateDisposition::TopRight,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HotkeySettings {
    pub arrange_now: String,
    pub toggle_focused: String,
    pub show_panel: String,
    pub toggle_slots: Vec<String>,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            arrange_now: "Ctrl+Shift+A".to_owned(),
            toggle_focused: "Ctrl+Shift+T".to_owned(),
            show_panel: "Ctrl+Shift+P".to_owned(),
            toggle_slots: (1..=8).map(|slot| format!("Ctrl+Shift+F{slot}")).collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetectionRule {
    pub signature: WindowSignature,
    pub disposition: CandidateDisposition,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub selected_monitor: Option<String>,
    pub auto_arrange: bool,
    pub reserve_two_slots: bool,
    pub default_application_mode: ApplicationDefault,
    pub table_aspect_ratio: Option<f64>,
    pub detection_rules: Vec<DetectionRule>,
    pub table_order: Vec<WindowSignature>,
    pub hotkeys: HotkeySettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            selected_monitor: None,
            auto_arrange: true,
            reserve_two_slots: true,
            default_application_mode: ApplicationDefault::Ignored,
            table_aspect_ratio: None,
            detection_rules: Vec::new(),
            table_order: Vec::new(),
            hotkeys: HotkeySettings::default(),
        }
    }
}

impl AppConfig {
    #[must_use]
    pub fn disposition_for(&self, signature: &WindowSignature) -> Option<CandidateDisposition> {
        self.detection_rules
            .iter()
            .rev()
            .find(|rule| &rule.signature == signature)
            .map(|rule| rule.disposition)
            .or_else(|| {
                self.detection_rules
                    .iter()
                    .rev()
                    .find(|rule| {
                        rule.signature.title_pattern.is_empty()
                            && rule.signature.process_name == signature.process_name
                            && rule.signature.class_name == signature.class_name
                    })
                    .map(|rule| rule.disposition)
            })
    }

    pub fn set_disposition(
        &mut self,
        signature: WindowSignature,
        disposition: CandidateDisposition,
    ) {
        self.detection_rules
            .retain(|rule| rule.signature != signature);
        self.detection_rules.push(DetectionRule {
            signature,
            disposition,
        });
    }

    pub fn set_application_disposition(
        &mut self,
        signature: WindowSignature,
        disposition: CandidateDisposition,
    ) {
        self.set_disposition(signature.clone(), disposition);
        self.set_disposition(
            WindowSignature {
                title_pattern: String::new(),
                ..signature
            },
            disposition,
        );
    }
}

#[derive(Clone, Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    #[must_use]
    pub fn for_current_user() -> Self {
        let path = env::var_os("APPDATA")
            .filter(|root| !root.is_empty())
            .map(PathBuf::from)
            .map(|root| {
                root.join("ClubGGTools")
                    .join("ClubGG Table Arranger")
                    .join("config")
                    .join("config.json")
            })
            .unwrap_or_else(|| PathBuf::from("clubgg-table-arranger.config.json"));
        Self { path }
    }

    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn log_path(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("logs")
            .join("table-arranger.log")
    }

    #[must_use]
    pub fn ui_state_path(&self) -> PathBuf {
        self.path.with_file_name("ui-state-v4.ron")
    }

    pub fn load(&self) -> Result<AppConfig, ConfigError> {
        if !self.path.exists() {
            return Ok(AppConfig::default());
        }
        let contents = fs::read_to_string(&self.path)?;
        let mut config: AppConfig = serde_json::from_str(&contents)?;
        if config.version < 4 {
            let application_rules: Vec<_> = config
                .detection_rules
                .iter()
                .filter(|rule| {
                    matches!(
                        rule.disposition,
                        CandidateDisposition::TopRight | CandidateDisposition::FreeSpace
                    )
                })
                .cloned()
                .collect();
            for rule in application_rules {
                config.set_disposition(
                    WindowSignature {
                        title_pattern: String::new(),
                        ..rule.signature
                    },
                    rule.disposition,
                );
            }
        }
        config.version = CONFIG_VERSION;
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = serde_json::to_string_pretty(config)?;
        fs::write(&self.path, serialized)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuration data is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_rule_last_write_wins() {
        let signature = WindowSignature {
            process_name: "clubgg.exe".to_owned(),
            class_name: "table".to_owned(),
            title_pattern: "table #".to_owned(),
        };
        let mut config = AppConfig::default();
        config.set_disposition(signature.clone(), CandidateDisposition::Ignored);
        config.set_disposition(signature.clone(), CandidateDisposition::Table);
        config.set_disposition(signature.clone(), CandidateDisposition::FreeSpace);
        config.set_disposition(signature.clone(), CandidateDisposition::Parked);
        assert_eq!(
            config.disposition_for(&signature),
            Some(CandidateDisposition::Parked)
        );
        assert_eq!(config.detection_rules.len(), 1);
    }

    #[test]
    fn application_rules_use_exact_choice_then_stable_process_fallback() {
        let first = WindowSignature {
            process_name: "browser.exe".to_owned(),
            class_name: "BrowserWindow".to_owned(),
            title_pattern: "first page".to_owned(),
        };
        let second = WindowSignature {
            title_pattern: "different page".to_owned(),
            ..first.clone()
        };
        let mut config = AppConfig::default();
        config.set_application_disposition(first.clone(), CandidateDisposition::FreeSpace);
        config.set_application_disposition(second.clone(), CandidateDisposition::TopRight);

        assert_eq!(
            config.disposition_for(&first),
            Some(CandidateDisposition::FreeSpace)
        );
        assert_eq!(
            config.disposition_for(&second),
            Some(CandidateDisposition::TopRight)
        );
        assert_eq!(
            config.disposition_for(&WindowSignature {
                title_pattern: "new session title".to_owned(),
                ..first
            }),
            Some(CandidateDisposition::TopRight)
        );
    }

    #[test]
    fn older_configurations_default_to_reserving_two_slots() {
        let config: AppConfig = serde_json::from_str(r#"{"version":4}"#).unwrap();

        assert!(config.reserve_two_slots);
        assert_eq!(config.default_application_mode, ApplicationDefault::Ignored);
    }
}
