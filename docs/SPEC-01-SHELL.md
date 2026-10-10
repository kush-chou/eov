# Specification: Tauri v2 Desktop Shell & Workspace

**Document Identifier**: `SPEC-LAMINA-01-SHELL`  
**System Title**: Desktop Shell, Window Management & Workspace Ergonomics  
**Revision**: `0.1.0-draft`  
**Status**: Rough Draft (To Revisit Post-Implementation)  
**Parent Document**: `SPEC-LAMINA-00-MASTER`  

---

## 1. Objective & Scope
Defines the client application shell built on Tauri v2, managing window creation, multi-panel workspace tiling, global keyboard shortcuts, and the command palette.

---

## 2. Shell Architecture (Tauri v2)

```text
┌─────────────────────────────────────────────────────────────┐
│                 TAURI V2 DESKTOP APPLICATION                │
├─────────────────────────────────────────────────────────────┤
│ Rust Backend Core (`src-tauri`):                            │
│ ├── Application State (`AppState`)                          │
│ ├── Global Shortcut Listener (`tauri-plugin-global-shortcut`)│
│ ├── Window State Persistence (`tauri-plugin-window-state`)  │
│ └── Custom Protocol Registrations (`lamina-tile://`)         │
├─────────────────────────────────────────────────────────────┤
│ Webview Frontend (`src`):                                   │
│ ├── Workspace Tiling Engine (`dockview`)                    │
│ ├── Command Palette Overlay (`cmdk`)                        │
│ └── Global Diagnostic HUD Overlay (`Opt + Space`)           │
└─────────────────────────────────────────────────────────────┘
```

### 2.1 Window Configurations
- **Default Resolution**: $1440 \times 900$, minimum $1024 \times 768$.
- **Window Decorations**: Native titlebar hidden or integrated with window drag controls (`data-tauri-drag-region`).
- **Performance**: Hardware acceleration enabled via Webview2 (Windows) / WKWebView (macOS).

---

## 3. Workspace Layout & Panel Tiling
- **Tiling Engine**: `dockview` (or lightweight CSS grid flex layout).
- **Supported Panel Types**:
  1. `wsi_viewport`: Histopathology canvas.
  2. `dicom_viewport`: Radiology volumetric viewer.
  3. `literature_reader`: Webview container for PubMed and clinical PDFs.
- **Split Behaviors**: Horizontal and vertical splits with persistent panel sizing ratios.

---

## 4. Keyboard Shortcuts & Command Palette

### 4.1 Global Shortcut
- **Key Combination**: `Option + Space` (macOS) / `Alt + Space` (Windows).
- **Behavior**: Toggles the Diagnostic ROI Quick Query HUD over the currently focused viewport.
- **Plugin**: `tauri-plugin-global-shortcut`.

### 4.2 Application Command Palette
- **Trigger**: `Cmd + Shift + P` (macOS) / `Ctrl + Shift + P` (Windows).
- **Capabilities**:
  - Open WSI slide (`.svs`, `.ndpi`, `.mrxs`, `.tif`).
  - Open DICOM series directory.
  - Split viewport horizontally / vertically.
  - Cycle radiology windowing presets (Soft Tissue, Lung, Bone, Brain).

---

## 5. IPC Contract Definitions

### Shell Commands
```rust
#[tauri::command]
pub async fn open_file_dialog(filter_type: String) -> Result<Option<String>, String>;

#[tauri::command]
pub async fn toggle_quick_query_hud(app_handle: tauri::AppHandle) -> Result<(), String>;
```

### Frontend State Events
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActiveViewportChangedEvent {
    pub viewport_id: String,
    pub kind: String, // "wsi" | "dicom" | "reader"
}
```

---

## 6. Verification & Acceptance Criteria
1. Application compiles cleanly via `cargo check` and launches under Tauri v2.
2. `Option + Space` summons and dismisses the query overlay without window focus loss.
3. Multi-panel splits resize smoothly without dropping viewport rendering state.
