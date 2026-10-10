use crate::privacy::sanitizer::{OpticalAllowList, PrivacyError};
use crate::wsi::coordinates::SlideRect;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Diagnostic Query payload sent to cloud CDS gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticQueryRequest {
    pub query_text: String,
    pub patch_png_base64: Option<String>,
    pub magnification_mpp: Option<f64>,
    pub previous_interaction_id: Option<String>,
}

impl DiagnosticQueryRequest {
    /// Validates the query request against the in-memory HIPAA optical allow-list.
    /// Strictly rejects malformed base64, corrupt images, and patches exceeding 1024x1024.
    pub fn validate_optical_allow_list(&self) -> Result<(), PrivacyError> {
        if let Some(ref b64) = self.patch_png_base64 {
            let bytes = base64::Engine::decode(&base64::prelude::BASE64_STANDARD, b64)
                .map_err(|e| PrivacyError::InvalidPatch(format!("Base64 decode failure: {}", e)))?;
            let img = image::load_from_memory(&bytes)
                .map_err(|e| PrivacyError::InvalidPatch(format!("Image decode failure: {}", e)))?;
            OpticalAllowList::validate_patch_dimensions(img.width(), img.height())?;
        }
        Ok(())
    }
}

/// Diagnostic Query streaming event relayed to Webview UI matching SPEC-00-MASTER.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum DiagnosticQueryEvent {
    TokenDelta { text: String },
    VerificationScore { confidence: f32, citation_verified: bool },
    Completed { interaction_id: String },
    Error { code: String, message: String },
}

/// Streaming SSE event received from cloud CDS gateway matching SPEC-05-INTELLIGENCE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum DiagnosticStreamEvent {
    #[serde(rename = "token")]
    TokenDelta { text: String },

    #[serde(rename = "tool_call")]
    ToolCallStarted { tool_name: String, query: String },

    #[serde(rename = "citation")]
    CitationDiscovered {
        index: u32,
        title: String,
        pmid: String,
        url: String,
    },

    #[serde(rename = "verification")]
    VerificationScore { confidence: f32, verified: bool },

    #[serde(rename = "done")]
    Completed { interaction_id: String },

    #[serde(rename = "error")]
    Failed { code: String, message: String },
}

/// HUD state tracked by the workstation core runtime.
#[derive(Debug, Default)]
pub struct HudState {
    pub visible: Mutex<bool>,
    pub active_viewport: Mutex<String>,
    pub active_roi: Mutex<Option<SlideRect>>,
}

impl HudState {
    pub fn new() -> Self {
        Self {
            visible: Mutex::new(false),
            active_viewport: Mutex::new("wsi".to_string()),
            active_roi: Mutex::new(None),
        }
    }

    pub fn toggle_visibility(&self) -> bool {
        let mut guard = self.visible.lock().unwrap();
        *guard = !*guard;
        *guard
    }

    pub fn is_visible(&self) -> bool {
        *self.visible.lock().unwrap()
    }

    pub fn set_active_viewport(&self, viewport_id: String) {
        let mut guard = self.active_viewport.lock().unwrap();
        *guard = viewport_id;
    }

    pub fn get_active_viewport(&self) -> String {
        self.active_viewport.lock().unwrap().clone()
    }

    pub fn set_active_roi(&self, roi: Option<SlideRect>) {
        let mut guard = self.active_roi.lock().unwrap();
        *guard = roi;
    }

    pub fn get_active_roi(&self) -> Option<SlideRect> {
        *self.active_roi.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsi::engine::encode_rgba_to_png;

    #[test]
    fn test_hud_state_toggle() {
        let hud = HudState::new();
        assert!(!hud.is_visible());
        assert_eq!(hud.get_active_viewport(), "wsi");
        assert!(hud.toggle_visibility());
        assert!(hud.is_visible());
        assert!(!hud.toggle_visibility());
        assert!(!hud.is_visible());

        hud.set_active_viewport("dicom".to_string());
        assert_eq!(hud.get_active_viewport(), "dicom");
    }

    #[test]
    fn test_diagnostic_query_request_validation() {
        // 1. None patch -> Ok
        let req = DiagnosticQueryRequest {
            query_text: "Analyze tissue architecture".to_string(),
            patch_png_base64: None,
            magnification_mpp: Some(0.25),
            previous_interaction_id: None,
        };
        assert!(req.validate_optical_allow_list().is_ok());

        // 2. Valid 128x128 PNG patch -> Ok
        let rgba = vec![200u8; 128 * 128 * 4];
        let png = encode_rgba_to_png(&rgba, 128, 128).unwrap();
        let b64 = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &png);
        let req_valid = DiagnosticQueryRequest {
            query_text: "Analyze tissue architecture".to_string(),
            patch_png_base64: Some(b64),
            magnification_mpp: Some(0.25),
            previous_interaction_id: None,
        };
        assert!(req_valid.validate_optical_allow_list().is_ok());

        // 3. Oversized 1025x512 PNG patch -> Err(PatchExceedsDimensionLimit)
        let rgba_large = vec![200u8; 1025 * 2 * 4];
        let png_large = encode_rgba_to_png(&rgba_large, 1025, 2).unwrap();
        let b64_large = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &png_large);
        let req_large = DiagnosticQueryRequest {
            query_text: "Analyze tissue architecture".to_string(),
            patch_png_base64: Some(b64_large),
            magnification_mpp: Some(0.25),
            previous_interaction_id: None,
        };
        assert!(matches!(
            req_large.validate_optical_allow_list(),
            Err(PrivacyError::PatchExceedsDimensionLimit(1025, 2))
        ));

        // 4. Invalid base64 -> Err(InvalidPatch)
        let req_bad_b64 = DiagnosticQueryRequest {
            query_text: "Analyze tissue architecture".to_string(),
            patch_png_base64: Some("not-valid-base64!!!".to_string()),
            magnification_mpp: None,
            previous_interaction_id: None,
        };
        assert!(matches!(
            req_bad_b64.validate_optical_allow_list(),
            Err(PrivacyError::InvalidPatch(_))
        ));

        // 5. Valid base64 but corrupt image -> Err(InvalidPatch)
        let corrupt_b64 = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, b"not a png image");
        let req_corrupt = DiagnosticQueryRequest {
            query_text: "Analyze tissue architecture".to_string(),
            patch_png_base64: Some(corrupt_b64),
            magnification_mpp: None,
            previous_interaction_id: None,
        };
        assert!(matches!(
            req_corrupt.validate_optical_allow_list(),
            Err(PrivacyError::InvalidPatch(_))
        ));
    }
}
