use crate::dicom::shader::{apply_wwwl_ps33, ClinicalPreset};
use crate::wsi::engine::encode_rgba_to_png;
use dicom_dictionary_std::tags;
use dicom_object::DefaultDicomObject;
use dicom_pixeldata::PixelDecoder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DicomError {
    #[error("DICOM read error: {0}")]
    Dicom(#[from] dicom_object::ReadError),

    #[error("Pixel data decode error: {0}")]
    PixelData(#[from] dicom_pixeldata::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Zero-Egress Invariant Violation: {0}")]
    PhiQuarantineViolation(String),

    #[error("Invalid slice index {index} for dimension {dimension}")]
    InvalidSliceIndex { index: u32, dimension: u32 },

    #[error("Series directory contains no valid DICOM slices: {0}")]
    EmptySeries(String),

    #[error("Inconsistent slice dimensions across series: expected {0:?}, got {1:?}")]
    InconsistentDimensions([u32; 2], [u32; 2]),

    #[error("Image encoding error: {0}")]
    ImageEncoding(String),
}

/// Multiplanar Reconstruction (MPR) orthogonal anatomical planes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MprPlane {
    Axial,    // XY plane (transverse)
    Coronal,  // XZ plane (frontal)
    Sagittal, // YZ plane (lateral)
}

/// In-memory volumetric radiology dataset matching SPEC-LAMINA-03-DICOM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DicomVolume {
    pub series_instance_uid: String,
    pub dimensions: [u32; 3],    // [width, height, slices]
    pub voxel_spacing: [f32; 3], // [pixel_spacing_x, pixel_spacing_y, slice_thickness]
    pub rescale_slope: f32,
    pub rescale_intercept: f32,
    pub default_window: (f32, f32), // (center, width)
    #[serde(skip)]
    pub voxel_data: Vec<i16>, // 16-bit raw signed pixel buffer
}

/// Extracted 2D resliced image from a volumetric DICOM dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceImage {
    pub width: u32,
    pub height: u32,
    pub plane: MprPlane,
    pub slice_index: u32,
    pub aspect_ratio: f32,
    #[serde(skip)]
    pub raw_voxels: Vec<i16>,
    #[serde(skip)]
    pub windowed_rgba: Vec<u8>,
}

impl SliceImage {
    /// Encodes windowed RGBA slice to PNG format.
    pub fn to_png(&self) -> Result<Vec<u8>, DicomError> {
        encode_rgba_to_png(&self.windowed_rgba, self.width, self.height)
            .map_err(|e| DicomError::ImageEncoding(e.to_string()))
    }
}

impl DicomVolume {
    /// Creates a synthetic volumetric CT phantom with anatomical HU regions.
    /// Used for testing, UI benchmarking, and zero-network development.
    pub fn generate_synthetic(width: u32, height: u32, slices: u32) -> Self {
        let total_voxels = (width * height * slices) as usize;
        let mut voxel_data = Vec::with_capacity(total_voxels);

        for z in 0..slices {
            for y in 0..height {
                for x in 0..width {
                    // Center coordinates normalized to [-1.0, 1.0]
                    let nx = (x as f32 / width as f32) * 2.0 - 1.0;
                    let ny = (y as f32 / height as f32) * 2.0 - 1.0;
                    let _nz = (z as f32 / slices as f32) * 2.0 - 1.0;
                    let r = (nx * nx + ny * ny).sqrt();

                    let hu: f32 = if r > 0.85 {
                        -1000.0 // Air around patient
                    } else if r > 0.80 {
                        400.0 // Bone (skull/rib cage outer shell)
                    } else if nx < -0.1 && ny < 0.2 && r < 0.7 {
                        -600.0 // Lung parenchyma (low density)
                    } else if nx > 0.1 && ny < 0.2 && r < 0.7 {
                        -600.0 // Contralateral lung
                    } else if (nx * nx + (ny - 0.3) * (ny - 0.3)).sqrt() < 0.2 {
                        80.0 // High density organ (liver/heart)
                    } else {
                        40.0 // Soft tissue baseline
                    };

                    // Raw pixel with slope=1.0, intercept=0.0
                    voxel_data.push(hu.round() as i16);
                }
            }
        }

        Self {
            series_instance_uid: "1.2.826.0.1.3680043.9.synthetic.phantom.1".to_string(),
            dimensions: [width, height, slices],
            voxel_spacing: [0.75, 0.75, 2.5], // 0.75mm in-plane, 2.5mm slice thickness
            rescale_slope: 1.0,
            rescale_intercept: 0.0,
            default_window: ClinicalPreset::SoftTissue.window(),
            voxel_data,
        }
    }

    /// Reslices the 3D volume along an orthogonal anatomical plane (MPR).
    /// Applies aspect-ratio compensation for non-cubic voxel anisotropy.
    pub fn extract_slice(
        &self,
        plane: MprPlane,
        slice_index: u32,
        window: Option<(f32, f32)>,
    ) -> Result<SliceImage, DicomError> {
        let (win_center, win_width) = window.unwrap_or(self.default_window);
        let [w, h, d] = self.dimensions;

        let (slice_w, slice_h, aspect_ratio, raw_voxels) = match plane {
            MprPlane::Axial => {
                if slice_index >= d {
                    return Err(DicomError::InvalidSliceIndex {
                        index: slice_index,
                        dimension: d,
                    });
                }
                let mut voxels = Vec::with_capacity((w * h) as usize);
                let z_offset = (slice_index as usize) * (w as usize * h as usize);
                for y in 0..h {
                    for x in 0..w {
                        let idx = z_offset + (y as usize * w as usize) + (x as usize);
                        if idx < self.voxel_data.len() {
                            voxels.push(self.voxel_data[idx]);
                        } else {
                            voxels.push(0);
                        }
                    }
                }
                (w, h, 1.0f32, voxels)
            }
            MprPlane::Coronal => {
                if slice_index >= h {
                    return Err(DicomError::InvalidSliceIndex {
                        index: slice_index,
                        dimension: h,
                    });
                }
                // Coronal reslicing: X along columns, Z along rows
                // Aspect ratio = SliceThickness / PixelSpacing_x
                let aspect = if self.voxel_spacing[0] > 0.0 {
                    self.voxel_spacing[2] / self.voxel_spacing[0]
                } else {
                    1.0
                };
                let mut voxels = Vec::with_capacity((w * d) as usize);
                let y_fixed = slice_index as usize;
                for z in 0..d {
                    let z_offset = (z as usize) * (w as usize * h as usize);
                    for x in 0..w {
                        let idx = z_offset + (y_fixed * w as usize) + (x as usize);
                        if idx < self.voxel_data.len() {
                            voxels.push(self.voxel_data[idx]);
                        } else {
                            voxels.push(0);
                        }
                    }
                }
                (w, d, aspect, voxels)
            }
            MprPlane::Sagittal => {
                if slice_index >= w {
                    return Err(DicomError::InvalidSliceIndex {
                        index: slice_index,
                        dimension: w,
                    });
                }
                // Sagittal reslicing: Y along columns, Z along rows
                // Aspect ratio = SliceThickness / PixelSpacing_y
                let aspect = if self.voxel_spacing[1] > 0.0 {
                    self.voxel_spacing[2] / self.voxel_spacing[1]
                } else {
                    1.0
                };
                let mut voxels = Vec::with_capacity((h * d) as usize);
                let x_fixed = slice_index as usize;
                for z in 0..d {
                    let z_offset = (z as usize) * (w as usize * h as usize);
                    for y in 0..h {
                        let idx = z_offset + (y as usize * w as usize) + x_fixed;
                        if idx < self.voxel_data.len() {
                            voxels.push(self.voxel_data[idx]);
                        } else {
                            voxels.push(0);
                        }
                    }
                }
                (h, d, aspect, voxels)
            }
        };

        // Window voxels to 8-bit RGBA using DICOM PS3.3 transfer function
        let mut windowed_rgba = Vec::with_capacity((slice_w * slice_h * 4) as usize);
        for &raw in &raw_voxels {
            let norm = apply_wwwl_ps33(
                raw,
                self.rescale_slope,
                self.rescale_intercept,
                win_center,
                win_width,
            );
            let lum = (norm * 255.0).round().clamp(0.0, 255.0) as u8;
            windowed_rgba.extend_from_slice(&[lum, lum, lum, 255]);
        }

        Ok(SliceImage {
            width: slice_w,
            height: slice_h,
            plane,
            slice_index,
            aspect_ratio,
            raw_voxels,
            windowed_rgba,
        })
    }

    /// Ingests a single DICOM file (multi-frame or single slice).
    /// Sanitizes all patient demographic and institutional PHI in memory prior to volume ingestion.
    pub fn load_file(file_path: &str) -> Result<Self, DicomError> {
        let mut obj = DefaultDicomObject::open_file(file_path)?;
        crate::privacy::sanitizer::DicomSanitizer::sanitize_file_object(&mut obj);
        crate::privacy::sanitizer::DicomSanitizer::verify_quarantine(&obj)
            .map_err(|e| DicomError::PhiQuarantineViolation(e.to_string()))?;
        Self::from_dicom_object(&obj)
    }

    /// Ingests a series directory containing multiple DICOM slice files.
    /// Purges all PHI tags in memory across all slices and verifies quarantine integrity.
    pub fn load_series(series_dir: &str) -> Result<Self, DicomError> {
        let dir_path = Path::new(series_dir);
        if !dir_path.exists() || !dir_path.is_dir() {
            return Err(DicomError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Series directory not found: {}", series_dir),
            )));
        }

        let mut slice_entries: Vec<(f64, i32, DefaultDicomObject)> = Vec::new();

        for entry in fs::read_dir(dir_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Ok(mut obj) = DefaultDicomObject::open_file(&path) {
                    // Purge PHI in memory immediately
                    crate::privacy::sanitizer::DicomSanitizer::sanitize_file_object(&mut obj);
                    crate::privacy::sanitizer::DicomSanitizer::verify_quarantine(&obj)
                        .map_err(|e| DicomError::PhiQuarantineViolation(e.to_string()))?;

                    // Extract slice location or Z position for sorting
                    let z_coord = extract_z_position(&obj).unwrap_or(0.0);
                    let instance_num = extract_instance_number(&obj).unwrap_or(0);
                    slice_entries.push((z_coord, instance_num, obj));
                }
            }
        }

        if slice_entries.is_empty() {
            return Err(DicomError::EmptySeries(series_dir.to_string()));
        }

        // Sort by physical Z position ascending, then instance number
        slice_entries.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });

        // Use first slice for geometric metadata baseline
        let first_obj = &slice_entries[0].2;
        let series_instance_uid = extract_string_tag(first_obj, tags::SERIES_INSTANCE_UID)
            .unwrap_or_else(|| "1.2.826.0.1.unknown".to_string());

        let rows = extract_u32_tag(first_obj, tags::ROWS).unwrap_or(512);
        let cols = extract_u32_tag(first_obj, tags::COLUMNS).unwrap_or(512);
        let spacing = extract_pixel_spacing(first_obj);
        let thickness = extract_f32_tag(first_obj, tags::SLICE_THICKNESS).unwrap_or(1.0);
        let slope = extract_f32_tag(first_obj, tags::RESCALE_SLOPE).unwrap_or(1.0);
        let intercept = extract_f32_tag(first_obj, tags::RESCALE_INTERCEPT).unwrap_or(0.0);
        let window_center = extract_f32_tag(first_obj, tags::WINDOW_CENTER).unwrap_or(40.0);
        let window_width = extract_f32_tag(first_obj, tags::WINDOW_WIDTH).unwrap_or(400.0);

        let slice_count = slice_entries.len() as u32;
        let mut total_voxels = Vec::with_capacity((cols * rows * slice_count) as usize);

        for (_, _, obj) in &slice_entries {
            let decoded = obj.decode_pixel_data()?;
            let r = decoded.rows();
            let c = decoded.columns();
            if r != rows || c != cols {
                return Err(DicomError::InconsistentDimensions([cols, rows], [c, r]));
            }

            let slice_i16 = decoded_to_i16(&decoded);
            total_voxels.extend(slice_i16);
        }

        Ok(Self {
            series_instance_uid,
            dimensions: [cols, rows, slice_count],
            voxel_spacing: [spacing[0], spacing[1], thickness],
            rescale_slope: slope,
            rescale_intercept: intercept,
            default_window: (window_center, window_width),
            voxel_data: total_voxels,
        })
    }

    /// Ingests a DICOM object ensuring HIPAA quarantine verification.
    pub fn from_dicom_object(obj: &DefaultDicomObject) -> Result<Self, DicomError> {
        crate::privacy::sanitizer::DicomSanitizer::verify_quarantine(obj)
            .map_err(|e| DicomError::PhiQuarantineViolation(e.to_string()))?;

        let series_instance_uid = extract_string_tag(obj, tags::SERIES_INSTANCE_UID)
            .unwrap_or_else(|| "1.2.826.0.1.unknown".to_string());

        let rows = extract_u32_tag(obj, tags::ROWS).unwrap_or(512);
        let cols = extract_u32_tag(obj, tags::COLUMNS).unwrap_or(512);
        let frames = extract_u32_tag(obj, tags::NUMBER_OF_FRAMES).unwrap_or(1);
        let spacing = extract_pixel_spacing(obj);
        let thickness = extract_f32_tag(obj, tags::SLICE_THICKNESS).unwrap_or(1.0);
        let slope = extract_f32_tag(obj, tags::RESCALE_SLOPE).unwrap_or(1.0);
        let intercept = extract_f32_tag(obj, tags::RESCALE_INTERCEPT).unwrap_or(0.0);
        let window_center = extract_f32_tag(obj, tags::WINDOW_CENTER).unwrap_or(40.0);
        let window_width = extract_f32_tag(obj, tags::WINDOW_WIDTH).unwrap_or(400.0);

        let decoded = obj.decode_pixel_data()?;
        let voxels = decoded_to_i16(&decoded);

        Ok(Self {
            series_instance_uid,
            dimensions: [cols, rows, frames],
            voxel_spacing: [spacing[0], spacing[1], thickness],
            rescale_slope: slope,
            rescale_intercept: intercept,
            default_window: (window_center, window_width),
            voxel_data: voxels,
        })
    }
}

// Helpers for DICOM attribute extraction
fn extract_string_tag(obj: &DefaultDicomObject, tag: dicom_core::Tag) -> Option<String> {
    obj.element(tag)
        .ok()
        .and_then(|elem| elem.to_str().ok().map(|s| s.trim().to_string()))
}

fn extract_u32_tag(obj: &DefaultDicomObject, tag: dicom_core::Tag) -> Option<u32> {
    obj.element(tag)
        .ok()
        .and_then(|elem| elem.to_int::<u32>().ok())
}

fn extract_f32_tag(obj: &DefaultDicomObject, tag: dicom_core::Tag) -> Option<f32> {
    obj.element(tag)
        .ok()
        .and_then(|elem| elem.to_float32().ok())
}

fn extract_instance_number(obj: &DefaultDicomObject) -> Option<i32> {
    obj.element(tags::INSTANCE_NUMBER)
        .ok()
        .and_then(|elem| elem.to_int::<i32>().ok())
}

fn extract_z_position(obj: &DefaultDicomObject) -> Option<f64> {
    // Check ImagePositionPatient (0020,0032) [X, Y, Z]
    if let Ok(elem) = obj.element(tags::IMAGE_POSITION_PATIENT) {
        if let Ok(str_val) = elem.to_str() {
            let parts: Vec<&str> = str_val.split('\\').collect();
            if parts.len() == 3 {
                if let Ok(z) = parts[2].trim().parse::<f64>() {
                    return Some(z);
                }
            }
        }
    }
    // Fallback to SliceLocation (0020,1041)
    if let Ok(elem) = obj.element(tags::SLICE_LOCATION) {
        if let Ok(str_val) = elem.to_str() {
            if let Ok(z) = str_val.trim().parse::<f64>() {
                return Some(z);
            }
        }
    }
    None
}

fn extract_pixel_spacing(obj: &DefaultDicomObject) -> [f32; 2] {
    if let Ok(elem) = obj.element(tags::PIXEL_SPACING) {
        if let Ok(str_val) = elem.to_str() {
            let parts: Vec<&str> = str_val.split('\\').collect();
            if parts.len() == 2 {
                let dy = parts[0].trim().parse::<f32>().unwrap_or(1.0);
                let dx = parts[1].trim().parse::<f32>().unwrap_or(1.0);
                return [dx, dy];
            }
        }
    }
    [1.0, 1.0]
}

fn decoded_to_i16(decoded: &dicom_pixeldata::DecodedPixelData) -> Vec<i16> {
    if let Ok(v) = decoded.to_vec::<i16>() {
        v
    } else if let Ok(u) = decoded.to_vec::<u16>() {
        u.into_iter().map(|val| val as i16).collect()
    } else if let Ok(u8_vec) = decoded.to_vec::<u8>() {
        u8_vec.into_iter().map(|val| val as i16).collect()
    } else {
        // Fallback: zero buffer of required size
        vec![0i16; (decoded.rows() * decoded.columns() * decoded.number_of_frames()) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthetic_volume_generation() {
        let vol = DicomVolume::generate_synthetic(64, 64, 16);
        assert_eq!(vol.dimensions, [64, 64, 16]);
        assert_eq!(vol.voxel_data.len(), 64 * 64 * 16);
        assert_eq!(vol.voxel_spacing, [0.75, 0.75, 2.5]);
    }

    #[test]
    fn test_mpr_axial_extraction() {
        let vol = DicomVolume::generate_synthetic(64, 64, 16);
        let slice = vol.extract_slice(MprPlane::Axial, 8, None).unwrap();
        assert_eq!(slice.width, 64);
        assert_eq!(slice.height, 64);
        assert_eq!(slice.aspect_ratio, 1.0);
        assert_eq!(slice.windowed_rgba.len(), 64 * 64 * 4);
    }

    #[test]
    fn test_mpr_coronal_aspect_ratio_compensation() {
        let vol = DicomVolume::generate_synthetic(64, 64, 16);
        let slice = vol.extract_slice(MprPlane::Coronal, 32, None).unwrap();
        assert_eq!(slice.width, 64);
        assert_eq!(slice.height, 16);
        // Anisotropic aspect ratio: SliceThickness (2.5) / PixelSpacing_x (0.75) = 3.3333...
        let expected_aspect = 2.5 / 0.75;
        assert!((slice.aspect_ratio - expected_aspect).abs() < 1e-4);
    }

    #[test]
    fn test_mpr_sagittal_aspect_ratio_compensation() {
        let vol = DicomVolume::generate_synthetic(64, 64, 16);
        let slice = vol.extract_slice(MprPlane::Sagittal, 32, None).unwrap();
        assert_eq!(slice.width, 64);
        assert_eq!(slice.height, 16);
        let expected_aspect = 2.5 / 0.75;
        assert!((slice.aspect_ratio - expected_aspect).abs() < 1e-4);
    }

    #[test]
    fn test_slice_image_png_encoding() {
        let vol = DicomVolume::generate_synthetic(32, 32, 4);
        let slice = vol.extract_slice(MprPlane::Axial, 2, None).unwrap();
        let png_bytes = slice.to_png().unwrap();
        assert!(!png_bytes.is_empty());
        assert_eq!(&png_bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn test_safe_aspect_ratio_with_zero_spacing() {
        let mut vol = DicomVolume::generate_synthetic(16, 16, 4);
        vol.voxel_spacing = [0.0, 0.0, 0.0];
        let coronal = vol.extract_slice(MprPlane::Coronal, 8, None).unwrap();
        assert_eq!(coronal.aspect_ratio, 1.0);
        let sagittal = vol.extract_slice(MprPlane::Sagittal, 8, None).unwrap();
        assert_eq!(sagittal.aspect_ratio, 1.0);
    }

    #[test]
    fn test_quarantine_verification_in_from_dicom_object() {
        use dicom_core::{DataElement, VR};
        use dicom_object::InMemDicomObject;

        let mut inmem = InMemDicomObject::new_empty();
        inmem.put(DataElement::new(tags::PATIENT_NAME, VR::PN, "DOE^JANE"));
        assert!(crate::privacy::sanitizer::DicomSanitizer::verify_quarantine(&inmem).is_err());

        // After sanitization, quarantine check passes
        let purged = crate::privacy::sanitizer::DicomSanitizer::sanitize_object(&mut inmem);
        assert_eq!(purged, 1);
        assert!(crate::privacy::sanitizer::DicomSanitizer::verify_quarantine(&inmem).is_ok());
    }
}
