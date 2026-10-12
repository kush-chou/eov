use crate::dicom::volume::{DicomVolume, MprPlane};
use crate::hud::DiagnosticQueryRequest;
pub use crate::reader::{ReaderDocument, ReaderExcerpt};
use crate::state::AppState;
use crate::wsi::coordinates::SlideRect;
use crate::wsi::engine::SlideMetadata;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetInfo {
    pub name: String,
    pub center: f32,
    pub width: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceImageResponse {
    pub width: u32,
    pub height: u32,
    pub plane: String,
    pub slice_index: u32,
    pub aspect_ratio: f32,
    pub png_base64: String,
}

/// ViewportCommand IPC contract matching SPEC-00-MASTER.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ViewportCommand {
    LoadSlide { path: String },
    LoadDicom { series_dir: String },
    RequestTile { level: u32, tile_x: u32, tile_y: u32 },
    SetWindowLevel { window_width: f32, window_level: f32 },
    ExtractRoiPatch { rect: SlideRect, max_dimension: u32 },
}

/// ActiveViewportChangedEvent matching SPEC-01-SHELL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveViewportChangedEvent {
    pub viewport_id: String,
    pub kind: String, // "wsi" | "dicom" | "reader"
}

#[tauri::command]
pub async fn open_file_dialog(filter_type: String) -> Result<Option<String>, String> {
    match filter_type.as_str() {
        "wsi" => Ok(Some("sample".to_string())),
        "dicom" => Ok(Some("synthetic".to_string())),
        _ => Ok(None),
    }
}

#[tauri::command]
pub async fn set_active_viewport(
    viewport_id: String,
    kind: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    state.hud.set_active_viewport(viewport_id.clone());
    let event = ActiveViewportChangedEvent { viewport_id, kind };
    let _ = app_handle.emit("lamina:active-viewport-changed", event);
    Ok(())
}

#[tauri::command]
pub async fn reader_extract_selection(
    _window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<Option<ReaderExcerpt>, String> {
    Ok(state.reader.get_selection())
}

#[tauri::command]
pub async fn reader_set_selection(
    excerpt: ReaderExcerpt,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.reader.set_selection(excerpt);
    Ok(())
}

#[tauri::command]
pub async fn reader_clear_selection(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.reader.clear_selection();
    Ok(())
}

#[tauri::command]
pub async fn reader_get_documents(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ReaderDocument>, String> {
    Ok(state.reader.documents.clone())
}

#[tauri::command]
pub async fn reader_navigate(
    window: tauri::WebviewWindow,
    url: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    crate::reader::ReaderState::validate_url(&url).map_err(|e| e.to_string())?;
    let _ = state.reader.set_active_url(url.clone());
    let url_json = serde_json::to_string(&url).map_err(|e| e.to_string())?;
    let script = format!("window.location.href = {};", url_json);
    window.eval(&script).map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn handle_viewport_command_impl(
    command: ViewportCommand,
    state: &AppState,
) -> Result<serde_json::Value, String> {
    match command {
        ViewportCommand::LoadSlide { path } => {
            let engine = state.wsi.clone();
            let meta = tokio::task::spawn_blocking(move || engine.open_slide(&path).map_err(|e| e.to_string()))
                .await
                .map_err(|e| e.to_string())??;
            Ok(serde_json::to_value(meta).map_err(|e| e.to_string())?)
        }
        ViewportCommand::LoadDicom { series_dir } => {
            let volume = tokio::task::spawn_blocking(move || {
                DicomVolume::load_series(&series_dir).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())??;

            let mut guard = state.dicom.write().unwrap();
            *guard = Some(volume.clone());
            Ok(serde_json::to_value(volume).map_err(|e| e.to_string())?)
        }
        ViewportCommand::RequestTile { level, tile_x, tile_y } => {
            let png_bytes = match state.wsi.get_tile_png("sample", level, tile_x, tile_y) {
                Ok(bytes) => bytes,
                Err(_) => {
                    let rgba = crate::protocol::generate_synthetic_he_tile(level, tile_x, tile_y);
                    crate::wsi::engine::encode_rgba_to_png(&rgba, crate::wsi::coordinates::TILE_SIZE, crate::wsi::coordinates::TILE_SIZE)
                        .map_err(|e| e.to_string())?
                }
            };
            let b64 = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &png_bytes);
            Ok(serde_json::json!({ "png_base64": b64 }))
        }
        ViewportCommand::SetWindowLevel { window_width, window_level } => {
            let mut guard = state.dicom.write().unwrap();
            if let Some(vol) = guard.as_mut() {
                vol.default_window = (window_level, window_width);
            }
            Ok(serde_json::json!({ "window_width": window_width, "window_level": window_level }))
        }
        ViewportCommand::ExtractRoiPatch { rect, max_dimension } => {
            let engine = state.wsi.clone();
            let bytes = tokio::task::spawn_blocking(move || {
                match engine.extract_roi_patch("sample", rect, max_dimension) {
                    Ok(b) => Ok(b),
                    Err(_) => {
                        let dim = max_dimension.min(1024).max(1);
                        let rgba = crate::protocol::generate_synthetic_he_tile(0, 0, 0);
                        crate::wsi::engine::encode_rgba_to_png(&rgba, dim.min(256), dim.min(256))
                            .map_err(|e| e.to_string())
                    }
                }
            })
            .await
            .map_err(|e| e.to_string())??;
            let b64 = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &bytes);
            Ok(serde_json::json!({ "patch_base64": b64 }))
        }
    }
}

#[tauri::command]
pub async fn handle_viewport_command(
    command: ViewportCommand,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    handle_viewport_command_impl(command, &state).await
}

#[tauri::command]
pub async fn load_slide(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<SlideMetadata, String> {
    let engine = state.wsi.clone();
    tokio::task::spawn_blocking(move || engine.open_slide(&path).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_slide_metadata(
    slide_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<SlideMetadata, String> {
    state.wsi.get_metadata(&slide_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn extract_roi_patch(
    slide_id: String,
    rect: SlideRect,
    max_dimension_px: Option<u32>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<u8>, String> {
    let engine = state.wsi.clone();
    let max_dim = max_dimension_px.unwrap_or(1024);
    tokio::task::spawn_blocking(move || {
        engine
            .extract_roi_patch(&slide_id, rect, max_dim)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn load_dicom_series(
    series_dir: String,
    state: tauri::State<'_, AppState>,
) -> Result<DicomVolume, String> {
    let volume = tokio::task::spawn_blocking(move || {
        DicomVolume::load_series(&series_dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut guard = state.dicom.write().unwrap();
    *guard = Some(volume.clone());

    Ok(volume)
}

#[tauri::command]
pub async fn load_synthetic_dicom(
    width: Option<u32>,
    height: Option<u32>,
    slices: Option<u32>,
    state: tauri::State<'_, AppState>,
) -> Result<DicomVolume, String> {
    let w = width.unwrap_or(256);
    let h = height.unwrap_or(256);
    let s = slices.unwrap_or(32);

    let volume = DicomVolume::generate_synthetic(w, h, s);
    let mut guard = state.dicom.write().unwrap();
    *guard = Some(volume.clone());

    Ok(volume)
}

#[tauri::command]
pub async fn get_dicom_slice(
    plane: String,
    slice_index: u32,
    window_center: Option<f32>,
    window_width: Option<f32>,
    state: tauri::State<'_, AppState>,
) -> Result<SliceImageResponse, String> {
    let mpr_plane = match plane.to_lowercase().as_str() {
        "coronal" => MprPlane::Coronal,
        "sagittal" => MprPlane::Sagittal,
        _ => MprPlane::Axial,
    };

    let guard = state.dicom.read().unwrap();
    let volume = guard
        .as_ref()
        .ok_or_else(|| "No active DICOM volume loaded".to_string())?;

    let win = match (window_center, window_width) {
        (Some(c), Some(w)) => Some((c, w)),
        _ => None,
    };

    let slice = volume
        .extract_slice(mpr_plane, slice_index, win)
        .map_err(|e| e.to_string())?;

    let png_bytes = slice.to_png().map_err(|e| e.to_string())?;
    let png_base64 = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &png_bytes);

    Ok(SliceImageResponse {
        width: slice.width,
        height: slice.height,
        plane,
        slice_index,
        aspect_ratio: slice.aspect_ratio,
        png_base64,
    })
}

#[tauri::command]
pub async fn get_clinical_presets() -> Result<Vec<PresetInfo>, String> {
    Ok(vec![
        PresetInfo {
            name: "Soft Tissue".to_string(),
            center: 40.0,
            width: 400.0,
        },
        PresetInfo {
            name: "Lung".to_string(),
            center: -600.0,
            width: 1500.0,
        },
        PresetInfo {
            name: "Bone".to_string(),
            center: 400.0,
            width: 1500.0,
        },
        PresetInfo {
            name: "Brain".to_string(),
            center: 40.0,
            width: 80.0,
        },
    ])
}

#[tauri::command]
pub async fn toggle_quick_query_hud(app_handle: tauri::AppHandle) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    let is_now_visible = state.hud.toggle_visibility();

    // Emit event to all webviews so HUD can animate in/out
    let _ = app_handle.emit("lamina:hud-toggled", is_now_visible);
    Ok(())
}

#[tauri::command]
pub async fn submit_diagnostic_query(
    request: DiagnosticQueryRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    // Optical Allow-List check before anything is dispatched
    request
        .validate_optical_allow_list()
        .map_err(|e| e.to_string())?;

    let interaction_id = state.intelligence.prepare_session();
    let app = app_handle.clone();
    let emitter = move |event: crate::hud::DiagnosticStreamEvent| {
        let _ = app.emit("lamina:diagnostic-stream", event);
    };

    let intel = state.intelligence.clone();
    let id_clone = interaction_id.clone();
    tokio::spawn(async move {
        let _ = intel
            .stream_diagnostic_query_with_id(&id_clone, request, emitter)
            .await;
    });

    Ok(interaction_id)
}

pub fn cancel_diagnostic_query_impl(
    interaction_id: Option<String>,
    state: &AppState,
) -> bool {
    if let Some(id) = interaction_id {
        state.intelligence.cancel_session(&id)
    } else {
        state.intelligence.cancel_all();
        true
    }
}

#[tauri::command]
pub async fn cancel_diagnostic_query(
    interaction_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    Ok(cancel_diagnostic_query_impl(interaction_id, &state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsi::coordinates::SlidePoint;

    #[test]
    fn test_viewport_command_serde() {
        let cmd = ViewportCommand::LoadSlide {
            path: "/path/to/slide.svs".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"LoadSlide\""));
        let deserialized: ViewportCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, ViewportCommand::LoadSlide { .. }));

        let cmd_roi = ViewportCommand::ExtractRoiPatch {
            rect: SlideRect::new(SlidePoint::new(100.0, 200.0), 500.0, 500.0),
            max_dimension: 1024,
        };
        let json_roi = serde_json::to_string(&cmd_roi).unwrap();
        assert!(json_roi.contains("\"type\":\"ExtractRoiPatch\""));
    }

    #[test]
    fn test_active_viewport_changed_event_serde() {
        let evt = ActiveViewportChangedEvent {
            viewport_id: "panel_wsi".to_string(),
            kind: "wsi".to_string(),
        };
        let json = serde_json::to_string(&evt).unwrap();
        let back: ActiveViewportChangedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.viewport_id, "panel_wsi");
        assert_eq!(back.kind, "wsi");
    }

    #[test]
    fn test_reader_excerpt_serde() {
        let excerpt = ReaderExcerpt {
            text: "Differential diagnosis".to_string(),
            source_url: "https://pubmed.ncbi.nlm.nih.gov/12345".to_string(),
            document_title: "Thoracic Tumours".to_string(),
            pmid_or_doi: Some("12345".to_string()),
        };
        let json = serde_json::to_string(&excerpt).unwrap();
        let back: ReaderExcerpt = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pmid_or_doi.as_deref(), Some("12345"));
    }

    #[test]
    fn test_open_file_dialog_defaults() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let wsi_res = rt.block_on(open_file_dialog("wsi".to_string())).unwrap();
        assert_eq!(wsi_res, Some("sample".to_string()));

        let dicom_res = rt.block_on(open_file_dialog("dicom".to_string())).unwrap();
        assert_eq!(dicom_res, Some("synthetic".to_string()));

        let unknown_res = rt.block_on(open_file_dialog("other".to_string())).unwrap();
        assert_eq!(unknown_res, None);
    }

    #[tokio::test]
    async fn test_handle_viewport_command_synthetic_fallbacks() {
        // Test that handle_viewport_command succeeds on RequestTile and ExtractRoiPatch
        // even with default sample slide
        let state = AppState::new();

        let tile_cmd = ViewportCommand::RequestTile {
            level: 0,
            tile_x: 0,
            tile_y: 0,
        };
        let res = handle_viewport_command_impl(tile_cmd, &state).await;
        assert!(res.is_ok());
        let val = res.unwrap();
        assert!(val.get("png_base64").is_some());

        let roi_cmd = ViewportCommand::ExtractRoiPatch {
            rect: SlideRect::new(SlidePoint::new(0.0, 0.0), 256.0, 256.0),
            max_dimension: 256,
        };
        let roi_res = handle_viewport_command_impl(roi_cmd, &state).await;
        assert!(roi_res.is_ok());
        let roi_val = roi_res.unwrap();
        assert!(roi_val.get("patch_base64").is_some());
    }

    #[tokio::test]
    async fn test_cancel_diagnostic_query_logic() {
        let state = AppState::new();

        // Cancel with non-existent id returns false
        let res1 = cancel_diagnostic_query_impl(Some("non_existent".to_string()), &state);
        assert!(!res1);

        // Prepare a real session and cancel it specifically
        let active_id = state.intelligence.prepare_session();
        let res2 = cancel_diagnostic_query_impl(Some(active_id), &state);
        assert!(res2);

        // Cancel all returns true
        let res3 = cancel_diagnostic_query_impl(None, &state);
        assert!(res3);
    }
}
