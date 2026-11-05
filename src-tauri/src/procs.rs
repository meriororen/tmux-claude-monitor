//! Finds the Claude Code process inside a tmux pane's process tree.
//! `pane_current_command` is useless for this (Claude Code sets its process
//! title to a bare version string like "2.1.314"), so we walk descendants
//! of the pane PID in a single `ps` snapshot per poll tick.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ClaudeProc {
    pub pid: i32,
    pub cmdline: String,
    pub resume_session_id: Option<String>,
}

/// One `ps` snapshot per poll tick; the caller holds the output.
pub fn snapshot_ps() -> String {
    Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn parse_ps(ps_output: &str) -> Vec<(i32, i32, &str)> {
    ps_output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid_s, rest) = line.split_once(char::is_whitespace)?;
            let rest = rest.trim_start();
            let (ppid_s, cmd) = rest.split_once(char::is_whitespace)?;
            Some((pid_s.parse().ok()?, ppid_s.parse().ok()?, cmd.trim_start()))
        })
        .collect()
}

fn is_claude_cmdline(cmd: &str) -> bool {
    let mut args = cmd.split_whitespace();
    let Some(argv0) = args.next() else {
        return false;
    };
    let base = argv0.rsplit('/').next().unwrap_or(argv0);
    if base == "claude" {
        return true;
    }
    if base == "node" {
        return args.any(|a| a.contains("claude") && a.ends_with("cli.js"));
    }
    false
}

/// Pure: BFS the descendant tree of `pane_pid` in the ps snapshot.
pub fn find_claude_in_pane(pane_pid: i32, ps_output: &str) -> Option<ClaudeProc> {
    let procs = parse_ps(ps_output);
    let mut children: HashMap<i32, Vec<(i32, &str)>> = HashMap::new();
    for (pid, ppid, cmd) in &procs {
        children.entry(*ppid).or_default().push((*pid, cmd));
    }
    let mut queue = vec![pane_pid];
    while let Some(pid) = queue.pop() {
        for (child_pid, cmd) in children.get(&pid).into_iter().flatten() {
            if is_claude_cmdline(cmd) {
                return Some(ClaudeProc {
                    pid: *child_pid,
                    cmdline: (*cmd).to_string(),
                    resume_session_id: extract_resume_id(cmd),
                });
            }
            queue.push(*child_pid);
        }
    }
    None
}

pub fn extract_resume_id(cmdline: &str) -> Option<String> {
    let mut args = cmdline.split_whitespace();
    while let Some(a) = args.next() {
        if a == "--resume" {
            return args.next().map(str::to_string);
        }
    }
    None
}

/// cwd of a PID via lsof. Call only when a new claude PID appears (cached by
/// the poller) — lsof is not free.
pub fn cwd_of_pid(pid: i32) -> Option<PathBuf> {
    let out = Command::new("/usr/sbin/lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n').map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = " 1993     1075 -zsh\n77840     1993 claude --resume 01234567-89ab-cdef-0123-456789abcdef\n77900    77840 git status\n";

    #[test]
    fn find_claude_direct() {
        let c = find_claude_in_pane(1993, PS).unwrap();
        assert_eq!(c.pid, 77840);
        assert_eq!(
            c.resume_session_id.as_deref(),
            Some("01234567-89ab-cdef-0123-456789abcdef")
        );
    }

    #[test]
    fn find_claude_node_wrapper() {
        let ps = " 10 1 -zsh\n 20 10 node /Users/x/.nvm/versions/node/v24/lib/node_modules/@anthropic-ai/claude-code/cli.js\n";
        assert_eq!(find_claude_in_pane(10, ps).unwrap().pid, 20);
    }

    #[test]
    fn find_claude_grandchild() {
        let ps = " 10 1 -zsh\n 15 10 -zsh\n 20 15 claude\n";
        assert_eq!(find_claude_in_pane(10, ps).unwrap().pid, 20);
    }

    #[test]
    fn no_claude_in_plain_shell_pane() {
        let ps = " 30 1 -zsh\n 31 30 nvim .\n";
        assert!(find_claude_in_pane(30, ps).is_none());
    }

    #[test]
    fn resume_id_absent() {
        assert!(extract_resume_id("claude --dangerously-skip-permissions").is_none());
    }
}
