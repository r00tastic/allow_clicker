use std::{fs, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_targets", alias = "target")]
    #[serde(deserialize_with = "de_targets")]
    pub targets: Vec<String>,
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
    #[serde(default = "default_cooldown")]
    pub cooldown_ms: u32,
}

fn default_targets() -> Vec<String> {
    vec!["Allow".to_string()]
}
fn default_interval() -> u32 { 500 }
fn default_cooldown() -> u32 { 1500 }

/// String veya Vec<String> kabul eder (eski config'ler icin backwards-compat)
fn de_targets<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("string veya string listesi")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<String>, E> {
            Ok(vec![v.to_string()])
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<Vec<String>, E> {
            Ok(vec![v])
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut out = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                out.push(s);
            }
            Ok(out)
        }
    }
    d.deserialize_any(V)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            targets: default_targets(),
            case_sensitive: false,
            dry_run: false,
            region_x: 0,
            region_y: 0,
            region_w: 0,
            region_h: 0,
            interval_ms: default_interval(),
            cooldown_ms: default_cooldown(),
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
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<Config>(&text) {
                return cfg;
            }
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
