// Pure-logic modules (unit-tested): `unreal` (detection + arg builder),
// `pipeline` (phase registry), `profiles` (schema/persist), `storage`, `footprint`
// (M5 - categorization rules + scan/clean, all `tempdir`-testable), and `runner`
// (M3 - its `classify`/`plan` are tested; the executor is cfg(not test)).
mod footprint;
mod history;
mod pipeline;
mod profiles;
mod runner;
mod settings;
mod storage;
mod unreal;

// Tauri-facing surface. Gated out of `cfg(test)` so the lib *test* binary doesn't
// link the webview/dialog runtime - that pulls Windows DLL imports the bare test
// executable can't resolve (STATUS_ENTRYPOINT_NOT_FOUND). Run pure tests with
// `cargo test --lib`; the app + IPC are exercised by running the app itself.
#[cfg(not(test))]
mod commands;
#[cfg(not(test))]
mod state;

#[cfg(not(test))]
use tauri_specta::{collect_commands, Builder};

/// The tauri-specta command registry - shared by the running app and the
/// TypeScript binding export so they can never drift.
#[cfg(not(test))]
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::validate_project,
        commands::open_project,
        commands::open_plugin,
        commands::current_plugin,
        commands::locate_engine,
        commands::list_recents,
        commands::remove_recent,
        commands::set_recent_starred,
        // M2 - profiles, templates, arg-builder preview, phase registry
        commands::current_project,
        commands::list_profiles,
        commands::create_profile,
        commands::duplicate_profile,
        commands::save_profile,
        commands::delete_profile,
        commands::list_templates,
        commands::create_template,
        commands::save_template,
        commands::delete_template,
        commands::preview_profile,
        commands::phase_registry,
        // M3 - runner: start / cancel / live snapshot (logs stream via uep://run-* events)
        commands::start_build,
        commands::cancel_build,
        commands::active_run,
        commands::check_output,
        // M4 - build history (records under .uep/history/, indexed in .uep/cache/history.db)
        commands::list_history,
        commands::list_history_page,
        commands::history_detail,
        commands::delete_history,
        commands::check_build_location,
        // M5 - footprint: scan (off-thread) into the Clean-tab tree + guarded clean
        commands::scan_footprint,
        commands::clean_footprint,
        // M6 - app settings (theme + notification prefs) + about version
        commands::load_settings,
        commands::save_settings,
        commands::app_version,
        // M6 - close-to-tray when a build is running (hides main, shows tray)
        commands::minimize_to_tray,
        // M7 - plugin packaging (RunUAT BuildPlugin): engine picker + package run
        commands::list_engines,
        commands::add_custom_engine,
        commands::preview_plugin_package,
        commands::start_plugin_package,
        commands::load_plugin_settings,
        commands::save_plugin_output,
        // Editor commandlet tools (project Tools tab): Resave / Validate
        commands::start_resave,
        commands::start_validate,
        // Remove UEP data from the open project (.uep) or plugin (.uap)
        commands::remove_uep_data,
    ])
}

#[cfg(not(test))]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // Regenerate TS bindings (`src/bindings.ts`) on every debug launch so the
    // frontend stays in sync with the Rust command/type definitions.
    #[cfg(debug_assertions)]
    builder
        .export(specta_typescript::Typescript::default(), "../src/bindings.ts")
        .expect("failed to export typescript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // M6 - OS toasts on build finish (fired from the runner, gated on the saved
        // notification preference).
        .plugin(tauri_plugin_notification::init())
        .manage(state::AppState::default())
        .setup(|app| {
            // The tray exists from launch but stays hidden - it only appears when the
            // user chooses "minimize to tray" on closing the main window mid-build
            // (`minimize_to_tray`), and hides again the moment the window is restored.
            setup_tray(app)?;
            Ok(())
        })
        .invoke_handler(builder.invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        // The **main** window owns the app lifecycle: when it's destroyed (a real
        // close, or "close & discard"), the whole app exits - auxiliary windows
        // (Settings, Build Settings, Build Logs) never keep it alive. Minimizing to
        // tray *hides* the main window (it isn't destroyed), so the app keeps running.
        .run(|app_handle, event| {
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = &event
            {
                if label == "main" {
                    app_handle.exit(0);
                }
            }
        });
}

/// Build the (initially hidden) system-tray icon: a Show/Quit menu plus left-click,
/// both of which restore the main window and re-hide the tray.
#[cfg(not(test))]
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItemBuilder::with_id("show", "Show UnrealEasyPackage").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("UnrealEasyPackage - build running")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => restore_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                restore_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    let tray = tray.build(app)?;
    tray.set_visible(false)?; // hidden until the user minimizes to tray
    Ok(())
}

/// Restore the main window (show + raise + focus) and re-hide the tray icon.
#[cfg(not(test))]
fn restore_main(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_visible(false);
    }
}
