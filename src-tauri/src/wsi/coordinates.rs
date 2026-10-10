use serde::{Deserialize, Serialize};

/// Standard pyramid tile size in pixels (256x256 RGBA).
pub const TILE_SIZE: u32 = 256;

/// Slide-space coordinate anchored in physical microns (µm).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SlidePoint {
    pub x_um: f64,
    pub y_um: f64,
}

impl SlidePoint {
    pub fn new(x_um: f64, y_um: f64) -> Self {
        Self { x_um, y_um }
    }
}

/// Slide-space bounding box anchored in physical microns (µm).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SlideRect {
    pub origin: SlidePoint,
    pub width_um: f64,
    pub height_um: f64,
}

impl SlideRect {
    pub fn new(origin: SlidePoint, width_um: f64, height_um: f64) -> Self {
        Self {
            origin,
            width_um,
            height_um,
        }
    }

    pub fn right_um(&self) -> f64 {
        self.origin.x_um + self.width_um
    }

    pub fn bottom_um(&self) -> f64 {
        self.origin.y_um + self.height_um
    }

    /// Converts physical slide rect to Level 0 pixel coordinates.
    pub fn to_level0_pixels(&self, mpp_x: f64, mpp_y: f64) -> (u64, u64, u64, u64) {
        let safe_mpp_x = if mpp_x > 0.0 { mpp_x } else { 0.25 };
        let safe_mpp_y = if mpp_y > 0.0 { mpp_y } else { 0.25 };
        let x0 = (self.origin.x_um / safe_mpp_x).max(0.0).round() as u64;
        let y0 = (self.origin.y_um / safe_mpp_y).max(0.0).round() as u64;
        let w0 = (self.width_um / safe_mpp_x).max(1.0).round() as u64;
        let h0 = (self.height_um / safe_mpp_y).max(1.0).round() as u64;
        (x0, y0, w0, h0)
    }
}

/// Viewport projection mapping between physical microns and screen pixel space.
pub struct ViewportProjector;

impl ViewportProjector {
    /// Maps physical slide coordinate (µm) to screen pixel coordinates:
    /// P_screen = (P_slide - O_viewport) * (Zoom / MPP)
    pub fn slide_to_screen(
        point: SlidePoint,
        viewport_origin: SlidePoint,
        zoom: f64,
        mpp_x: f64,
        mpp_y: f64,
    ) -> (f64, f64) {
        let safe_mpp_x = if mpp_x > 0.0 { mpp_x } else { 0.25 };
        let safe_mpp_y = if mpp_y > 0.0 { mpp_y } else { 0.25 };
        let safe_zoom = if zoom > 0.0 { zoom } else { 1.0 };
        let sx = (point.x_um - viewport_origin.x_um) * (safe_zoom / safe_mpp_x);
        let sy = (point.y_um - viewport_origin.y_um) * (safe_zoom / safe_mpp_y);
        (sx, sy)
    }

    /// Maps screen pixel coordinate back to physical slide coordinates (µm):
    /// P_slide = O_viewport + (P_screen * MPP / Zoom)
    pub fn screen_to_slide(
        screen_x: f64,
        screen_y: f64,
        viewport_origin: SlidePoint,
        zoom: f64,
        mpp_x: f64,
        mpp_y: f64,
    ) -> SlidePoint {
        let safe_mpp_x = if mpp_x > 0.0 { mpp_x } else { 0.25 };
        let safe_mpp_y = if mpp_y > 0.0 { mpp_y } else { 0.25 };
        let safe_zoom = if zoom > 0.0 { zoom } else { 1.0 };
        let x_um = viewport_origin.x_um + (screen_x * safe_mpp_x / safe_zoom);
        let y_um = viewport_origin.y_um + (screen_y * safe_mpp_y / safe_zoom);
        SlidePoint::new(x_um, y_um)
    }

    /// Maps Level 0 pixel coordinate to physical microns (µm).
    pub fn level0_to_slide(px: f64, py: f64, mpp_x: f64, mpp_y: f64) -> SlidePoint {
        let safe_mpp_x = if mpp_x > 0.0 { mpp_x } else { 0.25 };
        let safe_mpp_y = if mpp_y > 0.0 { mpp_y } else { 0.25 };
        SlidePoint::new(px * safe_mpp_x, py * safe_mpp_y)
    }

    /// Maps physical microns (µm) to Level 0 pixel coordinates.
    pub fn slide_to_level0(point: SlidePoint, mpp_x: f64, mpp_y: f64) -> (f64, f64) {
        let safe_mpp_x = if mpp_x > 0.0 { mpp_x } else { 0.25 };
        let safe_mpp_y = if mpp_y > 0.0 { mpp_y } else { 0.25 };
        (point.x_um / safe_mpp_x, point.y_um / safe_mpp_y)
    }

    /// Computes OpenSlide `read_region` Level 0 origin coordinates for a given tile:
    /// x0 = tile_x * tile_size * downsample(level)
    /// y0 = tile_y * tile_size * downsample(level)
    pub fn tile_to_level0_coords(
        tile_x: u32,
        tile_y: u32,
        downsample: f64,
        tile_size: u32,
    ) -> (u32, u32) {
        let x0 = (tile_x as f64 * tile_size as f64 * downsample).round() as u32;
        let y0 = (tile_y as f64 * tile_size as f64 * downsample).round() as u32;
        (x0, y0)
    }

    /// Selects the optimal pyramid level for a requested zoom factor.
    /// Zoom 1.0 -> Level 0 (downsample 1.0).
    /// Zoom 0.5 -> Level 1 (downsample 2.0).
    pub fn best_level_for_zoom(zoom: f64, downsamples: &[f64]) -> u32 {
        if downsamples.is_empty() {
            return 0;
        }
        let target_downsample = (1.0 / zoom).max(1.0);
        let mut best_level = 0;
        let mut min_diff = f64::MAX;

        for (level, &ds) in downsamples.iter().enumerate() {
            // Prefer levels that have downsample <= target_downsample to avoid downsampling blur,
            // or find closest.
            let diff = (ds - target_downsample).abs();
            if diff < min_diff {
                min_diff = diff;
                best_level = level as u32;
            }
        }
        best_level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slide_point_and_rect() {
        let pt = SlidePoint::new(100.5, 200.25);
        assert_eq!(pt.x_um, 100.5);
        assert_eq!(pt.y_um, 200.25);

        let rect = SlideRect::new(pt, 500.0, 400.0);
        assert_eq!(rect.right_um(), 600.5);
        assert_eq!(rect.bottom_um(), 600.25);
    }

    #[test]
    fn test_micron_level0_conversion() {
        let mpp = 0.25; // 0.25 um/px -> 40x objective
        let pt = SlidePoint::new(250.0, 500.0);
        let (px, py) = ViewportProjector::slide_to_level0(pt, mpp, mpp);
        assert_eq!(px, 1000.0);
        assert_eq!(py, 2000.0);

        let back_pt = ViewportProjector::level0_to_slide(px, py, mpp, mpp);
        assert_eq!(back_pt.x_um, 250.0);
        assert_eq!(back_pt.y_um, 500.0);
    }

    #[test]
    fn test_projection_roundtrip_stability_across_zooms() {
        let mpp_x = 0.252;
        let mpp_y = 0.252;
        let origin = SlidePoint::new(1500.0, 2500.0);
        let test_pt = SlidePoint::new(3200.5, 4800.75);

        let zoom_levels = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 40.0, 80.0, 100.0];

        for &zoom in &zoom_levels {
            let (sx, sy) = ViewportProjector::slide_to_screen(test_pt, origin, zoom, mpp_x, mpp_y);
            let back = ViewportProjector::screen_to_slide(sx, sy, origin, zoom, mpp_x, mpp_y);

            assert!(
                (back.x_um - test_pt.x_um).abs() < 1e-9,
                "X drift at zoom {}: {} vs {}",
                zoom,
                back.x_um,
                test_pt.x_um
            );
            assert!(
                (back.y_um - test_pt.y_um).abs() < 1e-9,
                "Y drift at zoom {}: {} vs {}",
                zoom,
                back.y_um,
                test_pt.y_um
            );
        }
    }

    #[test]
    fn test_tile_to_level0_downsample_math() {
        let tile_size = 256;
        // Level 0: downsample 1.0
        let (x0, y0) = ViewportProjector::tile_to_level0_coords(2, 3, 1.0, tile_size);
        assert_eq!(x0, 512);
        assert_eq!(y0, 768);

        // Level 2: downsample 4.0
        let (x0_l2, y0_l2) = ViewportProjector::tile_to_level0_coords(2, 3, 4.0, tile_size);
        assert_eq!(x0_l2, 2 * 256 * 4);
        assert_eq!(y0_l2, 3 * 256 * 4);

        // Level 4: downsample 16.0
        let (x0_l4, y0_l4) = ViewportProjector::tile_to_level0_coords(5, 7, 16.0, tile_size);
        assert_eq!(x0_l4, 5 * 256 * 16);
        assert_eq!(y0_l4, 7 * 256 * 16);
    }

    #[test]
    fn test_best_level_for_zoom() {
        let downsamples = vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
        assert_eq!(ViewportProjector::best_level_for_zoom(1.0, &downsamples), 0);
        assert_eq!(ViewportProjector::best_level_for_zoom(0.5, &downsamples), 1);
        assert_eq!(ViewportProjector::best_level_for_zoom(0.25, &downsamples), 2);
        assert_eq!(ViewportProjector::best_level_for_zoom(0.125, &downsamples), 3);
        assert_eq!(ViewportProjector::best_level_for_zoom(0.0625, &downsamples), 4);

        // Edge case: empty downsamples
        assert_eq!(ViewportProjector::best_level_for_zoom(1.0, &[]), 0);
        // Edge case: single downsample
        assert_eq!(ViewportProjector::best_level_for_zoom(0.1, &[1.0]), 0);
    }

    #[test]
    fn test_anisotropic_mpp_projection() {
        let mpp_x = 0.25;
        let mpp_y = 0.50; // Non-square pixels (e.g. specialized line scan sensors)
        let origin = SlidePoint::new(100.0, 200.0);
        let pt = SlidePoint::new(350.0, 700.0);
        let zoom = 2.0;

        let (sx, sy) = ViewportProjector::slide_to_screen(pt, origin, zoom, mpp_x, mpp_y);
        // sx = (350 - 100) * (2.0 / 0.25) = 250 * 8 = 2000.0
        // sy = (700 - 200) * (2.0 / 0.50) = 500 * 4 = 2000.0
        assert_eq!(sx, 2000.0);
        assert_eq!(sy, 2000.0);

        let roundtrip = ViewportProjector::screen_to_slide(sx, sy, origin, zoom, mpp_x, mpp_y);
        assert!((roundtrip.x_um - pt.x_um).abs() < 1e-9);
        assert!((roundtrip.y_um - pt.y_um).abs() < 1e-9);
    }

    #[test]
    fn test_zero_and_negative_bounds_handling() {
        let rect = SlideRect::new(SlidePoint::new(-50.0, -100.0), 0.0, 0.0);
        let (x0, y0, w0, h0) = rect.to_level0_pixels(0.25, 0.25);
        // Coordinates should clamp to 0 and width/height clamp to at least 1 pixel
        assert_eq!(x0, 0);
        assert_eq!(y0, 0);
        assert_eq!(w0, 1);
        assert_eq!(h0, 1);
    }

    #[test]
    fn test_zero_and_negative_zoom_and_mpp() {
        let pt = SlidePoint::new(100.0, 100.0);
        let origin = SlidePoint::new(0.0, 0.0);

        // Zero zoom & MPP should fall back safely without NaN or Inf
        let (sx, sy) = ViewportProjector::slide_to_screen(pt, origin, 0.0, 0.0, 0.0);
        assert!(!sx.is_nan() && !sx.is_infinite());
        assert!(!sy.is_nan() && !sy.is_infinite());

        let back = ViewportProjector::screen_to_slide(sx, sy, origin, 0.0, 0.0, 0.0);
        assert!(!back.x_um.is_nan() && !back.x_um.is_infinite());
        assert!(!back.y_um.is_nan() && !back.y_um.is_infinite());
    }
}
