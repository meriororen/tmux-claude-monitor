//! App settings as plain JSON. The poller re-reads this each tick so edits
//! via the ⚙ affordance apply without restart.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub poll_interval_ms: u64,
    pub stale_after_min: u64,
    pub notifications: bool,
    pub window_pos: Option<(i32, i32)>,
    /// Read at launch; changing it requires an app restart.
    pub global_shortcut: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            poll_interval_ms: 2000,
            stale_after_min: 30,
            notifications: true,
            window_pos: None,
            global_shortcut: "cmd+shift+k".to_string(),
        }
    }
}

impl Settings {
    pub fn file_in(dir: &Path) -> std::path::PathBuf {
        dir.join("settings.json")
    }

    /// Missing or unparseable file → defaults.
    pub fn load(dir: &Path) -> Settings {
        std::fs::read_to_string(Self::file_in(dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            Self::file_in(dir),
            serde_json::to_string_pretty(self).expect("settings serialize"),
        )
    }
}

#[tauri::command]
pub fn save_window_pos(app: tauri::AppHandle, x: i32, y: i32) -> Result<(), String> {
    let dir = crate::poller::config_dir(&app);
    let mut s = Settings::load(&dir);
    s.window_pos = Some((x, y));
    s.save(&dir).map_err(|e| e.to_string())
}

/// Creates settings.json with defaults only when missing. Never rewrites an
/// existing file: a load→save cycle would replace a malformed mid-edit file
/// with defaults and silently wipe the user's edits.
pub fn ensure_settings_file(dir: &Path) -> std::io::Result<()> {
    if Settings::file_in(dir).exists() {
        return Ok(());
    }
    Settings::default().save(dir)
}

/// ⚙ affordance: open settings.json in the default editor. The poller
/// re-reads settings each tick, so edits apply without restart.
#[tauri::command]
pub fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    let dir = crate::poller::config_dir(&app);
    ensure_settings_file(&dir).map_err(|e| e.to_string())?;
    std::process::Command::new("/usr/bin/open")
        .arg(Settings::file_in(&dir))
        .status()
        .map_err(|e| e.to_string())
        .and_then(|st| if st.success() { Ok(()) } else { Err("open failed".into()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_settings_file_never_overwrites_existing() {
        // A malformed (mid-edit) file must NOT be replaced with defaults.
        // Regression for review finding #4.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(Settings::file_in(tmp.path()), "{broken json").unwrap();
        ensure_settings_file(tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(Settings::file_in(tmp.path())).unwrap(),
            "{broken json"
        );
        // and creates defaults when missing
        let tmp2 = tempfile::tempdir().unwrap();
        ensure_settings_file(tmp2.path()).unwrap();
        assert_eq!(Settings::load(tmp2.path()), Settings::default());
    }

    #[test]
    fn settings_roundtrip_and_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        // missing file → defaults
        let s = Settings::load(tmp.path());
        assert_eq!(s.poll_interval_ms, 2000);
        assert_eq!(s.stale_after_min, 30);
        assert!(s.notifications);
        assert_eq!(s.window_pos, None);
        assert_eq!(s.global_shortcut, "cmd+shift+k");
        // save → load roundtrip
        let changed = Settings {
            poll_interval_ms: 5000,
            stale_after_min: 10,
            notifications: false,
            window_pos: Some((12, 34)),
            global_shortcut: "alt+space".to_string(),
        };
        changed.save(tmp.path()).unwrap();
        assert_eq!(Settings::load(tmp.path()), changed);
    }
}
