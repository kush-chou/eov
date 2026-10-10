use crate::dicom::volume::{DicomVolume, MprPlane};
use crate::hud::DiagnosticQueryRequest;
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

/// ReaderExcerpt contract matching SPEC-04-READER.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaderExcerpt {
    pub text: String,
    pub source_url: String,
    pub document_title: String,
    pub pmid_or_doi: Option<String>,
}

#[tauri::command]
pub async fn open_file_dialog(_filter_type: String) -> Result<Option<String>, String> {
    // In Phase 1 desktop client, provides file/folder dialog contract per SPEC-01-SHELL
    Ok(None)
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
) -> Result<Option<ReaderExcerpt>, String> {
    Ok(None)
}

#[tauri::command]
pub async fn reader_navigate(
    window: tauri::WebviewWindow,
    url: String,
) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("file://") && !url.starts_with("lamina-") {
        return Err("Navigation restricted to secure HTTPS or local application assets".to_string());
    }
    let script = format!("window.location.href = '{}';", url.replace('\'', "\\'"));
    window.eval(&script).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn handle_viewport_command(
    command: ViewportCommand,
    state: tauri::State<'_, AppState>,
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
            let png_bytes = state.wsi.get_tile_png("sample", level, tile_x, tile_y).map_err(|e| e.to_string())?;
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
                engine.extract_roi_patch("sample", rect, max_dimension).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())??;
            let b64 = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &bytes);
            Ok(serde_json::json!({ "patch_base64": b64 }))
        }
    }
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
    let w = width.unwrap_or(128);
    let h = height.unwrap_or(128);
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
    _state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    // Optical Allow-List check before anything is dispatched
    request
        .validate_optical_allow_list()
        .map_err(|e| e.to_string())?;

    // In Phase 1 Core, return an acknowledgment with session interaction id
    Ok("interaction_phase1_ack".to_string())
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
}
