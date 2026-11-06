#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(all(not(debug_assertions), not(feature = "custom-protocol")))]
compile_error!("Release builds require --features omnisync/custom-protocol. Use scripts/build.ps1 to embed the frontend.");
use anyhow::{Context, Result};
use omnisync_core::{
    engine::Engine,
    model::{Config, Version},
};
use omnisync_windows::{filesystem, integration, registration};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
struct AppState {
    engine: Arc<Engine>,
    intent: Mutex<Option<Vec<String>>>,
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
#[tauri::command]
async fn command(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    action: String,
    payload: Option<Value>,
) -> std::result::Result<Value, String> {
    dispatch(app, &state, action, payload.unwrap_or(Value::Null))
        .await
        .map_err(|e| format!("{e:#}"))
}
async fn dispatch(
    app: tauri::AppHandle,
    state: &AppState,
    action: String,
    payload: Value,
) -> Result<Value> {
    let engine = &state.engine;
    let text = |key: &str| {
        payload[key]
            .as_str()
            .map(String::from)
            .context(format!("Missing {key}"))
    };
    match action.as_str() {
        "snapshot" => Ok(serde_json::to_value(engine.snapshot().await?)?),
        "items" => Ok(serde_json::to_value(engine.items()?)?),
        "intent" => Ok(json!(state.intent.lock().unwrap().take())),
        "pickRoot" => {
            let file = payload["file"].as_bool().unwrap_or(false);
            let path = tokio::task::spawn_blocking(move || {
                let dialog = rfd::FileDialog::new().set_title("Choose an OmniSync backup source");
                if file {
                    dialog.pick_file()
                } else {
                    dialog.pick_folder()
                }
            })
            .await?;
            Ok(json!(path.map(|p| filesystem::display(&p))))
        }
        "preview" => {
            let config: Config = serde_json::from_value(payload["config"].clone())?;
            let description = text("description")?;
            let engine = engine.clone();
            Ok(serde_json::to_value(
                tokio::task::spawn_blocking(move || engine.preview(config, description)).await??,
            )?)
        }
        "shellPreview" => {
            let path = PathBuf::from(text("path")?);
            let include = payload["include"].as_bool().unwrap_or(false);
            let engine = engine.clone();
            Ok(serde_json::to_value(
                tokio::task::spawn_blocking(move || engine.shell_preview(&path, include)).await??,
            )?)
        }
        "apply" => {
            let before = engine.config()?;
            engine.apply_preview(&text("token")?)?;
            let after = engine.config()?;
            if before.start_at_sign_in != after.start_at_sign_in {
                integration::startup(after.start_at_sign_in, &std::env::current_exe()?)?;
            }
            Ok(Value::Null)
        }
        "undo" => Ok(serde_json::to_value(engine.undo_preview()?)?),
        "pause" => {
            engine.pause(Some(payload["seconds"].as_i64().unwrap_or(0)))?;
            Ok(Value::Null)
        }
        "resume" => {
            engine.pause(None)?;
            Ok(Value::Null)
        }
        "syncNow" => {
            engine.sync_now()?;
            Ok(Value::Null)
        }
        "importOAuth" => {
            let path = tokio::task::spawn_blocking(|| {
                rfd::FileDialog::new()
                    .set_title("Import Google OAuth Desktop app JSON")
                    .add_filter("Google OAuth client", &["json"])
                    .pick_file()
            })
            .await?;
            if let Some(path) = path {
                engine.auth.import_client(&path).await?;
                Ok(json!(true))
            } else {
                Ok(json!(false))
            }
        }
        "connect" => {
            engine.connect().await?;
            Ok(Value::Null)
        }
        "disconnect" => {
            engine.disconnect().await?;
            Ok(Value::Null)
        }
        "openDrive" => {
            let snapshot = engine.snapshot().await?;
            let url = snapshot
                .account
                .base_id
                .map(|id| format!("https://drive.google.com/drive/folders/{id}"))
                .unwrap_or_else(|| "https://drive.google.com/".into());
            integration::open_url(&url)?;
            Ok(Value::Null)
        }
        "catalogs" => engine.catalogs().await,
        "readCatalog" => Ok(serde_json::to_value(
            engine.read_catalog(&text("id")?).await?,
        )?),
        "restore" => {
            let version: Version = serde_json::from_value(payload["version"].clone())?;
            let relative = text("relative")?;
            let destination = tokio::task::spawn_blocking(|| {
                rfd::FileDialog::new()
                    .set_title("Restore outside your backup folders")
                    .pick_folder()
            })
            .await?;
            if let Some(destination) = destination {
                Ok(json!(engine.restore(version, relative, destination).await?))
            } else {
                Ok(Value::Null)
            }
        }
        "acknowledgeConflict" => {
            engine.acknowledge_conflict(&text("id")?)?;
            Ok(Value::Null)
        }
        "inventory" => Ok(serde_json::to_value(registration::inventory()?)?),
        "takeover" => {
            let selected: Vec<(String, String)> =
                serde_json::from_value(payload["selected"].clone())?;
            anyhow::ensure!(!selected.is_empty(), "Select specific registrations first");
            let path = engine.state.join("overlay-selection.json");
            std::fs::write(&path, serde_json::to_vec(&selected)?)?;
            let arg = format!("takeover-json \"{}\"", path.display());
            tokio::task::spawn_blocking(move || elevated(&arg)).await??;
            Ok(Value::Null)
        }
        "restoreOverlays" => {
            let receipt = tokio::task::spawn_blocking(|| {
                rfd::FileDialog::new()
                    .set_title("Choose OmniSync overlay recovery receipt")
                    .add_filter("Overlay recovery receipt", &["json"])
                    .pick_file()
            })
            .await?;
            if let Some(path) = receipt {
                let arg = format!("restore \"{}\"", path.display());
                tokio::task::spawn_blocking(move || elevated(&arg)).await??;
            }
            Ok(Value::Null)
        }
        "registerShell" => {
            tokio::task::spawn_blocking(|| elevated("register --overlays --context")).await??;
            Ok(Value::Null)
        }
        "hide" => {
            if let Some(window) = app.get_webview_window("main") {
                window.hide()?;
            }
            Ok(Value::Null)
        }
        "quit" => {
            engine.shutdown();
            app.exit(0);
            Ok(Value::Null)
        }
        _ => anyhow::bail!("Unknown application command"),
    }
}
fn elevated(arguments: &str) -> Result<()> {
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::CloseHandle,
            System::Threading::{GetExitCodeProcess, WaitForSingleObject},
            UI::{
                Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
                WindowsAndMessaging::SW_HIDE,
            },
        },
    };
    let helper = std::env::current_exe()?.with_file_name("omnisync-setup.exe");
    anyhow::ensure!(
        helper.is_file(),
        "Install OmniSync to use Explorer setup. The helper must be next to the app."
    );
    let file = omnisync_windows::wide(helper);
    let arguments = omnisync_windows::wide(arguments);
    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(arguments.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        ShellExecuteExW(&mut info)
            .context("Windows setup elevation was canceled or could not start")?;
        WaitForSingleObject(info.hProcess, u32::MAX);
        let mut code = 1;
        let result = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        result?;
        anyhow::ensure!(code==0,"Explorer setup stopped (code {code}). Review the selected registrations and recovery receipts in ProgramData\\OmniSync\\OverlayBackups.");
    }
    Ok(())
}
fn main() {
    if let Err(error) = start() {
        rfd::MessageDialog::new()
            .set_title("OmniSync could not start")
            .set_description(format!("{error:#}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}
fn start() -> Result<()> {
    let initial: Vec<_> = std::env::args().collect();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            *state.intent.lock().unwrap() = Some(args);
            show(app);
            let _ = app.emit("shell-intent", ());
        }))
        .invoke_handler(tauri::generate_handler![command]);
    let app = builder
        .setup(move |app| {
            let directory = app.path().app_local_data_dir()?;
            let engine = Engine::open(directory)?;
            app.manage(AppState {
                engine: engine.clone(),
                intent: Mutex::new(Some(initial.clone())),
            });
            let handle = app.handle().clone();
            let open = MenuItem::with_id(app, "open", "Open OmniSync", true, None::<&str>)?;
            let sync = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pause backup", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit OmniSync", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &sync, &pause, &quit])?;
            let tray = TrayIconBuilder::with_id("omnisync")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../../assets/tray/idle.png"
                ))?)
                .tooltip("OmniSync — starting")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let state = app.state::<AppState>();
                    match event.id.as_ref() {
                        "open" => show(app),
                        "sync" => {
                            if let Err(error) = state.engine.sync_now() {
                                let _ = app.emit("app-notice", error.to_string());
                                show(app);
                            }
                        }
                        "pause" => {
                            let paused = state.engine.config().map(|c| c.paused()).unwrap_or(false);
                            let _ = state.engine.pause(if paused { None } else { Some(0) });
                        }
                        "quit" => {
                            state.engine.shutdown();
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            if !initial.iter().any(|a| a == "--tray") {
                show(&handle);
            }
            let runner = engine.clone();
            tauri::async_runtime::spawn(async move {
                runner.run().await;
            });
            tauri::async_runtime::spawn(async move {
                let frames = tray_frames();
                let mut frame = 0;
                loop {
                    if let Ok((paused, phase, counts)) = engine.tray_status() {
                        let _ = pause.set_text(if paused {
                            "Resume backup"
                        } else {
                            "Pause backup"
                        });
                        let _ = sync.set_enabled(!paused);
                        let active = phase == "syncing" && !paused;
                        let bytes = if active && !filesystem::reduced_motion() {
                            let bytes = frames[frame % frames.len()];
                            frame += 1;
                            bytes
                        } else if paused {
                            include_bytes!("../../assets/tray/paused.png").as_slice()
                        } else if phase == "issue" {
                            include_bytes!("../../assets/tray/issue.png").as_slice()
                        } else if phase == "disconnected" {
                            include_bytes!("../../assets/tray/offline.png").as_slice()
                        } else if counts.pending > 0 {
                            include_bytes!("../../assets/tray/pending.png").as_slice()
                        } else if counts.backed_up > 0 {
                            include_bytes!("../../assets/tray/backed.png").as_slice()
                        } else {
                            include_bytes!("../../assets/tray/idle.png").as_slice()
                        };
                        if let Ok(image) = tauri::image::Image::from_bytes(bytes) {
                            let _ = tray.set_icon(Some(image));
                        }
                        let _ = tray.set_tooltip(Some(format!(
                            "OmniSync — {}\n{} backed up · {} pending · {} issues",
                            phase, counts.backed_up, counts.pending, counts.issues
                        )));
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())?;
    let mutex = unsafe {
        windows::Win32::System::Threading::CreateMutexW(
            None,
            false,
            windows::core::w!("Global\\OmniSync-App-Running"),
        )
        .ok()
    };
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app.state::<AppState>().engine.shutdown();
        }
    });
    if let Some(mutex) = mutex {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(mutex);
        }
    }
    Ok(())
}
include!("tray_frames.rs");
