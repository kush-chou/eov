# System Specification: Lamina Clinical Workstation

**Document Identifier**: `SPEC-LAMINA-00-MASTER`  
**System Title**: Lamina Clinical Workstation  
**Revision**: `2.0.0`  
**Classification**: Standalone Clinical Decision Support (CDS) Client Architecture  
**Target Platforms**: macOS (Apple Silicon `aarch64`), Windows 10/11 (`x86_64`)  

---

## 1. System Topology & Architectural Invariants

Lamina is a native desktop workstation that executes on-device Whole Slide Image (WSI) histopathology and multi-frame DICOM radiology decoding/rendering, paired with cloud-streamed clinical decision support.

```text
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                 SYSTEM ARCHITECTURE                                    │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ 1. DESKTOP CLIENT (Tauri v2 Shell)                                                     │
│    ├── UI Thread: Webview viewport canvas, literature reader, ROI query HUD.           │
│    └── Core Runtime (Rust):                                                            │
│        ├── WSI Engine: OpenSlide C FFI dynamic linking + pyramid tile LRU cache.       │
│        ├── DICOM Engine: dicom-rs parsing + GPU fragment shader WWWL.                  │
│        ├── Privacy Boundary: Strict in-memory optical allow-list (zero PHI egress).    │
│        └── Inference Client: Gateway dispatcher & SSE stream consumer.                 │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ 2. CLOUD INFERENCE PIPELINE                                                            │
│    ├── Gateway: Token authentication & request dispatch.                               │
│    ├── Synthesis Engine (System 2): Google Gemini Interactions API                     │
│    │   └── Remote execution, search grounding, and literature retrieval.               │
│    └── Verification Guardrail (System 1): Microsoft-Decision-1 API                     │
│        └── Discrete scoring, differential calibration, and citation verification.      │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### Invariants
1. **Dynamic Linking**: OpenSlide MUST be dynamically linked against the host system C shared library (`libopenslide.0.dylib` on macOS, `libopenslide-0.dll` on Windows) to maintain LGPL-2.1 compliance.
2. **Local PHI Quarantine**: No patient demographic information, medical record numbers, accession IDs, or uncropped whole-slide overview/label macro images may ever cross network boundaries.
3. **Viewport Frame Budget**: Panning and zooming across pyramidal gigapixel slides and volumetric DICOM stacks must execute at $\ge 60\text{ FPS}$ with sub-16ms render loop latency.
4. **Clean-Room Interfaces**: All cross-boundary IPC commands and public structs use explicit, backwards-compatible, serde-serializable contracts.

---

## 2. Core Subsystems

### 2.1 Whole Slide Imaging (WSI) Engine
- **Decoders**: OpenSlide C API via safe Rust abstractions (`openslide-rs`).
- **Tile Architecture**:
  - Standard pyramid tile dimensions: $256 \times 256$ pixels (32-bit RGBA).
  - Multi-resolution pyramid downsamples ($1\times, 2\times, 4\times, 8\times, 16\times, 32\times$).
  - Tile Cache: In-memory bounded LRU cache with an upper ceiling of $512\text{ MB}$.
- **Coordinate Space**:
  - Physical reference frame: Slide microns ($\mu\text{m}$) using slide-level microns-per-pixel (`MPP`).
  - Screen projection:
    $$P_{\text{screen}} = \left( (P_{\text{slide}} - O_{\text{viewport}}) \times \frac{\text{Zoom}}{\text{MPP}} \right)$$

### 2.2 Radiology (DICOM) Engine
- **Parser**: Native Rust DICOM decoder (`dicom-rs` ecosystem).
- **Window Width / Window Level (WWWL)**:
  - Real-time windowing executed on the GPU via WGSL fragment shader rather than baking into CPU pixel memory.
  - Transfer function:
    $$V_{\text{out}} = \text{clamp}\left(\frac{(V_{\text{raw}} \cdot \text{RescaleSlope} + \text{RescaleIntercept}) - (\text{Center} - 0.5)}{\text{Width} - 1} + 0.5, 0.0, 1.0\right)$$
- **Volumetric Reconstruction**:
  - Orthogonal slicing: Multiplanar Reconstruction (MPR) across Axial, Coronal, and Sagittal planes.
  - Aspect ratio preservation based on `PixelSpacing` ($x, y$) and `SliceThickness` ($z$).

### 2.3 Native Literature Reader
- **Rendering**: Embedded Tauri Webview component dedicated to literature inspection.
- **Supported Media**: Public medical journals, PubMed web endpoints, and local/remote clinical PDF documents.
- **Bridge Contract**: Bidirectional JS bridge for programmatic text selection extraction, allowing clinicians to pipe excerpts directly into diagnostic queries.

### 2.4 Clinical Diagnostic HUD (`Opt + Space`)
- **Query Card**: Ephemeral overlay triggered by global keyboard shortcut (`Option+Space` / `Alt+Space`).
- **ROI Binding**:
  - Clinician selects region of interest (ROI) via mouse drag.
  - Coordinates stored in slide-space microns (`SlideRect`) to prevent coordinate drift during viewport transformation.
  - Image patch downsampled to $\le 1024 \times 1024$ RGBA, encoded to JPEG/PNG for query attachment.

---

## 3. HIPAA Privacy & In-Memory PHI Sanitization

Before any payload is dispatched from the local desktop client to the cloud gateway, it must pass through an in-memory optical allow-list:

### Allow-List Specifications
| Field | Status | Handling |
| :--- | :--- | :--- |
| **Visual Patch** | Allowed | Cropped sub-region $\le 1024 \times 1024$ pixels. |
| **Clinician Query** | Allowed | Plaintext input submitted in the HUD prompt. |
| **Physical Magnification** | Allowed | Floating-point MPP or optical objective ($10\times, 20\times, 40\times$). |
| **Slide Label / Barcode** | **Forbidden** | Excluded in memory; never read from OpenSlide handle. |
| **Macro / Overview Camera** | **Forbidden** | Excluded in memory; never read from OpenSlide handle. |
| **DICOM Patient Tags** | **Forbidden** | `(0010, 0010)` PatientName, `(0010, 0020)` PatientID, `(0010, 0030)` PatientBirthDate stripped. |
| **DICOM Institution / Study Tags** | **Forbidden** | `(0008, 0080)` InstitutionName, `(0008, 0050)` AccessionNumber stripped. |

---

## 4. Cloud Inference Pipeline & Protocol

The system employs a two-tier intelligence pipeline separating deep generative synthesis from rapid calibration verification:

```text
Clinician Query + Patch
        │
        ▼
┌───────────────────────────────┐
│     Lamina Cloud Gateway      │
└───────┬───────────────┬───────┘
        │               │
        ▼               ▼
┌──────────────┐ ┌──────────────┐
│   Tier 2     │ │   Tier 1     │
│   Gemini     │ │  Microsoft  │
│ Interactions │ │  Decision-1  │
│     API      │ │     API      │
└───────┬──────┘ └──────┬───────┘
        │               │
        │ Synthesis     │ Calibration
        │ + Citations   │ & Confidence
        ▼               ▼
┌───────────────────────────────┐
│ Streamed Aggregated Response  │
└───────────────────────────────┘
```

### 4.1 Tier 2: Deep Grounded Synthesis (Google Gemini Interactions API)
- **Protocol**: `POST /v1beta/interactions` using Server-Sent Events (SSE).
- **Runtime**: Managed agent runtime (`agent="antigravity-preview-09-2026"` or `model="gemini-3.8-flash"`).
- **State Management**:
  - Multi-turn conversation tracked via `previous_interaction_id`.
  - Remote execution environment tracked via `environment_id`.
- **Tools**: Live PubMed search grounding, URL ingestion, and sandboxed code execution for deterministic clinical calculations.
- **Output**: Structured clinical Markdown (immunohistochemical panels, differential diagnoses, literature citations `[1]`).

### 4.2 Tier 1: Verification & Calibration Guardrail (Microsoft-Decision-1)
- **Role**: Validates clinical differential rankings and verifies literature citations against known peer-reviewed evidence indices.
- **Latency Target**: Sub-100ms evaluation.
- **Output**: Calibrated confidence scores, evidence quality metrics, and flag states for conflicting data.

---

## 5. Data Contracts & Serialization Schemas

All IPC communication between the Tauri frontend and the Rust backend uses serde-compatible JSON:

### Coordinate Definitions
```rust
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SlidePoint {
    pub x_um: f64,
    pub y_um: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SlideRect {
    pub origin: SlidePoint,
    pub width_um: f64,
    pub height_um: f64,
}
```

### Viewport Commands
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ViewportCommand {
    LoadSlide { path: String },
    LoadDicom { series_dir: String },
    RequestTile { level: u32, tile_x: u32, tile_y: u32 },
    SetWindowLevel { window_width: f32, window_level: f32 },
    ExtractRoiPatch { rect: SlideRect, max_dimension: u32 },
}
```

### Inference Request & Response
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiagnosticQueryRequest {
    pub query_text: String,
    pub patch_png_base64: Option<String>,
    pub magnification_mpp: Option<f64>,
    pub previous_interaction_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum DiagnosticQueryEvent {
    TokenDelta { text: String },
    VerificationScore { confidence: f32, citation_verified: bool },
    Completed { interaction_id: String },
    Error { code: String, message: String },
}
```

---

## 6. Verification Criteria & Acceptance Tests

1. **WSI Dynamic Linking**: `cargo test -p lamina-wsi` passes on systems with system OpenSlide dynamically loaded; fails with explicit dynamic loader error if OpenSlide `.dylib` or `.so` is absent.
2. **Coordinate Stability**: Unit tests prove `SlidePoint` to screen pixel projections maintain zero coordinate drift across zoom scales ($0.1\times$ to $100\times$).
3. **PHI Sanitization**: Integration tests prove DICOM files processed by the privacy filter have all patient tags purged, returning `None` for all tag queries outside the optical allow-list.
4. **Tile Blitting Performance**: Benchmarks verify $256 \times 256$ tile dispatch from cache executes in $< 1\text{ ms}$.
