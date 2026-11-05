//! Panel window behavior: always on top of everything, on every Space,
//! including over fullscreen apps. Tauri's flags cover always-on-top and
//! all-workspaces; the fullscreen-auxiliary collection behavior and the
//! elevated window level need a small NSWindow shim (spec: in scope for v1).

use crate::settings::Settings;

pub fn configure_panel(win: &tauri::WebviewWindow) {
    let _ = win.set_always_on_top(true);
    let _ = win.set_visible_on_all_workspaces(true);

    // Restore last saved position.
    let dir = crate::poller::config_dir(&win.app_handle().clone());
    if let Some((x, y)) = Settings::load(&dir).window_pos {
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    }

    // NSWindow shim: join all Spaces + float over fullscreen apps, at status
    // window level. Must run on the main thread (setup runs there).
    #[cfg(target_os = "macos")]
    if let Ok(ns) = win.ns_window() {
        use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
        let ns: &NSWindow = unsafe { &*(ns as *const NSWindow) };
        let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary;
        ns.setCollectionBehavior(behavior);
        // NSStatusWindowLevel = 25 (above normal floating windows).
        ns.setLevel(25);
    }
}

/// Show the panel, give it keyboard focus, and tell the frontend to reset
/// to normal mode ("panel-shown"). The only paths that may call this are
/// explicit summons: the global shortcut and the tray menu (anti-goal:
/// nothing else may steal focus).
pub fn show_panel(app: &tauri::AppHandle) {
    use tauri::Emitter;
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
        let _ = app.emit("panel-shown", ());
    }
}

pub fn hide_panel_now(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

/// ⌘⇧K semantics (decided with the user): pure visibility toggle —
/// show+focus if hidden, hide if visible, regardless of focus.
pub fn toggle_panel(app: &tauri::AppHandle) {
    match app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
    {
        Some(true) => hide_panel_now(app),
        _ => show_panel(app),
    }
}

/// Frontend Esc path.
#[tauri::command]
pub fn hide_panel(app: tauri::AppHandle) {
    hide_panel_now(&app);
}

use tauri::Manager;
