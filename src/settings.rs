use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const FAVORITES_FORMAT_VERSION: u8 = 1;

fn default_concurrency() -> usize {
    2
}

fn sanitize_concurrency(concurrency: usize) -> usize {
    if (1..=8).contains(&concurrency) {
        concurrency
    } else {
        default_concurrency()
    }
}

#[derive(Serialize, Clone)]
pub struct Settings {
    #[serde(default)]
    pub favorites_by_device: HashMap<String, Vec<String>>,
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    pub window_w: Option<f32>,
    pub window_h: Option<f32>,
    #[serde(default)]
    pub lang: crate::i18n::Lang,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub open_top_favorite_on_startup: bool,
    #[serde(rename = "favorites_format_version")]
    favorites_format_version: u8,
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawSettings {
            #[serde(default)]
            favorites_by_device: HashMap<String, Vec<String>>,
            window_x: Option<f32>,
            window_y: Option<f32>,
            window_w: Option<f32>,
            window_h: Option<f32>,
            #[serde(default)]
            lang: crate::i18n::Lang,
            #[serde(default = "default_concurrency")]
            concurrency: usize,
            #[serde(default)]
            open_top_favorite_on_startup: bool,
            #[serde(default)]
            favorites_format_version: Option<u8>,
        }

        let raw = RawSettings::deserialize(deserializer)?;
        let mut favorites_by_device = raw.favorites_by_device;
        if raw.favorites_format_version != Some(FAVORITES_FORMAT_VERSION) {
            // The legacy HashSet format also serialized as arrays. A missing or
            // unknown marker therefore means legacy order, which is migrated
            // deterministically without discarding the rest of the settings.
            for favorites in favorites_by_device.values_mut() {
                favorites.sort();
            }
        }

        Ok(Self {
            favorites_by_device,
            window_x: raw.window_x,
            window_y: raw.window_y,
            window_w: raw.window_w,
            window_h: raw.window_h,
            lang: raw.lang,
            concurrency: raw.concurrency,
            open_top_favorite_on_startup: raw.open_top_favorite_on_startup,
            favorites_format_version: FAVORITES_FORMAT_VERSION,
        })
    }
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
            open_top_favorite_on_startup: false,
            favorites_format_version: FAVORITES_FORMAT_VERSION,
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
