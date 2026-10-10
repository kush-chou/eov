use serde::{Deserialize, Serialize};

/// WGSL shader source code for real-time GPU Window Width / Window Level (WWWL)
/// implementing the exact DICOM PS3.3 standard transfer function with the `-0.5` center offset.
pub const WWWL_WGSL_SHADER: &str = r#"
// Uniform buffer must adhere to strict 16-byte alignment (std140 layout)
struct WindowUniforms {
    slope: f32,
    intercept: f32,
    center: f32,
    width: f32,
};

@group(0) @binding(0) var<uniform> window: WindowUniforms;
@group(0) @binding(1) var raw_texture: texture_2d<f32>;
@group(0) @binding(2) var texture_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let raw_val = textureSample(raw_texture, texture_sampler, uv).r;
    let hu = raw_val * window.slope + window.intercept;
    let normalized = clamp(((hu - (window.center - 0.5)) / (window.width - 1.0)) + 0.5, 0.0, 1.0);
    return vec4<f32>(normalized, normalized, normalized, 1.0);
}
"#;

/// Extended WGSL shader supporting Multiplanar Reconstruction (MPR) aspect-ratio compensation.
pub const MPR_WWWL_WGSL_SHADER: &str = r#"
// Uniform buffer with 16-byte alignment: 8 x 4 bytes = 32 bytes (multiple of 16)
struct MprUniforms {
    slope: f32,
    intercept: f32,
    center: f32,
    width: f32,
    aspect_ratio: f32,
    slice_norm: f32,
    plane: u32,       // 0: Axial, 1: Coronal, 2: Sagittal
    _padding: u32,
};

@group(0) @binding(0) var<uniform> uniforms: MprUniforms;
@group(0) @binding(1) var volume_texture: texture_3d<f32>;
@group(0) @binding(2) var volume_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    var sample_coord: vec3<f32>;
    if (uniforms.plane == 0u) {
        // Axial (XY)
        sample_coord = vec3<f32>(uv.x, uv.y, uniforms.slice_norm);
    } else if (uniforms.plane == 1u) {
        // Coronal (XZ with aspect ratio scaling)
        sample_coord = vec3<f32>(uv.x, uniforms.slice_norm, uv.y / uniforms.aspect_ratio);
    } else {
        // Sagittal (YZ with aspect ratio scaling)
        sample_coord = vec3<f32>(uniforms.slice_norm, uv.x, uv.y / uniforms.aspect_ratio);
    }

    let raw_val = textureSample(volume_texture, volume_sampler, sample_coord).r;
    let hu = raw_val * uniforms.slope + uniforms.intercept;
    let normalized = clamp(((hu - (uniforms.center - 0.5)) / (uniforms.width - 1.0)) + 0.5, 0.0, 1.0);
    return vec4<f32>(normalized, normalized, normalized, 1.0);
}
"#;

/// 16-byte aligned Uniform Buffer struct matching the WGSL `WindowUniforms` layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowUniforms {
    pub slope: f32,
    pub intercept: f32,
    pub center: f32,
    pub width: f32,
}

impl WindowUniforms {
    pub fn new(slope: f32, intercept: f32, center: f32, width: f32) -> Self {
        Self {
            slope,
            intercept,
            center,
            width: width.max(1.0),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: WindowUniforms is repr(C) containing only four f32 (16 bytes total) with no uninit bytes.
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

/// 16-byte aligned Uniform Buffer struct matching the WGSL `MprUniforms` layout (32 bytes total).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MprUniforms {
    pub slope: f32,
    pub intercept: f32,
    pub center: f32,
    pub width: f32,
    pub aspect_ratio: f32,
    pub slice_norm: f32,
    pub plane: u32,       // 0: Axial, 1: Coronal, 2: Sagittal
    pub _padding: u32,
}

impl MprUniforms {
    pub fn new(
        slope: f32,
        intercept: f32,
        center: f32,
        width: f32,
        aspect_ratio: f32,
        slice_norm: f32,
        plane: u32,
    ) -> Self {
        Self {
            slope,
            intercept,
            center,
            width: width.max(1.0),
            aspect_ratio: aspect_ratio.max(0.001),
            slice_norm: slice_norm.clamp(0.0, 1.0),
            plane,
            _padding: 0,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: MprUniforms is repr(C) containing 8 32-bit fields (32 bytes total) with no uninit bytes.
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

/// Clinical radiology WWWL presets defined in DICOM PS3.3 / clinical imaging standards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClinicalPreset {
    SoftTissue,
    Lung,
    Bone,
    Brain,
}

impl ClinicalPreset {
    /// Returns (center, width) in Hounsfield Units (HU).
    pub fn window(&self) -> (f32, f32) {
        match self {
            ClinicalPreset::SoftTissue => (40.0, 400.0),
            ClinicalPreset::Lung => (-600.0, 1500.0),
            ClinicalPreset::Bone => (400.0, 1500.0),
            ClinicalPreset::Brain => (40.0, 80.0),
        }
    }

    pub fn all() -> &'static [ClinicalPreset] {
        &[
            ClinicalPreset::SoftTissue,
            ClinicalPreset::Lung,
            ClinicalPreset::Bone,
            ClinicalPreset::Brain,
        ]
    }
}

/// Evaluates the exact DICOM PS3.3 transfer function with the -0.5 center offset.
/// Transfer function:
/// HU = V_raw * RescaleSlope + RescaleIntercept
/// lower_bound = center - 0.5 - (width - 1.0) / 2.0
/// upper_bound = center - 0.5 + (width - 1.0) / 2.0
/// V_out = clamp(((HU - (center - 0.5)) / (width - 1.0)) + 0.5, 0.0, 1.0)
pub fn apply_wwwl_ps33(raw_val: i16, slope: f32, intercept: f32, center: f32, width: f32) -> f32 {
    let w = width.max(1.0);
    let hu = (raw_val as f32) * slope + intercept;
    let lower = center - 0.5 - (w - 1.0) / 2.0;
    let upper = center - 0.5 + (w - 1.0) / 2.0;

    if hu <= lower {
        0.0
    } else if hu >= upper {
        1.0
    } else {
        (((hu - (center - 0.5)) / (w - 1.0)) + 0.5).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_uniforms_16_byte_alignment() {
        assert_eq!(std::mem::size_of::<WindowUniforms>(), 16);
        assert_eq!(std::mem::align_of::<WindowUniforms>(), 4);
        let u = WindowUniforms::new(1.0, 0.0, 40.0, 400.0);
        assert_eq!(u.as_bytes().len(), 16);
    }

    #[test]
    fn test_mpr_uniforms_16_byte_alignment() {
        assert_eq!(std::mem::size_of::<MprUniforms>(), 32);
        assert_eq!(std::mem::size_of::<MprUniforms>() % 16, 0);
        assert_eq!(std::mem::align_of::<MprUniforms>(), 4);
        let mpr = MprUniforms::new(1.0, 0.0, 40.0, 400.0, 2.5, 0.5, 1);
        assert_eq!(mpr.as_bytes().len(), 32);
        assert_eq!(mpr.plane, 1);
        assert_eq!(mpr.aspect_ratio, 2.5);
    }

    #[test]
    fn test_clinical_presets_values() {
        assert_eq!(ClinicalPreset::SoftTissue.window(), (40.0, 400.0));
        assert_eq!(ClinicalPreset::Lung.window(), (-600.0, 1500.0));
        assert_eq!(ClinicalPreset::Bone.window(), (400.0, 1500.0));
        assert_eq!(ClinicalPreset::Brain.window(), (40.0, 80.0));
    }

    #[test]
    fn test_wwwl_ps33_transfer_function_exact_center() {
        // At center - 0.5, the output should be exactly 0.5
        let center = 40.0;
        let width = 400.0;
        let raw_at_center = 40; // HU = 40
        let val = apply_wwwl_ps33(raw_at_center, 1.0, 0.0, center, width);
        // ((40 - 39.5) / 399) + 0.5 = (0.5 / 399) + 0.5 ≈ 0.50125
        assert!((val - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_wwwl_ps33_clipping_bounds() {
        let center = 40.0;
        let width = 400.0;
        // Far below lower bound (-1000 HU)
        assert_eq!(apply_wwwl_ps33(-1000, 1.0, 0.0, center, width), 0.0);
        // Far above upper bound (+2000 HU)
        assert_eq!(apply_wwwl_ps33(2000, 1.0, 0.0, center, width), 1.0);
    }

    #[test]
    fn test_all_clinical_presets_monotonicity() {
        for preset in ClinicalPreset::all() {
            let (c, w) = preset.window();
            let mut prev = 0.0;
            for hu in (-1500..=2500).step_by(50) {
                let curr = apply_wwwl_ps33(hu as i16, 1.0, 0.0, c, w);
                assert!(curr >= prev, "Preset {:?} violated monotonicity at HU {}", preset, hu);
                assert!((0.0..=1.0).contains(&curr));
                prev = curr;
            }
        }
    }
}
