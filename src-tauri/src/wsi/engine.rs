use crate::wsi::cache::{CachedTile, TileCache, TileKey};
use crate::wsi::coordinates::{SlideRect, ViewportProjector, TILE_SIZE};
use image::{imageops::FilterType, ImageBuffer, ImageFormat, Rgba};
use openslide_rs::{Address, OpenSlide, Region, Size};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WsiError {
    #[error("OpenSlide error: {0}")]
    OpenSlide(#[from] openslide_rs::errors::OpenSlideError),

    #[error("Slide not found: {0}")]
    SlideNotFound(String),

    #[error("Invalid pyramid level {0}")]
    InvalidLevel(u32),

    #[error("Access to associated slide label/macro is prohibited under HIPAA quarantine")]
    PhiQuarantineViolation,

    #[error("Image encoding error: {0}")]
    ImageEncoding(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlideMetadata {
    pub slide_id: String,
    pub width_px: u64,
    pub height_px: u64,
    pub mpp_x: f64,
    pub mpp_y: f64,
    pub level_count: u32,
    pub level_downsamples: Vec<f64>,
}

/// Thread-safe RAII container holding a dynamically linked OpenSlide handle.
struct OpenSlideHandle {
    slide: OpenSlide,
    metadata: SlideMetadata,
}

// SAFETY: OpenSlide instances are safe to send and share across threads.
// All C calls in openslide-rs operate on re-entrant, thread-safe library internals.
unsafe impl Send for OpenSlideHandle {}
unsafe impl Sync for OpenSlideHandle {}

/// Core Whole Slide Image (WSI) Engine.
pub struct WsiEngine {
    slides: RwLock<HashMap<String, Arc<OpenSlideHandle>>>,
    tile_cache: Arc<TileCache>,
}

impl WsiEngine {
    pub fn new() -> Self {
        Self {
            slides: RwLock::new(HashMap::new()),
            tile_cache: Arc::new(TileCache::with_default_ceiling()),
        }
    }

    pub fn with_cache(cache: Arc<TileCache>) -> Self {
        Self {
            slides: RwLock::new(HashMap::new()),
            tile_cache: cache,
        }
    }

    /// Opens a WSI slide from disk via dynamic OpenSlide bindings and extracts metadata.
    pub fn open_slide(&self, file_path: &str) -> Result<SlideMetadata, WsiError> {
        self.assert_hipaa_quarantine(file_path)?;

        let path = Path::new(file_path);
        if !path.exists() {
            return Err(WsiError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Slide file not found at: {}", file_path),
            )));
        }

        // OpenSlide dynamically links against libopenslide shared library.
        // SAFETY: Path is verified and openslide_rs::OpenSlide::new initializes the C struct safely.
        let slide = OpenSlide::new(file_path)?;

        let level_count = slide.level_count();
        let (width_px, height_px) = if level_count > 0 {
            let dim0 = slide.level_dimensions(0)?;
            (dim0.w as u64, dim0.h as u64)
        } else {
            (0, 0)
        };

        let mut level_downsamples = Vec::new();
        for l in 0..level_count {
            let ds = slide.level_downsample(l)?;
            level_downsamples.push(ds);
        }

        // Extract Microns Per Pixel (MPP), falling back to 0.25 (standard 40x objective)
        let mpp_x = slide
            .property_value("openslide.mpp-x")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.25);

        let mpp_y = slide
            .property_value("openslide.mpp-y")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.25);

        let slide_id = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file_path)
            .to_string();

        let metadata = SlideMetadata {
            slide_id: slide_id.clone(),
            width_px,
            height_px,
            mpp_x,
            mpp_y,
            level_count,
            level_downsamples,
        };

        let handle = Arc::new(OpenSlideHandle {
            slide,
            metadata: metadata.clone(),
        });

        let mut map = self.slides.write().unwrap();
        map.insert(slide_id, handle);

        Ok(metadata)
    }

    /// Retrieves metadata for a previously loaded slide.
    pub fn get_metadata(&self, slide_id: &str) -> Result<SlideMetadata, WsiError> {
        self.assert_hipaa_quarantine(slide_id)?;

        let map = self.slides.read().unwrap();
        let handle = map
            .get(slide_id)
            .ok_or_else(|| WsiError::SlideNotFound(slide_id.to_string()))?;
        Ok(handle.metadata.clone())
    }

    /// Fetches a 256x256 RGBA tile, using the bounded LRU cache.
    pub fn get_tile(
        &self,
        slide_id: &str,
        level: u32,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Arc<Vec<u8>>, WsiError> {
        self.assert_hipaa_quarantine(slide_id)?;

        let key = TileKey::new(slide_id, level, tile_x, tile_y);

        // Check cache first (< 1 ms latency target)
        if let Some(cached) = self.tile_cache.get(&key) {
            return Ok(cached.rgba);
        }

        let handle = {
            let map = self.slides.read().unwrap();
            map.get(slide_id)
                .cloned()
                .ok_or_else(|| WsiError::SlideNotFound(slide_id.to_string()))?
        };

        if level >= handle.metadata.level_count {
            return Err(WsiError::InvalidLevel(level));
        }

        let downsample = handle.metadata.level_downsamples[level as usize];
        let (x0, y0) =
            ViewportProjector::tile_to_level0_coords(tile_x, tile_y, downsample, TILE_SIZE);

        let region = Region {
            address: Address { x: x0, y: y0 },
            size: Size {
                w: TILE_SIZE,
                h: TILE_SIZE,
            },
            level,
        };

        // SAFETY: read_image_rgba invokes OpenSlide C library read_region and unpacks ARGB to straight RGBA.
        let rgba_image = handle.slide.read_image_rgba(&region)?;
        let rgba_bytes = rgba_image.into_raw();

        let cached_tile = CachedTile::new(rgba_bytes, TILE_SIZE, TILE_SIZE);
        let bytes_arc = cached_tile.rgba.clone();
        self.tile_cache.insert(key, cached_tile);

        Ok(bytes_arc)
    }

    /// Fetches a tile and encodes it to PNG format for HTTP/custom protocol transfer.
    pub fn get_tile_png(
        &self,
        slide_id: &str,
        level: u32,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<Vec<u8>, WsiError> {
        self.assert_hipaa_quarantine(slide_id)?;
        let rgba = self.get_tile(slide_id, level, tile_x, tile_y)?;
        encode_rgba_to_png(&rgba, TILE_SIZE, TILE_SIZE)
    }

    /// Extracts an ROI patch anchored in slide-space microns, downsampling to <= 1024x1024 RGBA PNG.
    /// In-memory HIPAA Quarantine: never reads or exposes slide macro, label, or barcode.
    pub fn extract_roi_patch(
        &self,
        slide_id: &str,
        rect: SlideRect,
        max_dimension_px: u32,
    ) -> Result<Vec<u8>, WsiError> {
        self.assert_hipaa_quarantine(slide_id)?;

        let max_dim = max_dimension_px.min(1024).max(1);

        let handle = {
            let map = self.slides.read().unwrap();
            map.get(slide_id)
                .cloned()
                .ok_or_else(|| WsiError::SlideNotFound(slide_id.to_string()))?
        };

        let (x0, y0, w0, h0) =
            rect.to_level0_pixels(handle.metadata.mpp_x, handle.metadata.mpp_y);

        // Clamp ROI to slide bounds
        let clamped_x0 = x0.min(handle.metadata.width_px);
        let clamped_y0 = y0.min(handle.metadata.height_px);
        let clamped_w0 = w0.min(handle.metadata.width_px.saturating_sub(clamped_x0));
        let clamped_h0 = h0.min(handle.metadata.height_px.saturating_sub(clamped_y0));

        if clamped_w0 == 0 || clamped_h0 == 0 {
            return Err(WsiError::ImageEncoding(
                "ROI patch has zero dimension within slide boundary".to_string(),
            ));
        }

        // Determine optimal pyramid level and downsampling
        let max_req_dim = clamped_w0.max(clamped_h0) as f64;
        let scale_needed = (max_req_dim / max_dim as f64).max(1.0);

        let best_level = ViewportProjector::best_level_for_zoom(
            1.0 / scale_needed,
            &handle.metadata.level_downsamples,
        );
        let level_ds = handle.metadata.level_downsamples[best_level as usize];

        let read_w = ((clamped_w0 as f64) / level_ds).round().max(1.0) as u32;
        let read_h = ((clamped_h0 as f64) / level_ds).round().max(1.0) as u32;

        let region = Region {
            address: Address {
                x: clamped_x0 as u32,
                y: clamped_y0 as u32,
            },
            size: Size {
                w: read_w,
                h: read_h,
            },
            level: best_level,
        };

        let rgba_image = handle.slide.read_image_rgba(&region)?;

        // If dimensions still exceed max_dim (due to discrete level steps), downsample in memory
        let final_image = if rgba_image.width() > max_dim || rgba_image.height() > max_dim {
            let target_w = if rgba_image.width() >= rgba_image.height() {
                max_dim
            } else {
                ((rgba_image.width() as f64 * max_dim as f64) / rgba_image.height() as f64).round()
                    as u32
            };
            let target_h = if rgba_image.height() >= rgba_image.width() {
                max_dim
            } else {
                ((rgba_image.height() as f64 * max_dim as f64) / rgba_image.width() as f64).round()
                    as u32
            };
            image::imageops::resize(
                &rgba_image,
                target_w.max(1),
                target_h.max(1),
                FilterType::Lanczos3,
            )
        } else {
            rgba_image
        };

        encode_rgba_to_png(
            final_image.as_raw(),
            final_image.width(),
            final_image.height(),
        )
    }

    /// Rejects requests for slide label or macro overview to preserve HIPAA boundary.
    pub fn assert_hipaa_quarantine(&self, requested_resource: &str) -> Result<(), WsiError> {
        let lower = requested_resource.to_ascii_lowercase();
        if lower.contains("label")
            || lower.contains("macro")
            || lower.contains("barcode")
            || lower.contains("overview")
        {
            return Err(WsiError::PhiQuarantineViolation);
        }
        Ok(())
    }
}

impl Default for WsiEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to encode raw RGBA pixels into PNG format.
pub fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, WsiError> {
    let img: ImageBuffer<Rgba<u8>, &[u8]> =
        ImageBuffer::from_raw(width, height, rgba).ok_or_else(|| {
            WsiError::ImageEncoding(format!(
                "Buffer size mismatch for {}x{} RGBA image",
                width, height
            ))
        })?;

    let mut cursor = Cursor::new(Vec::new());
    img.write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| WsiError::ImageEncoding(e.to_string()))?;

    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsi::coordinates::SlidePoint;

    #[test]
    fn test_encode_rgba_to_png() {
        let width = 4;
        let height = 4;
        let rgba = vec![255u8; (width * height * 4) as usize];
        let png = encode_rgba_to_png(&rgba, width, height).expect("PNG encode should succeed");
        assert!(!png.is_empty());
        // Verify PNG magic header: \x89PNG\r\n\x1a\n
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn test_hipaa_quarantine_enforcement() {
        let engine = WsiEngine::new();
        assert!(engine.assert_hipaa_quarantine("label_image.png").is_err());
        assert!(engine.assert_hipaa_quarantine("macro_photo.jpg").is_err());
        assert!(engine.assert_hipaa_quarantine("barcode").is_err());
        assert!(engine.assert_hipaa_quarantine("overview_map").is_err());
        assert!(engine.assert_hipaa_quarantine("tile_0_1_2.png").is_ok());
        assert!(engine.assert_hipaa_quarantine("roi_patch_1024").is_ok());

        // Verify that engine methods enforce quarantine before I/O or state lookup
        assert!(matches!(
            engine.open_slide("/path/to/slide_label.svs"),
            Err(WsiError::PhiQuarantineViolation)
        ));
        assert!(matches!(
            engine.get_metadata("slide_macro"),
            Err(WsiError::PhiQuarantineViolation)
        ));
        assert!(matches!(
            engine.get_tile("macro_slide", 0, 0, 0),
            Err(WsiError::PhiQuarantineViolation)
        ));
        assert!(matches!(
            engine.get_tile_png("slide_label", 0, 0, 0),
            Err(WsiError::PhiQuarantineViolation)
        ));
        assert!(matches!(
            engine.extract_roi_patch("overview_camera", SlideRect::new(SlidePoint::new(0.0, 0.0), 100.0, 100.0), 1024),
            Err(WsiError::PhiQuarantineViolation)
        ));
    }

    #[test]
    fn test_patch_downsampling_math() {
        // A rect corresponding to 4000x2000 level0 pixels
        let mpp = 0.25;
        let rect = SlideRect::new(
            SlidePoint::new(0.0, 0.0),
            4000.0 * mpp, // 1000 um -> 4000 px
            2000.0 * mpp, // 500 um -> 2000 px
        );
        let (x0, y0, w0, h0) = rect.to_level0_pixels(mpp, mpp);
        assert_eq!(x0, 0);
        assert_eq!(y0, 0);
        assert_eq!(w0, 4000);
        assert_eq!(h0, 2000);

        let max_dim = 1024u32;
        let scale_needed = (w0.max(h0) as f64 / max_dim as f64).max(1.0);
        assert!((scale_needed - (4000.0 / 1024.0)).abs() < 1e-6);
    }
}
