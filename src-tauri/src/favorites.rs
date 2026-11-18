//! Favorite sessions persisted as plain JSON (`favorites.json`) in the config
//! dir. Load errors never propagate: a missing file reads as an empty store,
//! and a malformed one is moved aside (`favorites.json.corrupt-<ts>`) first.

use crate::poller::{CardState, SessionCard, TAIL_BYTES, derive_title};
use crate::transcript::{escape_project_path, parse_transcript_tail, read_tail_lines};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Favorite {
    pub session_uuid: String,
    pub cwd: String,
    pub title: String,
    pub starred_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Favorites {
    pub favorites: Vec<Favorite>,
}

impl Favorites {
    pub fn file_in(dir: &Path) -> PathBuf {
        dir.join("favorites.json")
    }

    /// Missing file → empty store. A file that reads but does not parse is
    /// renamed to `favorites.json.corrupt-<unix-ts>` (so a later save cannot
    /// destroy it) and the store starts empty. If the rename fails the file
    /// is left in place.
    pub fn load(dir: &Path) -> Favorites {
        let path = Self::file_in(dir);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Favorites::default();
        };
        match serde_json::from_str(&text) {
            Ok(favs) => favs,
            Err(_) => {
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let _ = std::fs::rename(&path, dir.join(format!("favorites.json.corrupt-{ts}")));
                Favorites::default()
            }
        }
    }

    /// Atomic: writes a temp file in the same directory, then renames it over
    /// `favorites.json`.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("favorites.json.tmp-{}", std::process::id()));
        let body = serde_json::to_string_pretty(self).expect("favorites serialize");
        let result = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, Self::file_in(dir)));
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// Adds the session if absent, removes it if present. Returns the new
    /// starred state.
    pub fn toggle(&mut self, uuid: &str, cwd: &str, title: &str, now_iso: &str) -> bool {
        if let Some(pos) = self.favorites.iter().position(|f| f.session_uuid == uuid) {
            self.favorites.remove(pos);
            false
        } else {
            self.favorites.push(Favorite {
                session_uuid: uuid.to_string(),
                cwd: cwd.to_string(),
                title: title.to_string(),
                starred_at: now_iso.to_string(),
            });
            true
        }
    }

    pub fn contains(&self, uuid: &str) -> bool {
        self.favorites.iter().any(|f| f.session_uuid == uuid)
    }
}

/// Card for a starred session with no live pane, rebuilt from its transcript
/// (bounded tail read). A missing transcript degrades to the saved title.
pub fn dead_card(fav: &Favorite, projects_root: &Path, now: SystemTime) -> SessionCard {
    let cwd = Path::new(&fav.cwd);
    let path = projects_root
        .join(escape_project_path(cwd))
        .join(format!("{}.jsonl", fav.session_uuid));
    let meta = std::fs::metadata(&path).ok();
    let lines = meta
        .as_ref()
        .and_then(|_| read_tail_lines(&path, TAIL_BYTES).ok());

    let (title, summary, idle_secs, transcript_missing) = match (meta, lines) {
        (Some(meta), Some(lines)) => {
            let info = parse_transcript_tail(&lines);
            let title = if info.summary_title.is_some() || info.first_user_text.is_some() {
                derive_title(cwd, None, &info)
            } else {
                fav.title.clone()
            };
            let idle = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .map_or(0, |d| d.as_secs());
            (title, info.last_assistant_text.unwrap_or_default(), idle, false)
        }
        _ => (fav.title.clone(), String::new(), u64::MAX, true),
    };

    SessionCard {
        pane_id: format!("dead:{}", fav.session_uuid),
        tmux_session: String::new(),
        window_index: 0,
        pane_index: 0,
        title,
        summary,
        state: CardState::Dead,
        idle_secs,
        session_uuid: Some(fav.session_uuid.clone()),
        cwd: fav.cwd.clone(),
        starred: true,
        transcript_missing,
    }
}

/// Live starred cards plus a dead card for every favorite without a live
/// session, ordered by recency (missing transcripts last).
pub fn favorites_view(
    live: &[SessionCard],
    favs: &Favorites,
    projects_root: &Path,
    now: SystemTime,
) -> Vec<SessionCard> {
    let mut out: Vec<SessionCard> = live.iter().filter(|c| c.starred).cloned().collect();
    out.extend(
        favs.favorites
            .iter()
            .filter(|f| {
                !live
                    .iter()
                    .any(|c| c.session_uuid.as_deref() == Some(f.session_uuid.as_str()))
            })
            .map(|f| dead_card(f, projects_root, now)),
    );
    out.sort_by_key(|c| c.idle_secs);
    out
}

/// `TCM_CLAUDE_BIN` env → settings → discovered install → bare `claude`.
///
/// The bare fallback runs on the tmux *server's* PATH, which is typically
/// just homebrew + system dirs — it misses the standard `~/.local/bin`
/// install, and the new window then dies instantly ("command not found",
/// surfacing as a "can't find window" switch error). So before falling
/// back, look for the binary in the common install locations and pass an
/// absolute path.
pub fn resolve_claude_bin(setting: Option<&str>) -> String {
    resolve_claude_bin_with(setting, find_claude_in(&default_claude_dirs()))
}

pub fn resolve_claude_bin_with(setting: Option<&str>, discovered: Option<String>) -> String {
    std::env::var("TCM_CLAUDE_BIN")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| setting.filter(|s| !s.is_empty()).map(str::to_string))
        .or(discovered)
        .unwrap_or_else(|| "claude".to_string())
}

fn default_claude_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
}

/// First `claude` found in `dirs`, as an absolute path. `is_file` follows
/// symlinks, so the usual `~/.local/bin/claude -> versions/<v>` link counts.
pub fn find_claude_in(dirs: &[PathBuf]) -> Option<String> {
    dirs.iter()
        .map(|d| d.join("claude"))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Session uuids are interpolated into a shell command: only `[0-9a-f-]`.
pub fn valid_uuid(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f' | '-'))
}

/// Seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ` (chrono is not a
/// dependency; `starred_at` is informational only).
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[tauri::command]
pub fn toggle_favorite(
    app: tauri::AppHandle,
    session_uuid: Option<String>,
    cwd: String,
    title: String,
) -> Result<bool, String> {
    let uuid = session_uuid.ok_or_else(|| "no transcript yet for this session".to_string())?;
    if !valid_uuid(&uuid) {
        return Err("invalid session uuid".to_string());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let dir = crate::poller::config_dir(&app);
    // Write-through under the lock so concurrent toggles cannot interleave
    // and the tick's clone never sees a state that is not on disk.
    let state = app.state::<crate::FavState>();
    let mut favs = state.0.lock().unwrap();
    toggle_and_persist(&mut favs, &dir, &uuid, &cwd, &title, &iso_utc(now))
}

/// Toggles on a copy and saves it; the in-memory store is replaced only when
/// the save succeeds, so memory never diverges from disk.
pub fn toggle_and_persist(
    favs: &mut Favorites,
    dir: &Path,
    uuid: &str,
    cwd: &str,
    title: &str,
    now_iso: &str,
) -> Result<bool, String> {
    let mut next = favs.clone();
    let starred = next.toggle(uuid, cwd, title, now_iso);
    next.save(dir).map_err(|e| e.to_string())?;
    *favs = next;
    Ok(starred)
}

/// tmux `-c` silently falls back to another dir when the cwd is gone, so
/// check up front to avoid a misleading downstream error.
pub fn check_cwd_exists(cwd: &str) -> Result<(), String> {
    if Path::new(cwd).is_dir() {
        Ok(())
    } else {
        Err(format!("working directory no longer exists: {cwd}"))
    }
}

#[tauri::command]
pub fn resume_session(app: tauri::AppHandle, session_uuid: String) -> Result<(), String> {
    if !valid_uuid(&session_uuid) {
        return Err("invalid session uuid".to_string());
    }
    let live = app
        .state::<crate::LiveIndex>()
        .0
        .lock()
        .unwrap()
        .get(&session_uuid)
        .cloned();
    if let Some((session, w, p)) = live {
        return crate::jump::switch_and_show(&app, &session, w, p);
    }

    let fav = app
        .state::<crate::FavState>()
        .0
        .lock()
        .unwrap()
        .favorites
        .iter()
        .find(|f| f.session_uuid == session_uuid)
        .cloned()
        .ok_or_else(|| "unknown favorite".to_string())?;
    let transcript = crate::poller::projects_root()
        .join(escape_project_path(Path::new(&fav.cwd)))
        .join(format!("{}.jsonl", fav.session_uuid));
    if !transcript.exists() {
        return Err("transcript missing".to_string());
    }

    check_cwd_exists(&fav.cwd)?;

    let settings = crate::settings::Settings::load(&crate::poller::config_dir(&app));
    let cmd = format!(
        "{} --resume {}",
        resolve_claude_bin(settings.claude_bin.as_deref()),
        fav.session_uuid
    );
    let bin = crate::tmux::resolve_tmux_bin().map_err(|e| format!("tmux not found: {e:?}"))?;
    let (session, w, p) = crate::tmux::new_window(&bin, &fav.cwd, &cmd)
        .map_err(|e| format!("tmux new-window failed: {e:?}"))?;
    crate::jump::switch_and_show(&app, &session, w, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_claude_bin_precedence() {
        // The TCM_CLAUDE_BIN branch is not tested: env is process-global.
        // Setting wins over a discovered binary; discovery wins over the
        // bare fallback (which relies on the tmux server PATH and in
        // practice misses ~/.local/bin installs).
        let found = Some("/home/u/.local/bin/claude".to_string());
        assert_eq!(
            resolve_claude_bin_with(Some("/x/claude"), found.clone()),
            "/x/claude"
        );
        assert_eq!(
            resolve_claude_bin_with(None, found),
            "/home/u/.local/bin/claude"
        );
        assert_eq!(resolve_claude_bin_with(None, None), "claude");
    }

    #[test]
    fn find_claude_in_picks_first_dir_containing_it() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty");
        let has = tmp.path().join("has");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&has).unwrap();
        std::fs::write(has.join("claude"), "#!/bin/sh\n").unwrap();

        assert_eq!(
            find_claude_in(&[empty.clone(), has.clone()]),
            Some(has.join("claude").to_string_lossy().into_owned())
        );
        assert_eq!(find_claude_in(&[empty]), None);
    }

    #[test]
    fn toggle_rejects_bad_uuid() {
        assert!(valid_uuid("0123456789abcdef-0123-4567-89ab-cdef01234567"));
        assert!(!valid_uuid(""));
        assert!(!valid_uuid("x; rm -rf"));
        assert!(!valid_uuid("ABC"));
        assert!(!valid_uuid("abc def"));
    }

    #[test]
    fn toggle_and_persist_keeps_memory_on_save_failure() {
        let tmp = tempfile::tempdir().unwrap();
        // A regular file where the config dir should be makes save fail.
        let blocked = tmp.path().join("blocked");
        std::fs::write(&blocked, "x").unwrap();
        let mut favs = Favorites::default();
        let r = toggle_and_persist(&mut favs, &blocked, "abc-1", "/w", "t", "now");
        assert!(r.is_err());
        assert!(favs.favorites.is_empty());

        // Success path swaps the state in and persists it.
        let ok_dir = tmp.path().join("cfg");
        assert_eq!(toggle_and_persist(&mut favs, &ok_dir, "abc-1", "/w", "t", "now"), Ok(true));
        assert!(favs.contains("abc-1"));
        assert!(Favorites::load(&ok_dir).contains("abc-1"));
    }

    #[test]
    fn iso_utc_formats_epoch() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn toggle_adds_then_removes() {
        let mut favs = Favorites::default();
        let uuid = "11111111-2222-3333-4444-555555555555";

        assert!(favs.toggle(uuid, "/work/a", "title a", "2026-11-17T10:00:00+09:00"));
        assert!(favs.contains(uuid));
        let added = &favs.favorites[0];
        assert_eq!(added.session_uuid, uuid);
        assert_eq!(added.cwd, "/work/a");
        assert_eq!(added.title, "title a");
        assert_eq!(added.starred_at, "2026-11-17T10:00:00+09:00");

        assert!(!favs.toggle(uuid, "/work/a", "title a", "2026-11-17T11:00:00+09:00"));
        assert!(!favs.contains(uuid));
        assert!(favs.favorites.is_empty());
    }

    #[test]
    fn roundtrip_save_load() {
        let tmp = tempfile::tempdir().unwrap();
        let mut favs = Favorites::default();
        favs.toggle("uuid-a", "/work/a", "a", "2026-11-17T10:00:00+09:00");
        favs.toggle("uuid-b", "/work/b", "b", "2026-11-17T10:05:00+09:00");

        favs.save(tmp.path()).unwrap();
        assert_eq!(Favorites::load(tmp.path()), favs);
    }

    #[test]
    fn load_malformed_moves_file_aside() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(Favorites::file_in(tmp.path()), "{broken").unwrap();

        assert!(Favorites::load(tmp.path()).favorites.is_empty());
        assert!(!Favorites::file_in(tmp.path()).exists());
        let aside: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("favorites.json.corrupt-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read_to_string(aside[0].path()).unwrap(), "{broken");
    }

    #[test]
    fn load_missing_does_not_rename() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(Favorites::load(tmp.path()).favorites.is_empty());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn saved_file_uses_snake_case_session_uuid() {
        let tmp = tempfile::tempdir().unwrap();
        let favs = Favorites { favorites: vec![fav(UUID_A, "/work/rainbow", "a")] };
        favs.save(tmp.path()).unwrap();
        let raw = std::fs::read_to_string(Favorites::file_in(tmp.path())).unwrap();
        assert!(raw.contains("\"session_uuid\""));
    }

    #[test]
    fn save_leaves_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        Favorites::default().save(tmp.path()).unwrap();
        let names: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["favorites.json".to_string()]);
    }

    #[test]
    fn check_cwd_exists_passes_and_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("proj");
        std::fs::create_dir(&dir).unwrap();
        assert!(check_cwd_exists(dir.to_str().unwrap()).is_ok());
        std::fs::remove_dir(&dir).unwrap();
        let err = check_cwd_exists(dir.to_str().unwrap()).unwrap_err();
        assert!(err.contains("working directory no longer exists"));
        assert!(err.contains(dir.to_str().unwrap()));
    }

    use crate::poller::{CardState, SessionCard};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    const UUID_A: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const UUID_B: &str = "bbbbbbbb-0000-0000-0000-000000000002";
    const UUID_C: &str = "cccccccc-0000-0000-0000-000000000003";

    fn fav(uuid: &str, cwd: &str, title: &str) -> Favorite {
        Favorite {
            session_uuid: uuid.into(),
            cwd: cwd.into(),
            title: title.into(),
            starred_at: "2026-11-17T10:00:00+09:00".into(),
        }
    }

    fn write_transcript(root: &Path, cwd: &str, uuid: &str, body: &str) {
        let dir = root.join(escape_project_path(Path::new(cwd)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{uuid}.jsonl")), body).unwrap();
    }

    const BODY: &str = concat!(
        r#"{"type":"user","message":{"role":"user","content":"tune the rainbow shader"},"sessionId":"s"}"#,
        "\n",
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Shader tuned."}]},"sessionId":"s"}"#,
        "\n"
    );

    fn live_card(uuid: &str, idle: u64) -> SessionCard {
        SessionCard {
            pane_id: "%1".into(),
            tmux_session: "main".into(),
            window_index: 0,
            pane_index: 0,
            title: "live".into(),
            summary: String::new(),
            state: CardState::Idle,
            idle_secs: idle,
            session_uuid: Some(uuid.into()),
            cwd: "/work/rainbow".into(),
            starred: true,
            transcript_missing: false,
        }
    }

    #[test]
    fn dead_card_reads_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        write_transcript(tmp.path(), "/work/rainbow", UUID_A, BODY);
        let now = SystemTime::now() + Duration::from_secs(60);

        let card = dead_card(&fav(UUID_A, "/work/rainbow", "saved"), tmp.path(), now);

        assert_eq!(card.pane_id, format!("dead:{UUID_A}"));
        assert_eq!(card.state, CardState::Dead);
        assert!(card.starred);
        assert!(!card.transcript_missing);
        assert_eq!(card.summary, "Shader tuned.");
        assert_eq!(card.title, "rainbow: tune the rainbow shader");
        assert!(card.idle_secs > 0);
        assert_eq!(card.session_uuid.as_deref(), Some(UUID_A));
    }

    #[test]
    fn dead_card_future_mtime_clamps() {
        let tmp = tempfile::tempdir().unwrap();
        write_transcript(tmp.path(), "/work/rainbow", UUID_A, BODY);

        let card = dead_card(&fav(UUID_A, "/work/rainbow", "saved"), tmp.path(), UNIX_EPOCH);

        assert_eq!(card.idle_secs, 0);
        assert!(!card.transcript_missing);
    }

    #[test]
    fn dead_card_missing_transcript_uses_saved_title() {
        let tmp = tempfile::tempdir().unwrap();
        let card = dead_card(&fav(UUID_A, "/work/rainbow", "saved"), tmp.path(), SystemTime::now());

        assert!(card.transcript_missing);
        assert_eq!(card.idle_secs, u64::MAX);
        assert_eq!(card.title, "saved");
        assert_eq!(card.summary, "");
    }

    #[test]
    fn missing_transcript_sorts_last() {
        let tmp = tempfile::tempdir().unwrap();
        write_transcript(tmp.path(), "/work/rainbow", UUID_B, BODY);
        let favs = Favorites {
            favorites: vec![
                fav(UUID_C, "/work/gone", "gone"),
                fav(UUID_B, "/work/rainbow", "b"),
            ],
        };
        let now = SystemTime::now() + Duration::from_secs(60);

        let view = favorites_view(&[live_card(UUID_A, 5)], &favs, tmp.path(), now);

        let ids: Vec<_> = view.iter().map(|c| c.session_uuid.clone().unwrap()).collect();
        assert_eq!(ids, vec![UUID_A.to_string(), UUID_B.to_string(), UUID_C.to_string()]);
        assert!(view[2].transcript_missing);
    }

    #[test]
    fn live_favorite_not_duplicated() {
        let tmp = tempfile::tempdir().unwrap();
        let favs = Favorites { favorites: vec![fav(UUID_A, "/work/rainbow", "a")] };
        let mut unstarred = live_card(UUID_B, 1);
        unstarred.starred = false;

        let view = favorites_view(&[live_card(UUID_A, 5), unstarred], &favs, tmp.path(), SystemTime::now());

        assert_eq!(view.len(), 1);
        assert_eq!(view[0].pane_id, "%1");
        assert_ne!(view[0].state, CardState::Dead);
    }
}
