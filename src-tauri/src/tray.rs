//! Menu-bar (tray) presence: the "app is running" signal, Show/Hide + Quit,
//! and a title showing how many sessions are waiting for input.

use crate::poller::{CardState, SessionCard};

pub struct TrayHandle(pub std::sync::Mutex<tauri::tray::TrayIcon>);

pub fn waiting_count(cards: &[SessionCard]) -> usize {
    cards
        .iter()
        .filter(|c| {
            matches!(
                c.state,
                CardState::NeedsInputStructured | CardState::NeedsInputProse
            )
        })
        .count()
}

pub fn tray_title(waiting: usize) -> String {
    if waiting == 0 {
        String::new()
    } else {
        format!("● {waiting}")
    }
}

pub fn shortcut_menu_label(err: Option<&str>) -> Option<String> {
    err.map(|e| format!("⚠ Shortcut unavailable: {e}"))
}

pub fn setup_tray(app: &tauri::App, shortcut_err: Option<String>) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;

    let toggle = MenuItem::with_id(app, "toggle", "Show/Hide Panel", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let mut menu = MenuBuilder::new(app).item(&toggle);
    if let Some(label) = shortcut_menu_label(shortcut_err.as_deref()) {
        let err_item = MenuItem::with_id(app, "shortcut-error", &label, false, None::<&str>)?;
        menu = menu.item(&err_item);
    }
    let menu = menu.separator().item(&quit).build()?;

    let tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => crate::window::toggle_panel(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(TrayHandle(std::sync::Mutex::new(tray)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poller::{CardState, SessionCard};

    fn card(id: &str, state: CardState) -> SessionCard {
        SessionCard {
            pane_id: id.to_string(),
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
    fn tray_title_formats() {
        assert_eq!(tray_title(0), "");
        assert_eq!(tray_title(2), "● 2");
    }

    #[test]
    fn waiting_counts_structured_and_prose() {
        let cards = vec![
            card("%1", CardState::Working),
            card("%2", CardState::NeedsInputStructured),
            card("%3", CardState::NeedsInputProse),
            card("%4", CardState::Stale),
        ];
        assert_eq!(waiting_count(&cards), 2);
    }

    #[test]
    fn shortcut_label_only_on_error() {
        assert!(shortcut_menu_label(None).is_none());
        assert!(shortcut_menu_label(Some("taken")).unwrap().contains("taken"));
    }
}
