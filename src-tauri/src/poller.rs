//! Assembles SessionCards from tmux/procs/transcript/state_detect every tick,
//! emits them to the webview, and decides notifications.

use crate::settings::Settings;
use crate::state_detect::{self, PaneState};
use crate::transcript::{self, TranscriptInfo};
use crate::{procs, tmux};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const TAIL_BYTES: u64 = 262_144;

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardState {
    Working,
    NeedsInputStructured,
    NeedsInputProse,
    Idle,
    Stale,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionCard {
    pub pane_id: String,
    pub tmux_session: String,
    pub window_index: u32,
    pub pane_index: u32,
    pub title: String,
    pub summary: String,
    pub state: CardState,
    pub idle_secs: u64,
}

/// True when the pane's transcript mapping must be (re)resolved: the claude
/// process changed, or the previous attempt found no transcript (the .jsonl
/// may not exist yet right after session start — retry until found).
fn should_reresolve(cached_pid: i32, new_pid: i32, has_transcript: bool) -> bool {
    cached_pid != new_pid || !has_transcript
}

/// settings.json is user-edited; clamp so a typo can't busy-loop the thread.
fn effective_poll_interval(ms: u64) -> u64 {
    ms.max(250)
}

pub fn derive_card_state(
    pane: PaneState,
    info: &TranscriptInfo,
    idle_secs: u64,
    stale_after_min: u64,
) -> CardState {
    match pane {
        PaneState::Working => CardState::Working,
        PaneState::WaitingStructured => CardState::NeedsInputStructured,
        PaneState::Idle => {
            if info.last_assistant_ends_question {
                CardState::NeedsInputProse
            } else if idle_secs >= stale_after_min * 60 {
                CardState::Stale
            } else {
                CardState::Idle
            }
        }
    }
}

pub fn derive_title(cwd: &Path, pane_title: Option<&str>, info: &TranscriptInfo) -> String {
    let base = cwd
        .file_name()
        .map(|b| b.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string_lossy().into_owned());
    let detail = pane_title
        .map(str::to_string)
        .or_else(|| info.summary_title.clone())
        .or_else(|| info.first_user_text.clone());
    match detail {
        Some(d) => format!("{base}: {d}"),
        None => base,
    }
}

pub fn derive_summary(
    state: CardState,
    pending_question: Option<&str>,
    info: &TranscriptInfo,
) -> String {
    match state {
        CardState::NeedsInputStructured => {
            format!("⚠ Waiting: {}", pending_question.unwrap_or("input needed"))
        }
        CardState::Working => info
            .current_tool
            .clone()
            .or_else(|| info.last_assistant_text.clone())
            .unwrap_or_default(),
        _ => info.last_assistant_text.clone().unwrap_or_default(),
    }
}

/// Notification policy: only structured needs-input ever notifies; one per
/// transition; on the first tick (prev=None) already-waiting sessions are
/// coalesced into a single message.
pub fn notifications_due(
    prev: Option<&HashMap<String, CardState>>,
    curr: &[SessionCard],
) -> Vec<String> {
    let waiting: Vec<&SessionCard> = curr
        .iter()
        .filter(|c| c.state == CardState::NeedsInputStructured)
        .collect();
    match prev {
        None => match waiting.len() {
            0 => vec![],
            1 => vec![format!("{} — waiting for input", waiting[0].title)],
            n => vec![format!("{n} sessions waiting for input")],
        },
        Some(prev) => waiting
            .iter()
            .filter(|c| prev.get(&c.pane_id) != Some(&CardState::NeedsInputStructured))
            .map(|c| format!("{} — waiting for input", c.title))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct PaneCache {
    claude_pid: i32,
    cwd: PathBuf,
    transcript: Option<PathBuf>,
}

fn projects_root() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .join("projects")
}

fn idle_secs_of(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Most recently active first (ascending idle). Stable: panes with equal
/// idle keep tmux pane order, so the list doesn't shuffle between ticks.
fn sort_most_recent_first(cards: &mut [SessionCard]) {
    cards.sort_by_key(|c| c.idle_secs);
}

fn build_cards(
    tmux_bin: &Path,
    ps: &str,
    cache: &mut HashMap<String, PaneCache>,
    stale_after_min: u64,
) -> Vec<SessionCard> {
    let panes = match tmux::list_panes(tmux_bin) {
        Ok(p) => p,
        Err(_) => return vec![], // no server / tmux gone → empty panel
    };
    let mut cards = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for pane in panes {
        let Some(claude) = procs::find_claude_in_pane(pane.pane_pid, ps) else {
            continue;
        };
        seen.insert(pane.pane_id.clone());
        let entry = cache.entry(pane.pane_id.clone()).or_default();
        if should_reresolve(entry.claude_pid, claude.pid, entry.transcript.is_some()) {
            // new claude process in this pane: re-resolve cwd + transcript
            entry.claude_pid = claude.pid;
            entry.cwd = procs::cwd_of_pid(claude.pid)
                .unwrap_or_else(|| PathBuf::from(&pane.current_path));
            entry.transcript = transcript::locate_transcript(
                &projects_root(),
                &entry.cwd,
                claude.resume_session_id.as_deref(),
            );
        }
        let pane_text = tmux::capture_pane(tmux_bin, &pane.pane_id).unwrap_or_default();
        let info = entry
            .transcript
            .as_deref()
            .and_then(|p| transcript::read_tail_lines(p, TAIL_BYTES).ok())
            .map(|lines| transcript::parse_transcript_tail(&lines))
            .unwrap_or_default();
        let idle_secs = entry.transcript.as_deref().map(idle_secs_of).unwrap_or(0);
        let pane_state = state_detect::classify_pane(&pane_text);
        let state = derive_card_state(pane_state, &info, idle_secs, stale_after_min);
        let pane_title = state_detect::extract_pane_title(&pane_text);
        let pending = state_detect::extract_pending_question(&pane_text);
        cards.push(SessionCard {
            pane_id: pane.pane_id.clone(),
            tmux_session: pane.session.clone(),
            window_index: pane.window_index,
            pane_index: pane.pane_index,
            title: derive_title(&entry.cwd, pane_title.as_deref(), &info),
            summary: derive_summary(state, pending.as_deref(), &info),
            state,
            idle_secs,
        });
    }
    cache.retain(|k, _| seen.contains(k));
    sort_most_recent_first(&mut cards);
    cards
}

/// Exposed for examples/scan.rs (manual verification); same code path as the loop.
pub fn build_cards_for_example(
    tmux_bin: &Path,
    ps: &str,
    cache: &mut HashMap<String, PaneCache>,
    stale_after_min: u64,
) -> Vec<SessionCard> {
    build_cards(tmux_bin, ps, cache, stale_after_min)
}

pub fn spawn(app: tauri::AppHandle) {
    use tauri::Emitter;
    use tauri_plugin_notification::NotificationExt;
    std::thread::spawn(move || {
        let config_dir = config_dir(&app);
        let tmux_bin = match tmux::resolve_tmux_bin() {
            Ok(b) => b,
            Err(_) => {
                let _ = app.emit("sessions-updated", Vec::<SessionCard>::new());
                return;
            }
        };
        let mut cache: HashMap<String, PaneCache> = HashMap::new();
        let mut prev: Option<HashMap<String, CardState>> = None;
        loop {
            let settings = Settings::load(&config_dir);
            let ps = procs::snapshot_ps();
            let cards = build_cards(&tmux_bin, &ps, &mut cache, settings.stale_after_min);
            if settings.notifications {
                for msg in notifications_due(prev.as_ref(), &cards) {
                    let _ = app
                        .notification()
                        .builder()
                        .title("Claude Sessions")
                        .body(&msg)
                        .show();
                }
            }
            prev = Some(
                cards
                    .iter()
                    .map(|c| (c.pane_id.clone(), c.state))
                    .collect(),
            );
            let _ = app.emit("sessions-updated", &cards);
            // Tray title: "● N" while N sessions wait for input (v2).
            if let Some(handle) = {
                use tauri::Manager;
                app.try_state::<crate::tray::TrayHandle>()
            } {
                let title = crate::tray::tray_title(crate::tray::waiting_count(&cards));
                let _ = handle
                    .0
                    .lock()
                    .unwrap()
                    .set_title(if title.is_empty() { None } else { Some(title) });
            }
            std::thread::sleep(std::time::Duration::from_millis(effective_poll_interval(
                settings.poll_interval_ms,
            )));
        }
    });
}

pub fn config_dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_detect::PaneState;
    use crate::transcript::TranscriptInfo;
    use std::collections::HashMap;
    use std::path::Path;

    fn info(last_assistant: Option<&str>) -> TranscriptInfo {
        TranscriptInfo {
            last_assistant_text: last_assistant.map(str::to_string),
            last_assistant_ends_question: last_assistant
                .is_some_and(|t| matches!(t.trim_end().chars().last(), Some('?') | Some('？'))),
            ..Default::default()
        }
    }

    fn card(pane_id: &str, state: CardState) -> SessionCard {
        SessionCard {
            pane_id: pane_id.to_string(),
            tmux_session: "s".into(),
            window_index: 0,
            pane_index: 0,
            title: "t".into(),
            summary: String::new(),
            state,
            idle_secs: 0,
        }
    }

    #[test]
    fn cards_sorted_most_recent_first_stable_on_ties() {
        let mut cards = vec![
            SessionCard { idle_secs: 300, ..card("%1", CardState::Idle) },
            SessionCard { idle_secs: 5, ..card("%2", CardState::Working) },
            SessionCard { idle_secs: 300, ..card("%3", CardState::Idle) },
            SessionCard { idle_secs: 0, ..card("%4", CardState::Working) },
        ];
        sort_most_recent_first(&mut cards);
        let ids: Vec<&str> = cards.iter().map(|c| c.pane_id.as_str()).collect();
        // ascending idle; ties (%1, %3) keep their pane order
        assert_eq!(ids, vec!["%4", "%2", "%1", "%3"]);
    }

    #[test]
    fn state_derivation_all_variants() {
        let stale_min = 30;
        assert_eq!(
            derive_card_state(PaneState::Working, &info(None), 0, stale_min),
            CardState::Working
        );
        assert_eq!(
            derive_card_state(PaneState::WaitingStructured, &info(None), 0, stale_min),
            CardState::NeedsInputStructured
        );
        assert_eq!(
            derive_card_state(PaneState::Idle, &info(Some("Want me to run tests?")), 10, stale_min),
            CardState::NeedsInputProse
        );
        assert_eq!(
            derive_card_state(PaneState::Idle, &info(Some("実行しますか？")), 10, stale_min),
            CardState::NeedsInputProse
        );
        assert_eq!(
            derive_card_state(PaneState::Idle, &info(Some("done.")), 31 * 60, stale_min),
            CardState::Stale
        );
        assert_eq!(
            derive_card_state(PaneState::Idle, &info(Some("done.")), 60, stale_min),
            CardState::Idle
        );
    }

    #[test]
    fn title_precedence() {
        let cwd = Path::new("/Users/x/work/rainbow");
        let mut i = TranscriptInfo::default();
        i.summary_title = Some("From title record".into());
        i.first_user_text = Some("first user message".into());
        assert_eq!(
            derive_title(cwd, Some("pane-ruler-title"), &i),
            "rainbow: pane-ruler-title"
        );
        assert_eq!(derive_title(cwd, None, &i), "rainbow: From title record");
        i.summary_title = None;
        assert_eq!(derive_title(cwd, None, &i), "rainbow: first user message");
        i.first_user_text = None;
        assert_eq!(derive_title(cwd, None, &i), "rainbow");
    }

    #[test]
    fn structured_summary_formatting() {
        let s = derive_summary(
            CardState::NeedsInputStructured,
            Some("Do you want to proceed?"),
            &info(Some("irrelevant")),
        );
        assert_eq!(s, "⚠ Waiting: Do you want to proceed?");
        let mut i = info(Some("last text"));
        i.current_tool = Some("Bash: cargo build".into());
        assert_eq!(derive_summary(CardState::Working, None, &i), "Bash: cargo build");
        assert_eq!(derive_summary(CardState::Idle, None, &info(Some("last text"))), "last text");
    }

    #[test]
    fn transcript_reresolved_until_found() {
        // A pane whose transcript didn't exist yet must be retried, not
        // cached as a permanent miss. Regression for review finding #3.
        assert!(should_reresolve(10, 10, false));
        assert!(!should_reresolve(10, 10, true));
        assert!(should_reresolve(10, 11, true));
    }

    #[test]
    fn poll_interval_clamped() {
        // settings.json is user-edited; 0 must not busy-loop the poll thread.
        assert_eq!(effective_poll_interval(0), 250);
        assert_eq!(effective_poll_interval(2000), 2000);
    }

    #[test]
    fn startup_coalesces_waiting() {
        let cards = vec![
            card("%1", CardState::NeedsInputStructured),
            card("%2", CardState::NeedsInputStructured),
        ];
        let msgs = notifications_due(None, &cards);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("2"), "got: {}", msgs[0]);
    }

    #[test]
    fn transition_notifies_once() {
        let mut prev = HashMap::new();
        prev.insert("%1".to_string(), CardState::Working);
        prev.insert("%2".to_string(), CardState::NeedsInputStructured);
        let cards = vec![
            card("%1", CardState::NeedsInputStructured), // transition → notify
            card("%2", CardState::NeedsInputStructured), // unchanged → silent
        ];
        assert_eq!(notifications_due(Some(&prev), &cards).len(), 1);
    }

    #[test]
    fn prose_never_notifies() {
        let mut prev = HashMap::new();
        prev.insert("%1".to_string(), CardState::Idle);
        let cards = vec![card("%1", CardState::NeedsInputProse)];
        assert!(notifications_due(Some(&prev), &cards).is_empty());
        // and on startup too
        assert!(notifications_due(None, &cards).is_empty());
    }
}
