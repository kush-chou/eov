use crate::dicom::volume::DicomVolume;
use crate::hud::HudState;
use crate::wsi::engine::WsiEngine;
use std::sync::{Arc, RwLock};

/// Root Application State managed by Tauri.
pub struct AppState {
    pub wsi: Arc<WsiEngine>,
    pub dicom: Arc<RwLock<Option<DicomVolume>>>,
    pub hud: Arc<HudState>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            wsi: Arc::new(WsiEngine::new()),
            dicom: Arc::new(RwLock::new(None)),
            hud: Arc::new(HudState::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
