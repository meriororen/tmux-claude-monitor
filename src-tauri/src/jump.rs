//! Card click → focus iTerm2 and land on the exact tmux pane.

use crate::tmux;
use std::process::Command;

/// (switch-client target, select-window target, select-pane target).
/// '=' forces exact session-name matching; tmux otherwise falls back to
/// prefix/fnmatch matching, which can pick the wrong session.
pub fn switch_target(session: &str, window_index: u32, pane_index: u32) -> (String, String, String) {
    (
        format!("={session}"),
        format!("={session}:{window_index}"),
        format!("={session}:{window_index}.{pane_index}"),
    )
}

pub fn activate_iterm() -> Result<(), String> {
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", "tell application \"iTerm2\" to activate"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[tauri::command]
pub fn jump_to_session(
    app: tauri::AppHandle,
    session: String,
    window_index: u32,
    pane_index: u32,
) -> Result<(), String> {
    let bin = tmux::resolve_tmux_bin().map_err(|e| format!("tmux not found: {e:?}"))?;
    activate_iterm()?;
    tmux::switch_to(&bin, &session, window_index, pane_index)
        .map_err(|e| format!("tmux switch failed: {e:?}"))?;
    // Every jump hides the panel — keyboard Enter and mouse click alike
    // (v2 spec, decided with the user).
    crate::window::hide_panel_now(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_target_formatting() {
        // '=' forces exact session-name matching: without it tmux prefix/
        // fnmatch matching can land on the wrong session (review finding #8).
        assert_eq!(
            switch_target("neovim", 2, 0),
            (
                "=neovim".to_string(),
                "=neovim:2".to_string(),
                "=neovim:2.0".to_string()
            )
        );
    }
}
