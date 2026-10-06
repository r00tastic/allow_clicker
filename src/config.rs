use std::{fs, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepAction {
    Wait,
    Stop,
}

impl Default for StepAction {
    fn default() -> Self { Self::Wait }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub word: String,
    #[serde(default = "default_delay")]
    pub delay_secs: f32,
    #[serde(default)]
    pub action: StepAction,
}

fn default_delay() -> f32 { 1.5 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_steps")]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub region_x: i32,
    #[serde(default)]
    pub region_y: i32,
    #[serde(default)]
    pub region_w: i32,
    #[serde(default)]
    pub region_h: i32,
    #[serde(default = "default_interval")]
    pub interval_ms: u32,
}

fn default_steps() -> Vec<Step> {
    vec![Step { word: "Allow".to_string(), delay_secs: 1.5, action: StepAction::Wait }]
}
fn default_interval() -> u32 { 500 }

impl Default for Config {
    fn default() -> Self {
        Self {
            steps: default_steps(),
            case_sensitive: false,
            dry_run: false,
            region_x: 0,
            region_y: 0,
            region_w: 0,
            region_h: 0,
            interval_ms: default_interval(),
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Some(dirs) = directories::ProjectDirs::from("dev", "Local", "AllowClicker") {
        let dir = dirs.config_dir();
        let _ = fs::create_dir_all(dir);
        dir.join("config.json")
    } else {
        PathBuf::from("allow_clicker_config.json")
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Self::default(),
        };

        if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
            if !cfg.steps.is_empty() {
                return cfg;
            }
        }

        // Eski format: {"targets": ["Allow", "Join"], "cooldown_ms": 1500}
        #[derive(Deserialize)]
        struct Legacy {
            #[serde(default)]
            targets: Vec<String>,
            #[serde(default)]
            target: Option<String>,
            #[serde(default = "legacy_cooldown")]
            cooldown_ms: u32,
            #[serde(default)]
            case_sensitive: bool,
            #[serde(default)]
            dry_run: bool,
            #[serde(default)]
            region_x: i32,
            #[serde(default)]
            region_y: i32,
            #[serde(default)]
            region_w: i32,
            #[serde(default)]
            region_h: i32,
            #[serde(default = "default_interval")]
            interval_ms: u32,
        }
        fn legacy_cooldown() -> u32 { 1500 }

        if let Ok(old) = serde_json::from_str::<Legacy>(&text) {
            let mut words = old.targets;
            if words.is_empty() {
                if let Some(t) = old.target {
                    words.push(t);
                }
            }
            if words.is_empty() {
                return Self::default();
            }
            let delay = old.cooldown_ms as f32 / 1000.0;
            let steps = words.into_iter().map(|w| Step { word: w, delay_secs: delay, action: StepAction::Wait }).collect();
            return Config {
                steps,
                case_sensitive: old.case_sensitive,
                dry_run: old.dry_run,
                region_x: old.region_x,
                region_y: old.region_y,
                region_w: old.region_w,
                region_h: old.region_h,
                interval_ms: old.interval_ms,
            };
        }

        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }
}
