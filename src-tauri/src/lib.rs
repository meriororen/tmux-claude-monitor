pub mod tray;
pub mod window;
pub mod jump;
pub mod poller;
pub mod settings;
pub mod favorites;
pub mod state_detect;
pub mod transcript;
pub mod procs;
pub mod tmux;

/// Shared favorites store: the poller clones it each tick, commands mutate it
/// and write through to favorites.json.
pub struct FavState(pub std::sync::Mutex<favorites::Favorites>);

/// session uuid → (tmux session, window, pane) of live sessions; rebuilt each tick.
pub struct LiveIndex(pub std::sync::Mutex<std::collections::HashMap<String, (String, u32, u32)>>);

/// Result of registering the global shortcut at launch: None = registered,
/// Some(err) = failed (e.g. taken by another app). The tray menu surfaces it.
pub struct ShortcutStatus(pub Option<String>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::Manager;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        window::toggle_panel(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            jump::jump_to_session,
            favorites::toggle_favorite,
            favorites::resume_session,
            settings::save_window_pos,
            settings::open_settings,
            window::hide_panel
        ])
        .on_window_event(|window, event| {
            // Close never quits: the app is a background utility. Hide instead.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // Menu-bar utility: no Dock icon, no ⌘Tab entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            // Register the summon shortcut; a failure (conflict) must not
            // stop the app — it is surfaced in the tray menu (Task 3).
            let shortcut = settings::Settings::load(&poller::config_dir(app.handle())).global_shortcut;
            let err = app
                .global_shortcut()
                .register(shortcut.as_str())
                .err()
                .map(|e| format!("{shortcut}: {e}"));
            app.manage(ShortcutStatus(err));
            app.manage(FavState(std::sync::Mutex::new(favorites::Favorites::load(
                &poller::config_dir(app.handle()),
            ))));
            app.manage(LiveIndex(std::sync::Mutex::new(Default::default())));
            tray::setup_tray(app, app.state::<ShortcutStatus>().0.clone())?;
            poller::spawn(app.handle().clone());
            if let Some(win) = app.get_webview_window("main") {
                window::configure_panel(&win);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
