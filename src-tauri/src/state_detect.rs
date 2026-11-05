//! Heuristics over `tmux capture-pane` text. All patterns live here, in one
//! place, because they encode Claude Code's TUI strings — captured against
//! Claude Code 2.1.31x (fixtures in tests/fixtures/pane_*.txt). When a Claude
//! Code update changes the UI, re-capture fixtures and adjust here only.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneState {
    Working,
    WaitingStructured,
    Idle,
}

/// Marker strings observed in Claude Code 2.1.31x fixtures:
/// - working: "esc to interrupt" (status line while a turn runs)
/// - structured waits: "Do you want" (permission dialog), "❯ 1." (numbered
///   option cursor), "Enter to confirm" (trust/選択 dialogs)
///   Verified: the permission dialog does NOT show "esc to interrupt", so
///   checking Working first is safe (pane_permission.txt).
const WORKING_MARKERS: &[&str] = &["esc to interrupt"];
/// NOTE: "Do you want" is deliberately NOT a marker — assistant prose turns
/// routinely end with "Do you want me to …?" and stay on screen after the
/// turn finishes; matching it would turn prose questions into structured
/// waits (false notifications). The permission dialog is caught by its
/// "❯ 1." option cursor instead.
const STRUCTURED_MARKERS: &[&str] = &["❯ 1.", "Enter to confirm"];

pub fn classify_pane(text: &str) -> PaneState {
    if WORKING_MARKERS.iter().any(|m| text.contains(m)) {
        return PaneState::Working;
    }
    if STRUCTURED_MARKERS.iter().any(|m| text.contains(m)) {
        return PaneState::WaitingStructured;
    }
    PaneState::Idle
}

/// The session title Claude Code renders in the ruler above the input box:
/// `──────…────── <title> ─`. A plain ruler (only `─`) has no title.
pub fn extract_pane_title(text: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let line = line.trim_end();
        let rest = line.strip_suffix(" ─")?;
        let (ruler, title) = rest.rsplit_once(' ')?;
        if ruler.len() >= 10 && ruler.chars().all(|c| c == '─') && !title.is_empty() {
            Some(title.to_string())
        } else {
            None
        }
    })
}

/// For a structured wait: the nearest non-empty line above the option list
/// (e.g. "Do you want to proceed?"), used as the card's "⚠ Waiting:" text.
pub fn extract_pending_question(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let anchor = lines
        .iter()
        .rposition(|l| l.contains("❯ 1.") || l.trim_start().starts_with("❯ No") || l.contains("Enter to confirm"))?;
    lines[..anchor]
        .iter()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.chars().all(|c| c == '─' || c == '╌'))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKING: &str = include_str!("../tests/fixtures/pane_working.txt");
    const PERMISSION: &str = include_str!("../tests/fixtures/pane_permission.txt");
    const QUESTION_BOX: &str = include_str!("../tests/fixtures/pane_question_box.txt");
    const IDLE: &str = include_str!("../tests/fixtures/pane_idle.txt");
    const IDLE_TITLE: &str = include_str!("../tests/fixtures/pane_idle_title.txt");

    #[test]
    fn classify_working() {
        assert_eq!(classify_pane(WORKING), PaneState::Working);
    }

    #[test]
    fn classify_permission_prompt() {
        assert_eq!(classify_pane(PERMISSION), PaneState::WaitingStructured);
    }

    #[test]
    fn classify_question_box() {
        assert_eq!(classify_pane(QUESTION_BOX), PaneState::WaitingStructured);
    }

    const IDLE_PROSE: &str = include_str!("../tests/fixtures/pane_idle_prose.txt");

    #[test]
    fn prose_question_on_screen_is_not_structured() {
        // A finished turn ending "Do you want …?" stays visible in the pane.
        // That is a PROSE question (amber, never notify) — it must not match
        // the structured markers. Regression for review finding #1.
        assert_eq!(classify_pane(IDLE_PROSE), PaneState::Idle);
    }

    #[test]
    fn classify_idle() {
        assert_eq!(classify_pane(IDLE), PaneState::Idle);
        assert_eq!(classify_pane(IDLE_TITLE), PaneState::Idle);
    }

    #[test]
    fn pane_title_extracted() {
        assert_eq!(
            extract_pane_title(IDLE_TITLE).as_deref(),
            Some("slide-5-logtonewrelic-link")
        );
        assert_eq!(extract_pane_title(IDLE).as_deref(), Some("idwr"));
        // titleless case: the working pane's ruler is plain
        assert_eq!(extract_pane_title(WORKING), None);
    }

    #[test]
    fn pending_question_extracted() {
        let q = extract_pending_question(PERMISSION).unwrap();
        assert!(!q.is_empty());
        assert!(q.contains("Do you want"), "got: {q}");
    }
}
