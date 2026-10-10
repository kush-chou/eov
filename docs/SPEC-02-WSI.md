# Specification: Whole Slide Image (WSI) Engine

**Document Identifier**: `SPEC-LAMINA-02-WSI`  
**System Title**: Gigapixel Histopathology Viewport & Tile Blitter  
**Revision**: `0.1.0-draft`  
**Status**: Rough Draft (To Revisit Post-Implementation)  
**Parent Document**: `SPEC-LAMINA-00-MASTER`  

---

## 1. Objective & Scope
Specifies the whole-slide imaging subsystem responsible for opening gigapixel histopathology slides (`.svs`, `.ndpi`, `.mrxs`, `.tif`), decoding pyramid tiles via dynamic OpenSlide bindings, caching tiles in a bounded memory pool, and projecting slide-space micron coordinates to the viewport canvas.

---

## 2. Dynamic Linking & Build Requirements

- **LGPL-2.1 Invariant**: OpenSlide C bindings (`openslide-rs`) must link dynamically to host system shared libraries:
  - macOS (Apple Silicon): `/opt/homebrew/lib/libopenslide.0.dylib`
  - Linux: `/usr/lib/x86_64-linux-gnu/libopenslide.so`
  - Windows: `libopenslide-0.dll`
- Static linking of OpenSlide is strictly prohibited.

---

## 3. Tile Decoding & Caching Pipeline

```text
WSI File on Disk (.svs, .ndpi)
        │
        ▼ (mmap via OpenSlide)
┌───────────────────────────────┐
│  Tokio Worker Thread Pool     │
│  (tokio::task::spawn_blocking)│
└──────────────┬────────────────┘
               │ Decodes 256×256 RGBA tiles
               ▼
┌───────────────────────────────┐
│ Bounded LRU Cache (512 MB)    │  (holds ~2,000 tiles)
└──────────────┬────────────────┘
               │ Raw byte stream
               ▼
┌───────────────────────────────┐
│ Custom URI Protocol Handler   │
│ (`lamina-tile://...`)         │
└──────────────┬────────────────┘
               │ Zero-copy transfer
               ▼
┌───────────────────────────────┐
│ Webview Viewport Canvas       │ (HTML5 Canvas / WebGPU)
└───────────────────────────────┘
```

### 3.1 Tile Specifications
- **Dimensions**: $256 \times 256$ pixels.
- **Color Format**: 32-bit RGBA uncompressed or JPEG/WebP compressed.
- **Cache Sizing**: Upper bound of $512\text{ MB}$ ($\approx 2,000$ uncompressed tiles). Eviction follows Least Recently Used (LRU) policy.
- **Downsample Math**: OpenSlide `read_region` receives Level 0 pixel coordinates:
  $$x_0 = \text{tile\_x} \times 256 \times \text{downsample}(l)$$
  $$y_0 = \text{tile\_y} \times 256 \times \text{downsample}(l)$$

### 3.2 High-Throughput Tile Protocol
Tiles are served via a custom Tauri asset protocol (`lamina-tile://{slide_id}/{level}/{x}/{y}`) returning raw image bytes directly to avoid Base64 JSON IPC serialization overhead.

---

## 4. Physical Coordinate Space (Microns)

Histopathology annotations and ROI bounding boxes are anchored in physical slide microns ($\mu\text{m}$) using the slide's optical Microns Per Pixel (`MPP`):

### 4.1 Coordinate Projection
$$x_{\mu\text{m}} = x_{\text{pixel\_level0}} \times \text{MPP}_x$$
$$y_{\mu\text{m}} = y_{\text{pixel\_level0}} \times \text{MPP}_y$$

Screen projection at zoom scale $Z$ with viewport origin $O_{\mu\text{m}}$:
$$x_{\text{screen}} = (x_{\mu\text{m}} - O_{x,\mu\text{m}}) \times \frac{Z}{\text{MPP}_x}$$
$$y_{\text{screen}} = (y_{\mu\text{m}} - O_{y,\mu\text{m}}) \times \frac{Z}{\text{MPP}_y}$$

### 4.2 Data Types
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SlideMetadata {
    pub slide_id: String,
    pub width_px: u64,
    pub height_px: u64,
    pub mpp_x: f64,
    pub mpp_y: f64,
    pub level_count: u32,
    pub level_downsamples: Vec<f64>,
}
```

---

## 5. ROI Extraction Contract

When the clinician activates the Quick Query HUD (`Opt + Space`), the engine extracts the selected bounding box:
```rust
#[tauri::command]
pub async fn extract_roi_patch(
    slide_id: String,
    rect: SlideRect,
    max_dimension_px: u32, // default: 1024
) -> Result<Vec<u8>, String>;
```
- The extracted patch is downsampled in memory to $\le 1024 \times 1024$ RGBA pixels and encoded to PNG.
- Label, barcode, and macro overview areas are strictly excluded from extraction.

---

## 6. Verification Criteria
1. Slide load time for a $2\text{ GB}$ `.svs` file takes $< 100\text{ ms}$ (metadata extraction).
2. Panning through level 0 maintains $\ge 60\text{ FPS}$ with cached tile retrieval $< 1\text{ ms}$.
3. Coordinate projections retain zero drift across zoom levels $0.1\times$ to $80\times$.
