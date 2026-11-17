//! Locates and parses Claude Code transcript files
//! (`~/.claude/projects/<escaped-cwd>/<session>.jsonl`). The format is
//! undocumented and version-dependent — every parse is defensive. Files grow
//! to hundreds of MB, so reads are bounded tails only.

use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct TranscriptInfo {
    /// Session title: latest `custom-title`, else latest `ai-title`, else
    /// legacy `summary` record. (Field name kept from the plan; current
    /// Claude Code writes title records, not summary records.)
    pub summary_title: Option<String>,
    pub first_user_text: Option<String>,
    pub last_assistant_text: Option<String>,
    /// Whether the FULL last assistant text (before display truncation)
    /// ends with '?'/'？' — the display string may end with '…' instead.
    pub last_assistant_ends_question: bool,
    /// Latest tool_use seen after the latest assistant text.
    pub current_tool: Option<String>,
}

/// Mirrors Claude Code's project-dir naming: every char outside
/// `[A-Za-z0-9-]` becomes `-` (verified: `_` and `.` also map to `-`).
pub fn escape_project_path(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect()
}

/// Context for resolving a pane WITHOUT a usable `--resume` pin. Several
/// claude processes can share one cwd (= one project folder), so a bare
/// newest-mtime scan can adopt a sibling pane's conversation — and starring
/// would then persist the wrong session uuid.
pub struct ResolveCtx<'a> {
    /// Session uuids pinned by panes' `--resume` args this tick. Files with
    /// these stems belong to those panes and are never adopted by fallback.
    pub claimed: &'a std::collections::HashSet<String>,
    /// When this pane's claude process started (from `ps etime`), if known.
    pub proc_start: Option<std::time::SystemTime>,
}

pub struct TranscriptCandidate {
    pub path: PathBuf,
    pub birth: Option<std::time::SystemTime>,
    pub mtime: Option<std::time::SystemTime>,
}

/// Timestamp jitter allowance between process start and file creation.
const BIRTH_SLACK: std::time::Duration = std::time::Duration::from_secs(60);

/// Pure choice among candidate jsonls for an unpinned pane. A fresh `claude`
/// CREATES its session file right after the process starts, so:
///   1. newest-mtime among files born at/after (start − slack) — the pane's
///      own sessions, including the new file `/clear` creates;
///   2. else newest-mtime among files written at/after start — an in-TUI
///      `/resume` continues an old file but writes to it during this
///      process's life (can transiently pick a busy sibling until the pane's
///      own file exists; the per-tick rescan self-corrects via tier 1);
///   3. else None — better unresolved (retried next tick) than adopting a
///      sibling pane's conversation.
/// Without a known start time, legacy behavior: newest mtime.
pub fn pick_unpinned_transcript(
    cands: &[TranscriptCandidate],
    proc_start: Option<std::time::SystemTime>,
) -> Option<PathBuf> {
    let Some(start) = proc_start else {
        return cands.iter().max_by_key(|c| c.mtime).map(|c| c.path.clone());
    };
    let floor = start.checked_sub(BIRTH_SLACK).unwrap_or(start);
    if let Some(c) = cands
        .iter()
        .filter(|c| c.birth.is_some_and(|b| b >= floor))
        .max_by_key(|c| c.mtime)
    {
        return Some(c.path.clone());
    }
    cands
        .iter()
        .filter(|c| c.mtime.is_some_and(|m| m >= start))
        .max_by_key(|c| c.mtime)
        .map(|c| c.path.clone())
}

fn candidates(
    projects_root: &Path,
    cwd: &Path,
    claimed: &std::collections::HashSet<String>,
) -> Vec<TranscriptCandidate> {
    let proj = projects_root.join(escape_project_path(cwd));
    let Ok(rd) = std::fs::read_dir(&proj) else {
        return vec![];
    };
    rd.filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter(|e| {
            !e.path()
                .file_stem()
                .is_some_and(|s| claimed.contains(s.to_string_lossy().as_ref()))
        })
        .map(|e| {
            let md = e.metadata().ok();
            TranscriptCandidate {
                path: e.path(),
                birth: md.as_ref().and_then(|m| m.created().ok()),
                mtime: md.as_ref().and_then(|m| m.modified().ok()),
            }
        })
        .collect()
}

pub fn locate_transcript(
    projects_root: &Path,
    cwd: &Path,
    resume_id: Option<&str>,
    ctx: &ResolveCtx,
) -> Option<PathBuf> {
    let proj = projects_root.join(escape_project_path(cwd));
    if let Some(id) = resume_id {
        let p = proj.join(format!("{id}.jsonl"));
        if p.exists() {
            return Some(p);
        }
    }
    pick_unpinned_transcript(&candidates(projects_root, cwd, ctx.claimed), ctx.proc_start)
}

/// Resolves the transcript and reports whether it is pinned by a `--resume`
/// id (the `<id>.jsonl` file exists). Pinned entries never move; unpinned
/// ones follow `pick_unpinned_transcript` (`/clear` starts a new session file).
pub fn resolve_transcript(
    projects_root: &Path,
    cwd: &Path,
    resume_id: Option<&str>,
    ctx: &ResolveCtx,
) -> (Option<PathBuf>, bool) {
    let found = locate_transcript(projects_root, cwd, resume_id, ctx);
    let pinned = match (&found, resume_id) {
        (Some(p), Some(id)) => p.file_stem().is_some_and(|s| s == id),
        _ => false,
    };
    (found, pinned)
}

/// Per-tick refresh of an already resolved transcript: pinned entries are
/// returned as-is, unpinned ones re-scan the project folder (one `read_dir`).
/// A scan with no eligible candidate keeps the current file.
pub fn rescan_transcript(
    projects_root: &Path,
    cwd: &Path,
    pinned: bool,
    current: Option<PathBuf>,
    ctx: &ResolveCtx,
) -> Option<PathBuf> {
    if pinned {
        return current;
    }
    locate_transcript(projects_root, cwd, None, ctx).or(current)
}

/// Reads at most `max_bytes` from the end of the file. When the read starts
/// mid-file the first (possibly partial) line is dropped.
pub fn read_tail_lines(path: &Path, max_bytes: u64) -> std::io::Result<Vec<String>> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Ok(lines)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

fn text_of_content(content: &Value) -> Option<String> {
    match content {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Array(blocks) => blocks.iter().find_map(|b| {
            (b["type"] == "text")
                .then(|| b["text"].as_str())
                .flatten()
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().to_string())
        }),
        _ => None,
    }
}

fn tool_label(block: &Value) -> Option<String> {
    let name = block["name"].as_str()?;
    let input = &block["input"];
    let target = input["file_path"]
        .as_str()
        .and_then(|p| Path::new(p).file_name())
        .map(|b| b.to_string_lossy().into_owned())
        .or_else(|| input["command"].as_str().map(|c| truncate_chars(c, 40)))
        .or_else(|| input["description"].as_str().map(|d| truncate_chars(d, 40)));
    Some(match target {
        Some(t) => format!("{name}: {t}"),
        None => name.to_string(),
    })
}

/// Defensive line-by-line parse; unparseable lines are ignored.
pub fn parse_transcript_tail(lines: &[String]) -> TranscriptInfo {
    let mut info = TranscriptInfo::default();
    let (mut custom, mut ai, mut legacy) = (None, None, None);
    for line in lines {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v["type"].as_str().unwrap_or("") {
            "custom-title" => custom = v["customTitle"].as_str().map(str::to_string),
            "ai-title" => ai = v["aiTitle"].as_str().map(str::to_string),
            "summary" => legacy = v["summary"].as_str().map(str::to_string),
            "user" => {
                if info.first_user_text.is_none()
                    && let Some(t) = text_of_content(&v["message"]["content"]) {
                        info.first_user_text = Some(truncate_chars(&t, 120));
                    }
            }
            "assistant" => {
                if let Some(blocks) = v["message"]["content"].as_array() {
                    for b in blocks {
                        match b["type"].as_str().unwrap_or("") {
                            "text" => {
                                if let Some(t) = b["text"].as_str().map(str::trim)
                                    && !t.is_empty()
                                {
                                    info.last_assistant_text = Some(truncate_chars(t, 120));
                                    info.last_assistant_ends_question =
                                        matches!(t.chars().last(), Some('?') | Some('？'));
                                    info.current_tool = None;
                                }
                            }
                            "tool_use" => info.current_tool = tool_label(b),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    info.summary_title = custom.or(ai).or(legacy);
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn escape_matches_real_layout() {
        assert_eq!(
            escape_project_path(Path::new("/Users/x/work/rainbow")),
            "-Users-x-work-rainbow"
        );
        // underscores become dashes too (verified against real ~/.claude/projects)
        assert_eq!(
            escape_project_path(Path::new("/Users/x/work/data_platform.v2")),
            "-Users-x-work-data-platform-v2"
        );
    }

    fn no_claims() -> std::collections::HashSet<String> {
        Default::default()
    }

    fn make_project(dir: &Path, cwd: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let proj = dir.join(escape_project_path(Path::new(cwd)));
        fs::create_dir_all(&proj).unwrap();
        for (name, content) in files {
            let mut f = fs::File::create(proj.join(name)).unwrap();
            writeln!(f, "{content}").unwrap();
        }
        proj
    }

    #[test]
    fn locate_prefers_resume_id() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/Users/x/work/rainbow";
        let proj = make_project(tmp.path(), cwd, &[("aaaa.jsonl", "{}")]);
        // decoy with newer mtime
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = fs::File::create(proj.join("bbbb.jsonl")).unwrap();
        let e = no_claims();
        let ctx = ResolveCtx { claimed: &e, proc_start: None };
        let found = locate_transcript(tmp.path(), Path::new(cwd), Some("aaaa"), &ctx).unwrap();
        assert_eq!(found, proj.join("aaaa.jsonl"));
    }

    #[test]
    fn locate_transcript_mtime_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/Users/x/work/rainbow";
        let proj = make_project(tmp.path(), cwd, &[("old.jsonl", "{}")]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = fs::File::create(proj.join("new.jsonl")).unwrap();
        let e = no_claims();
        let ctx = ResolveCtx { claimed: &e, proc_start: None };
        let found = locate_transcript(tmp.path(), Path::new(cwd), None, &ctx).unwrap();
        assert_eq!(found, proj.join("new.jsonl"));
    }

    fn t(s: u64) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(s)
    }

    fn cand(name: &str, birth: u64, mtime: u64) -> TranscriptCandidate {
        TranscriptCandidate {
            path: PathBuf::from(format!("/p/{name}.jsonl")),
            birth: Some(t(birth)),
            mtime: Some(t(mtime)),
        }
    }

    #[test]
    fn unpinned_prefers_file_born_after_proc_start() {
        // Regression: two claude panes share a cwd. The sibling's transcript
        // (born long before this pane's process, but written constantly) must
        // not win over this pane's own file (born after process start).
        let cands = [cand("sibling", 1000, 9000), cand("own", 5010, 6000)];
        assert_eq!(
            pick_unpinned_transcript(&cands, Some(t(5000))),
            Some(PathBuf::from("/p/own.jsonl"))
        );
    }

    #[test]
    fn unpinned_tier2_mtime_after_start() {
        // In-TUI /resume continues an old file: nothing born after start, but
        // the resumed file was WRITTEN during this process's life.
        let cands = [cand("resumed", 1000, 5500), cand("stale", 900, 4000)];
        assert_eq!(
            pick_unpinned_transcript(&cands, Some(t(5000))),
            Some(PathBuf::from("/p/resumed.jsonl"))
        );
    }

    #[test]
    fn unpinned_none_when_all_predate_start() {
        // Better unresolved (retried next tick) than a sibling's conversation.
        let cands = [cand("a", 1000, 2000), cand("b", 900, 4000)];
        assert_eq!(pick_unpinned_transcript(&cands, Some(t(5000))), None);
    }

    #[test]
    fn unpinned_without_start_newest_mtime() {
        let cands = [cand("a", 1000, 9000), cand("b", 5010, 6000)];
        assert_eq!(
            pick_unpinned_transcript(&cands, None),
            Some(PathBuf::from("/p/a.jsonl"))
        );
    }

    #[test]
    fn locate_skips_claimed_uuids() {
        // A file pinned by another pane's --resume must never be adopted by
        // an unpinned pane's fallback, even when it is the newest.
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/Users/x/work/rainbow";
        let proj = make_project(tmp.path(), cwd, &[("bbbb.jsonl", "{}")]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = fs::File::create(proj.join("aaaa.jsonl")).unwrap(); // newest, claimed
        let claimed: std::collections::HashSet<String> = ["aaaa".to_string()].into();
        let ctx = ResolveCtx { claimed: &claimed, proc_start: None };
        assert_eq!(
            locate_transcript(tmp.path(), Path::new(cwd), None, &ctx),
            Some(proj.join("bbbb.jsonl"))
        );
    }

    #[test]
    fn rescan_keeps_current_when_nothing_eligible() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/Users/x/work/rainbow";
        let proj = make_project(tmp.path(), cwd, &[("old.jsonl", "{}")]);
        let current = Some(proj.join("old.jsonl"));
        // proc "started" far in the future → no candidate is eligible; the
        // rescan must keep the current file rather than dropping to None.
        let empty = std::collections::HashSet::new();
        let far_future = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let ctx = ResolveCtx { claimed: &empty, proc_start: Some(far_future) };
        assert_eq!(
            rescan_transcript(tmp.path(), Path::new(cwd), false, current.clone(), &ctx),
            current
        );
    }

    #[test]
    fn rescan_switches_to_newer_when_not_pinned() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/Users/x/work/rainbow";
        let proj = make_project(tmp.path(), cwd, &[("old.jsonl", "{}")]);
        let e = no_claims();
        let ctx = ResolveCtx { claimed: &e, proc_start: None };
        let (first, pinned) = resolve_transcript(tmp.path(), Path::new(cwd), None, &ctx);
        assert_eq!(first, Some(proj.join("old.jsonl")));
        assert!(!pinned);

        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = fs::File::create(proj.join("new.jsonl")).unwrap();
        let next = rescan_transcript(tmp.path(), Path::new(cwd), pinned, first, &ctx);
        assert_eq!(next, Some(proj.join("new.jsonl")));
    }

    #[test]
    fn rescan_keeps_resume_pinned_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/Users/x/work/rainbow";
        let proj = make_project(tmp.path(), cwd, &[("aaaa.jsonl", "{}")]);
        let e = no_claims();
        let ctx = ResolveCtx { claimed: &e, proc_start: None };
        let (first, pinned) = resolve_transcript(tmp.path(), Path::new(cwd), Some("aaaa"), &ctx);
        assert_eq!(first, Some(proj.join("aaaa.jsonl")));
        assert!(pinned);

        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = fs::File::create(proj.join("bbbb.jsonl")).unwrap();
        let next = rescan_transcript(tmp.path(), Path::new(cwd), pinned, first.clone(), &ctx);
        assert_eq!(next, first);
    }

    #[test]
    fn resume_id_without_file_falls_back_unpinned() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = "/Users/x/work/rainbow";
        let proj = make_project(tmp.path(), cwd, &[("cccc.jsonl", "{}")]);
        let e = no_claims();
        let ctx = ResolveCtx { claimed: &e, proc_start: None };
        let (found, pinned) = resolve_transcript(tmp.path(), Path::new(cwd), Some("zzzz"), &ctx);
        assert_eq!(found, Some(proj.join("cccc.jsonl")));
        assert!(!pinned);
    }

    #[test]
    fn read_tail_lines_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        for i in 0..200_000 {
            writeln!(f, "line-number-{i:07}").unwrap();
        }
        let lines = read_tail_lines(&path, 4096).unwrap();
        assert!(lines.len() < 300, "tail must be bounded, got {}", lines.len());
        assert_eq!(lines.last().unwrap(), "line-number-0199999");
        // first returned line is complete, not a partial from mid-seek
        assert!(lines[0].starts_with("line-number-"));
        assert_eq!(lines[0].len(), "line-number-0000000".len());
    }

    #[test]
    fn parse_tail_extracts_fields() {
        let fixture = include_str!("../tests/fixtures/transcript_sample.jsonl");
        let lines: Vec<String> = fixture.lines().map(str::to_string).collect();
        let info = parse_transcript_tail(&lines);
        // custom-title wins over ai-title
        assert_eq!(info.summary_title.as_deref(), Some("retry-backoff"));
        assert_eq!(
            info.first_user_text.as_deref(),
            Some("please fix the retry backoff in the http client")
        );
        let last = info.last_assistant_text.unwrap();
        assert!(last.starts_with("Done. The client now parses"));
        assert!(last.chars().count() <= 120);
        // The full (pre-truncation) text ends with '?', and that must survive
        // the display truncation. Regression for review finding #2.
        assert!(last.ends_with('…'), "fixture text must exceed 120 chars");
        assert!(info.last_assistant_ends_question);
        let tool = info.current_tool.unwrap();
        assert!(tool.starts_with("Bash"), "got: {tool}");
    }
}
