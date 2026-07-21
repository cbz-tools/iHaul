use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

fn default_concurrency() -> usize { 2 }

fn sanitize_concurrency(concurrency: usize) -> usize {
    if (1..=8).contains(&concurrency) { concurrency } else { default_concurrency() }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default)]
    pub favorites_by_device: HashMap<String, HashSet<String>>,
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    pub window_w: Option<f32>,
    pub window_h: Option<f32>,
    #[serde(default)]
    pub lang: crate::i18n::Lang,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            favorites_by_device: HashMap::new(),
            window_x: None,
            window_y: None,
            window_w: None,
            window_h: None,
            lang: crate::i18n::Lang::default(),
            concurrency: default_concurrency(),
        }
    }
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
        let mut settings: Self = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        settings.concurrency = sanitize_concurrency(settings.concurrency);
        settings
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_concurrency_is_two() {
        assert_eq!(Settings::default().concurrency, 2);
    }

    #[test]
    fn invalid_concurrency_uses_the_default() {
        assert_eq!(sanitize_concurrency(0), 2);
        assert_eq!(sanitize_concurrency(9), 2);
        assert_eq!(sanitize_concurrency(4), 4);
    }
}
