pub mod commands;
pub mod dicom;
pub mod hud;
pub mod privacy;
pub mod protocol;
pub mod state;
pub mod wsi;

use commands::*;
use state::AppState;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{
    Builder as ShortcutBuilder, GlobalShortcutExt, Shortcut, ShortcutState,
};

pub fn run() {
    let shortcut_plugin = ShortcutBuilder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                let app_state = app.state::<AppState>();
                let is_visible = app_state.hud.toggle_visibility();
                let _ = app.emit("lamina:hud-toggled", is_visible);
            }
        })
        .build();

    let mut builder = tauri::Builder::default()
        .plugin(shortcut_plugin)
        .manage(AppState::new());

    // Register custom lamina-tile:// protocol for zero-copy binary streaming
    builder = protocol::register_tile_protocol(builder);

    builder
        .invoke_handler(tauri::generate_handler![
            load_slide,
            get_slide_metadata,
            extract_roi_patch,
            load_dicom_series,
            load_synthetic_dicom,
            get_dicom_slice,
            get_clinical_presets,
            toggle_quick_query_hud,
            submit_diagnostic_query,
            open_file_dialog,
            set_active_viewport,
            reader_extract_selection,
            reader_navigate,
            handle_viewport_command,
        ])
        .setup(|app| {
            // Register global shortcut: Option+Space (macOS) / Alt+Space (Windows)
            if let Ok(shortcut) = "Alt+Space".parse::<Shortcut>() {
                let _ = app.global_shortcut().register(shortcut);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Lamina workstation");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shortcut_parsing() {
        let alt = "Alt+Space".parse::<Shortcut>();
        let opt = "Option+Space".parse::<Shortcut>();
        println!("Alt+Space: {:?}", alt);
        println!("Option+Space: {:?}", opt);
    }
}

