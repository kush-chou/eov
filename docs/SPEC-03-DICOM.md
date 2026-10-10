# Specification: Volumetric Radiology (DICOM) Engine

**Document Identifier**: `SPEC-LAMINA-03-DICOM`  
**System Title**: Volumetric DICOM Ingestion, GPU WWWL & MPR Reslicing  
**Revision**: `0.1.0-draft`  
**Status**: Rough Draft (To Revisit Post-Implementation)  
**Parent Document**: `SPEC-LAMINA-00-MASTER`  

---

## 1. Objective & Scope
Defines the radiology subsystem for ingesting multi-slice CT/MRI DICOM series, converting raw pixel data to Hounsfield Units (HU), applying real-time GPU Window Width / Window Level (WWWL) transformations, and supporting Multiplanar Reconstruction (MPR).

---

## 2. Ingestion & Transfer Syntaxes

- **Decoder**: Native Rust `dicom-rs` ecosystem (`dicom-object`, `dicom-pixeldata`).
- **Supported Syntaxes**:
  - `1.2.840.10008.1.2` (Implicit VR Little Endian)
  - `1.2.840.10008.1.2.1` (Explicit VR Little Endian)
  - `1.2.840.10008.1.2.4.50` (JPEG Baseline Process 1)
  - `1.2.840.10008.1.2.4.90` (JPEG 2000 Part 1 Lossless/Lossy)
  - `1.2.840.10008.1.2.5` (RLE Lossless)

### In-Memory Volume Struct
```rust
#[derive(Debug, Clone)]
pub struct DicomVolume {
    pub series_instance_uid: String,
    pub dimensions: [u32; 3], // [width, height, slices]
    pub voxel_spacing: [f32; 3], // [pixel_spacing_x, pixel_spacing_y, slice_thickness]
    pub rescale_slope: f32,
    pub rescale_intercept: f32,
    pub default_window: (f32, f32), // (center, width)
    pub voxel_data: Vec<i16>, // 16-bit raw signed pixel buffer
}
```

---

## 3. Real-Time GPU Windowing (WWWL)

Window Width and Window Level transformations are performed inside a GPU fragment/compute shader rather than mutating CPU pixel buffers:

### 3.1 Transfer Function (DICOM PS3.3 Standard)
Raw pixel conversion to normalized $[0.0, 1.0]$ luminance:
$$\text{HU} = V_{\text{raw}} \cdot \text{RescaleSlope} + \text{RescaleIntercept}$$
$$V_{\text{out}} = \text{clamp}\left(\frac{\text{HU} - (c - 0.5)}{w - 1} + 0.5, 0.0, 1.0\right)$$
where $c$ is Window Center and $w$ is Window Width.

### 3.2 WGSL Shader Implementation
```wgsl
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
```

### 3.3 Clinical Presets
- **Soft Tissue**: Center $+40$, Width $400$
- **Lung**: Center $-600$, Width $1500$
- **Bone**: Center $+400$, Width $1500$
- **Brain**: Center $+40$, Width $80$

---

## 4. Multiplanar Reconstruction (MPR)
- **Planes**: Axial ($XY$), Coronal ($XZ$), Sagittal ($YZ$).
- **Aspect Ratio Compensation**: Corrects non-cubic voxel anisotropy using physical spacing ratio:
  $$\text{Aspect}_{\text{coronal}} = \frac{\text{SliceThickness}}{\text{PixelSpacing}_x}$$

---

## 5. HIPAA PHI In-Memory Quarantine

Prior to rendering or extracting any slice for clinical query:
1. Patient tags `(0010,0010)` PatientName, `(0010,0020)` PatientID, `(0010,0030)` PatientBirthDate are stripped.
2. Institution tags `(0008,0080)` InstitutionName, `(0008,0050)` AccessionNumber are stripped.
3. Only the physical voxel buffer and geometric spacing metrics are retained in memory.

---

## 6. Verification Criteria
1. DICOM series loads from local disk and decodes $\ge 500$ slices in $< 1.5\text{ seconds}$.
2. WWWL uniform mutations adjust display contrast with sub-1ms GPU latency ($>60\text{ FPS}$).
3. Verification tests confirm complete absence of patient demographic metadata in extracted query buffers.
