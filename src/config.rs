use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persisted user settings, stored as JSON in the platform config dir
/// (e.g. ~/.config/con621/config.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Target frames-per-second for video/animation playback (1..=60).
    pub fps: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { fps: 15 }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("con621").join("config.json"))
    }

    /// Load config from disk, falling back to defaults on any error.
    pub fn load() -> Self {
        let Some(path) = Self::path() else { return Self::default() };
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist config to disk. Errors are returned for the caller to surface.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("Cannot find config directory")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())
    }

    pub fn set_fps(&mut self, fps: u32) {
        self.fps = fps.clamp(1, 60);
    }
}
