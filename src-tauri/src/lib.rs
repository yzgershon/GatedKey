//! GatedKey - remap any key on your keyboard to launch anything.

mod actions;
mod config;
mod hook;
mod keys;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;

use config::{Action, Binding, Config};
use keys::Chord;

struct AppState {
    config: Mutex<Config>,
    path: PathBuf,
}

#[derive(Serialize, Clone)]
struct AppSuggestion {
    name: String,
    path: String,
}

fn config_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("config.json")
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.config.lock().expect("config lock").clone()
}

#[tauri::command]
fn save_config(new: Config, app: AppHandle, state: State<AppState>) -> Result<(), String> {
    hook::set_bindings(new.bindings.clone());
    hook::set_enabled(new.enabled);

    // Autostart is handled here rather than in the frontend, so the setting and
    // the registry entry can never disagree after a failed save.
    let launcher = app.autolaunch();
    let _ = if new.start_with_windows {
        launcher.enable()
    } else {
        launcher.disable()
    };

    new.save(&state.path).map_err(|e| e.to_string())?;
    *state.config.lock().expect("config lock") = new;
    Ok(())
}

/// Open a file picker. Non-blocking: the result comes back as an event, because
/// blocking a command on a modal dialog deadlocks the main thread.
#[tauri::command]
fn pick_app(app: AppHandle) {
    let handle = app.clone();
    app.dialog()
        .file()
        .add_filter("Programs", &["exe", "lnk", "bat", "cmd"])
        .pick_file(move |path| {
            if let Some(path) = path {
                let _ = handle.emit("file-picked", path.to_string());
            }
        });
}

#[tauri::command]
fn pick_folder(app: AppHandle) {
    let handle = app.clone();
    app.dialog().file().pick_folder(move |path| {
        if let Some(path) = path {
            let _ = handle.emit("file-picked", path.to_string());
        }
    });
}

/// Turn the capture overlay on or off. While on, every non-modifier keypress is
/// swallowed and reported to the UI instead of doing its normal job.
#[tauri::command]
fn set_learn_mode(on: bool) {
    hook::set_learn_mode(on);
}

/// Describe a chord for display, so the UI never has to know virtual-key codes.
#[tauri::command]
fn describe_chord(chord: Chord) -> String {
    chord.label()
}

/// The chord a Copilot key emits, for the quick-start.
#[tauri::command]
fn copilot_chord() -> Chord {
    Chord::copilot_key()
}

/// Run a binding's action now, so the user can confirm it works without having
/// to press the key and hope.
#[tauri::command]
fn test_action(action: Action) -> Result<(), String> {
    actions::execute(&action)
}

/// Shortcuts from both Start Menu roots, so the app picker is populated without
/// the user hunting through Program Files for an .exe.
#[tauri::command]
fn suggest_apps() -> Vec<AppSuggestion> {
    let mut out: Vec<AppSuggestion> = Vec::new();
    let roots = [
        std::env::var("APPDATA").ok().map(PathBuf::from),
        std::env::var("ProgramData").ok().map(PathBuf::from),
    ];
    for root in roots.into_iter().flatten() {
        let programs = root.join(r"Microsoft\Windows\Start Menu\Programs");
        collect_shortcuts(&programs, &mut out, 0);
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    out
}

fn collect_shortcuts(dir: &std::path::Path, out: &mut Vec<AppSuggestion>, depth: usize) {
    if depth > 4 || out.len() > 500 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_shortcuts(&path, out, depth + 1);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
        {
            if let Some(stem) = path.file_stem() {
                let name = stem.to_string_lossy().into_owned();
                if name.to_lowercase().contains("uninstall") {
                    continue;
                }
                out.push(AppSuggestion {
                    name,
                    path: path.to_string_lossy().into_owned(),
                });
            }
        }
    }
}

#[tauri::command]
fn open_config_folder(state: State<AppState>) -> Result<(), String> {
    let dir = state.path.parent().ok_or("no config folder")?;
    actions::execute(&Action::OpenFolder {
        path: dir.to_string_lossy().into_owned(),
    })
}

// ------------------------------------------------------------------- tray

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open GatedKey", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("gatedkey-tray")
        .menu(&menu)
        .tooltip("GatedKey")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                reveal_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn reveal_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// -------------------------------------------------------------------- run

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // Windows passes this when it starts us at login, which is the only
            // way to tell an automatic launch from the user opening the app.
            Some(vec!["--autostart"]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            set_learn_mode,
            describe_chord,
            copilot_chord,
            test_action,
            suggest_apps,
            open_config_folder,
            pick_app,
            pick_folder,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let path = config_path(&handle);
            let first_run = !path.exists();
            let mut cfg = Config::load(&path);

            // A key remapper that does not survive a reboot is not a working key
            // remapper: the hook dies with the process and every bound key
            // silently reverts. So autostart is on by default, and the checkbox
            // in the footer shows it rather than hiding it.
            if first_run {
                cfg.start_with_windows = true;
                let _ = cfg.save(&path);
            }

            // Re-apply on every launch so the registry entry and the checkbox
            // cannot drift apart, which would leave the UI lying about it.
            let launcher = handle.autolaunch();
            let _ = if cfg.start_with_windows {
                launcher.enable()
            } else {
                launcher.disable()
            };

            if hook::debug_on() {
                eprintln!(
                    "gatedkey: loaded {} binding(s) from {}",
                    cfg.bindings.len(),
                    path.display()
                );
            }
            hook::set_bindings(cfg.bindings.clone());
            hook::set_enabled(cfg.enabled);
            // Only hide when Windows launched us at login. If the user opened the
            // app themselves they want to see it, and an app that starts to
            // nothing visible reads as an app that failed to start.
            let launched_at_login = std::env::args().any(|a| a == "--autostart");
            let start_hidden = cfg.start_minimised && launched_at_login;
            app.manage(AppState {
                config: Mutex::new(cfg),
                path,
            });

            let action_handle = handle.clone();
            let learn_handle = handle.clone();
            // Must run on the main thread: see the note on hook::start.
            let installed = hook::start(
                move |binding: Binding| {
                    if hook::debug_on() {
                        eprintln!("gatedkey: firing '{}'", binding.name);
                    }
                    if let Err(err) = actions::execute(&binding.action) {
                        eprintln!("gatedkey: '{}' failed: {err}", binding.name);
                        let _ = action_handle.emit("action-error", err);
                    }
                },
                move |chord: Chord| {
                    let _ = learn_handle.emit("chord-captured", chord);
                },
            );

            if !installed {
                // Worth shouting about: without the hook the app runs, shows
                // every binding, and silently does nothing at all.
                eprintln!("gatedkey: hook not installed, no key will fire");
                let _ = handle.emit(
                    "action-error",
                    "GatedKey could not install its keyboard hook. No keys will work.",
                );
            }

            build_tray(&handle)?;

            if !start_hidden {
                reveal_window(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides to the tray. The hook dies with the process, so a
            // real close would silently stop every binding the user set up.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running GatedKey");
}
