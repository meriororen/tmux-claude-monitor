//! Read-only tmux wrappers (list/capture) plus the one sanctioned mutation
//! (switch-client on user click). All binaries are absolute paths: a GUI
//! .app does not inherit the login shell PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct TmuxPane {
    pub session: String,
    pub window_index: u32,
    pub pane_index: u32,
    pub pane_id: String,
    pub pane_pid: i32,
    pub current_command: String,
    pub current_path: String,
}

#[derive(Debug)]
pub enum TmuxError {
    NoServer,
    NotFound,
    Io(String),
}

const PANE_FORMAT: &str = "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}";

pub fn resolve_tmux_bin() -> Result<PathBuf, TmuxError> {
    if let Ok(p) = std::env::var("TCM_TMUX_BIN") {
        return Ok(PathBuf::from(p));
    }
    for candidate in [
        "/opt/homebrew/bin/tmux",
        "/usr/local/bin/tmux",
        "/usr/bin/tmux",
    ] {
        if Path::new(candidate).exists() {
            return Ok(PathBuf::from(candidate));
        }
    }
    Err(TmuxError::NotFound)
}

pub fn parse_panes(output: &str) -> Vec<TmuxPane> {
    output
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 7 {
                return None;
            }
            Some(TmuxPane {
                session: f[0].to_string(),
                window_index: f[1].parse().ok()?,
                pane_index: f[2].parse().ok()?,
                pane_id: f[3].to_string(),
                pane_pid: f[4].parse().ok()?,
                current_command: f[5].to_string(),
                current_path: f[6].to_string(),
            })
        })
        .collect()
}

pub(crate) fn classify_tmux_failure(stderr: &str) -> TmuxError {
    if stderr.contains("no server running") || stderr.contains("error connecting to") {
        TmuxError::NoServer
    } else {
        TmuxError::Io(stderr.trim().to_string())
    }
}

fn run_tmux(bin: &Path, args: &[&str]) -> Result<String, TmuxError> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| TmuxError::Io(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(classify_tmux_failure(&String::from_utf8_lossy(&out.stderr)))
    }
}

pub fn list_panes(bin: &Path) -> Result<Vec<TmuxPane>, TmuxError> {
    run_tmux(bin, &["list-panes", "-a", "-F", PANE_FORMAT]).map(|out| parse_panes(&out))
}

pub fn capture_pane(bin: &Path, pane_id: &str) -> Result<String, TmuxError> {
    run_tmux(bin, &["capture-pane", "-p", "-t", pane_id])
}

pub fn switch_to(
    bin: &Path,
    session: &str,
    window_index: u32,
    pane_index: u32,
) -> Result<(), TmuxError> {
    // '=' forces exact session-name matching (see jump::switch_target).
    let client = format!("={session}");
    let window = format!("={session}:{window_index}");
    let pane = format!("={session}:{window_index}.{pane_index}");
    run_tmux(bin, &["switch-client", "-t", &client])?;
    run_tmux(bin, &["select-window", "-t", &window])?;
    run_tmux(bin, &["select-pane", "-t", &pane])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_panes_basic() {
        let out = "neovim\t2\t0\t%6\t1993\t2.1.314\t/Users/x/work/rainbow\n";
        let panes = parse_panes(out);
        let p = &panes[0];
        assert_eq!(
            (
                p.session.as_str(),
                p.window_index,
                p.pane_index,
                p.pane_id.as_str(),
                p.pane_pid
            ),
            ("neovim", 2, 0, "%6", 1993)
        );
        assert_eq!(p.current_command, "2.1.314");
        assert_eq!(p.current_path, "/Users/x/work/rainbow");
    }

    #[test]
    fn parse_panes_with_pipe_in_name() {
        let out = "my|weird session\t0\t1\t%3\t42\tzsh\t/tmp\n";
        assert_eq!(parse_panes(out)[0].session, "my|weird session");
    }

    #[test]
    fn parse_panes_skips_malformed_lines() {
        assert!(parse_panes("garbage-no-tabs\n").is_empty());
    }

    #[test]
    fn list_panes_no_server() {
        assert!(matches!(
            classify_tmux_failure("no server running on /tmp/x"),
            TmuxError::NoServer
        ));
    }

    #[test]
    fn resolve_tmux_env_override() {
        unsafe { std::env::set_var("TCM_TMUX_BIN", "/usr/bin/true") };
        assert_eq!(resolve_tmux_bin().unwrap(), PathBuf::from("/usr/bin/true"));
        unsafe { std::env::remove_var("TCM_TMUX_BIN") };
    }
}
