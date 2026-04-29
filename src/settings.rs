use std::collections::HashSet;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

fn default_concurrency() -> usize { 2 }

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Settings {
    #[serde(default)]
    pub favorites: HashSet<String>,
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    pub window_w: Option<f32>,
    pub window_h: Option<f32>,
    #[serde(default)]
    pub lang: crate::i18n::Lang,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

impl Settings {
    pub fn app_data_dir() -> PathBuf {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("ihaul")
    }

    fn path() -> PathBuf {
        Self::app_data_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }
}
