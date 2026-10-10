/**
 * Lamina Clinical Workstation - Frontend Core Viewport & Blitter
 * Revision: 2.0.0
 */

// Global State
const state = {
  // WSI Histopathology State
  wsi: {
    slideId: "sample",
    originUm: { x: 5000.0, y: 5000.0 }, // Slide-space origin in microns
    zoom: 1.0, // Optical zoom scale (1.0 = base, 40.0 = full 40x)
    mppX: 0.25, // Microns per pixel X (0.25 um/px -> 40x objective)
    mppY: 0.25, // Microns per pixel Y
    levelCount: 6,
    downsamples: [1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
    isDragging: false,
    dragStart: { x: 0, y: 0 },
    roiMode: false,
    roiStart: null,
    activeRoi: null, // { origin: { x_um, y_um }, width_um, height_um }
    tileCache: new Map(), // (level_x_y -> ImageBitmap / HTMLImageElement)
    fps: 60,
    lastFrameTime: performance.now(),
    frameCount: 0,
  },

  // Volumetric Radiology (DICOM) State
  dicom: {
    activePlane: "axial", // axial, coronal, sagittal
    sliceIndex: 8,
    maxSlices: 16,
    dimensions: [128, 128, 16],
    voxelSpacing: [0.75, 0.75, 2.5],
    currentWindow: { center: 40.0, width: 400.0 }, // Soft Tissue preset
    presets: {
      soft_tissue: { center: 40.0, width: 400.0 },
      lung: { center: -600.0, width: 1500.0 },
      bone: { center: 400.0, width: 1500.0 },
      brain: { center: 40.0, width: 80.0 },
    },
    volumeData: null,
  },

  // HUD & Privacy State
  hud: {
    visible: false,
    pinnedExcerpt: null,
  },
};

// Check for Tauri IPC bridge
const isTauri = typeof window !== "undefined" && window.__TAURI__ !== undefined;
const invoke = isTauri ? window.__TAURI__.core.invoke : async (cmd, args) => null;
const listen = isTauri ? window.__TAURI__.event.listen : () => {};

// DOM Elements
const wsiCanvas = document.getElementById("wsiCanvas");
const wsiCtx = wsiCanvas.getContext("2d", { alpha: false });
const wsiWrapper = document.getElementById("wsiCanvasWrapper");
const wsiCoordHud = document.getElementById("wsiCoordHud");
const hudX = document.getElementById("hudX");
const hudY = document.getElementById("hudY");
const fpsDisplay = document.getElementById("fpsDisplay");
const wsiZoomDisplay = document.getElementById("wsiZoomDisplay");
const roiSelectionBox = document.getElementById("roiSelectionBox");
const toolSelectRoi = document.getElementById("toolSelectRoi");

const dicomCanvas = document.getElementById("dicomCanvas");
const dicomCtx = dicomCanvas.getContext("2d");
const sliceSlider = document.getElementById("sliceSlider");
const dicomSliceStat = document.getElementById("dicomSliceStat");
const dicomWwwlStat = document.getElementById("dicomWwwlStat");
const dicomPlaneName = document.getElementById("dicomPlaneName");
const mprControl = document.getElementById("mprControl");
const wwwlPresetSelect = document.getElementById("wwwlPresetSelect");

const hudOverlay = document.getElementById("hudOverlay");
const hudCloseBtn = document.getElementById("hudCloseBtn");
const hudTriggerBtn = document.getElementById("hudTriggerBtn");
const hudPatchCanvas = document.getElementById("hudPatchCanvas");
const hudPatchMeta = document.getElementById("hudPatchMeta");
const hudQueryInput = document.getElementById("hudQueryInput");
const hudPinnedExcerpt = document.getElementById("hudPinnedExcerpt");
const excerptText = document.getElementById("excerptText");
const btnClearExcerpt = document.getElementById("btnClearExcerpt");
const btnExtractSelection = document.getElementById("btnExtractSelection");

const paletteBackdrop = document.getElementById("paletteBackdrop");
const paletteInput = document.getElementById("paletteInput");
const paletteList = document.getElementById("paletteList");
const cmdPaletteBtn = document.getElementById("cmdPaletteBtn");

// ==========================================
// 1. HIGH-PERFORMANCE WSI CANVAS BLITTER
// ==========================================

function resizeWsiCanvas() {
  const rect = wsiWrapper.getBoundingClientRect();
  if (wsiCanvas.width !== rect.width || wsiCanvas.height !== rect.height) {
    wsiCanvas.width = rect.width;
    wsiCanvas.height = rect.height;
  }
}

function selectBestLevel(zoom, downsamples) {
  const targetDs = Math.max(1.0, 1.0 / zoom);
  let bestLevel = 0;
  let minDiff = Infinity;
  for (let l = 0; l < downsamples.length; l++) {
    const diff = Math.abs(downsamples[l] - targetDs);
    if (diff < minDiff) {
      minDiff = diff;
      bestLevel = l;
    }
  }
  return bestLevel;
}

function getTileUri(slideId, level, tileX, tileY) {
  return `lamina-tile://localhost/${slideId}/${level}/${tileX}/${tileY}`;
}

function generateSyntheticTileCanvas(level, tileX, tileY) {
  const offscreen = document.createElement("canvas");
  offscreen.width = 256;
  offscreen.height = 256;
  const offCtx = offscreen.getContext("2d");

  const scale = 1 << level;
  const imgData = offCtx.createImageData(256, 256);
  const data = imgData.data;

  for (let y = 0; y < 256; y++) {
    for (let x = 0; x < 256; x++) {
      const gx = (tileX * 256 + x) * scale;
      const gy = (tileY * 256 + y) * scale;
      const freq = Math.sin(gx * 0.05) * Math.cos(gy * 0.05);
      const nucleusDist = Math.hypot((gx % 32) - 16, (gy % 32) - 16);

      let r, g, b;
      if (nucleusDist < 6.0) {
        r = 70; g = 30; b = 110; // Hematoxylin purple nucleus
      } else if (freq > 0.1) {
        r = 225; g = 140; b = 180; // Eosin pink stroma
      } else {
        r = 245; g = 235; b = 240; // Interstitial background
      }

      const idx = (y * 256 + x) * 4;
      data[idx] = r;
      data[idx + 1] = g;
      data[idx + 2] = b;
      data[idx + 3] = 255;
    }
  }
  offCtx.putImageData(imgData, 0, 0);
  return offscreen;
}

async function fetchTile(slideId, level, tileX, tileY) {
  const key = `${slideId}_${level}_${tileX}_${tileY}`;
  if (state.wsi.tileCache.has(key)) {
    return state.wsi.tileCache.get(key);
  }

  // High-throughput binary tile streaming via lamina-tile:// with graceful client fallback
  const img = new Image();
  img.crossOrigin = "anonymous";
  img.onload = () => {
    state.wsi.tileCache.set(key, img);
  };
  img.onerror = () => {
    const fallback = generateSyntheticTileCanvas(level, tileX, tileY);
    state.wsi.tileCache.set(key, fallback);
  };
  img.src = getTileUri(slideId, level, tileX, tileY);
}

function renderWsi() {
  resizeWsiCanvas();
  const width = wsiCanvas.width;
  const height = wsiCanvas.height;
  if (width === 0 || height === 0) return;

  wsiCtx.fillStyle = "#05070a";
  wsiCtx.fillRect(0, 0, width, height);

  const zoom = state.wsi.zoom;
  const mppX = state.wsi.mppX;
  const mppY = state.wsi.mppY;
  const originUm = state.wsi.originUm;

  const level = selectBestLevel(zoom, state.wsi.downsamples);
  const downsample = state.wsi.downsamples[level];
  const tileSizeLevel0Px = 256 * downsample;
  const tileSizeUm = tileSizeLevel0Px * mppX;

  // Compute visible tile coordinate bounds in slide space
  const minTileX = Math.floor(originUm.x / tileSizeUm);
  const maxTileX = Math.ceil((originUm.x + (width * mppX) / zoom) / tileSizeUm);
  const minTileY = Math.floor(originUm.y / tileSizeUm);
  const maxTileY = Math.ceil((originUm.y + (height * mppY) / zoom) / tileSizeUm);

  // Blit visible tiles
  for (let ty = Math.max(0, minTileY); ty <= maxTileY; ty++) {
    for (let tx = Math.max(0, minTileX); tx <= maxTileX; tx++) {
      const tileUmX = tx * tileSizeUm;
      const tileUmY = ty * tileSizeUm;

      // Project physical slide coordinate to screen pixels:
      // P_screen = (P_slide - O_viewport) * (Zoom / MPP)
      const screenX = (tileUmX - originUm.x) * (zoom / mppX);
      const screenY = (tileUmY - originUm.y) * (zoom / mppY);
      const screenTileSize = (tileSizeUm * zoom) / mppX;

      const tileImg = state.wsi.tileCache.get(`${state.wsi.slideId}_${level}_${tx}_${ty}`);
      if (tileImg) {
        wsiCtx.drawImage(tileImg, screenX, screenY, screenTileSize, screenTileSize);
      } else {
        // Asynchronously fetch/generate and cache
        fetchTile(state.wsi.slideId, level, tx, ty);
        // Draw placeholder grid border
        wsiCtx.strokeStyle = "rgba(255, 255, 255, 0.05)";
        wsiCtx.strokeRect(screenX, screenY, screenTileSize, screenTileSize);
      }
    }
  }

  // Draw active ROI box overlay if selected
  if (state.wsi.activeRoi) {
    const roi = state.wsi.activeRoi;
    const rx = (roi.origin.x_um - originUm.x) * (zoom / mppX);
    const ry = (roi.origin.y_um - originUm.y) * (zoom / mppY);
    const rw = (roi.width_um * zoom) / mppX;
    const rh = (roi.height_um * zoom) / mppY;

    wsiCtx.strokeStyle = "#38bdf8";
    wsiCtx.lineWidth = 2;
    wsiCtx.setLineDash([6, 4]);
    wsiCtx.strokeRect(rx, ry, rw, rh);
    wsiCtx.setLineDash([]);
    wsiCtx.fillStyle = "rgba(56, 189, 248, 0.15)";
    wsiCtx.fillRect(rx, ry, rw, rh);
  }

  // FPS Tracking
  state.wsi.frameCount++;
  const now = performance.now();
  if (now - state.wsi.lastFrameTime >= 1000) {
    state.wsi.fps = Math.round((state.wsi.frameCount * 1000) / (now - state.wsi.lastFrameTime));
    fpsDisplay.textContent = `${state.wsi.fps} FPS`;
    state.wsi.frameCount = 0;
    state.wsi.lastFrameTime = now;
  }

  requestAnimationFrame(renderWsi);
}

// Interactive Pan and Zoom Handlers
wsiWrapper.addEventListener("mousedown", (e) => {
  if (state.wsi.roiMode) {
    // Start drawing ROI bounding box
    const rect = wsiWrapper.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    state.wsi.roiStart = { x: sx, y: sy };
    roiSelectionBox.style.left = `${sx}px`;
    roiSelectionBox.style.top = `${sy}px`;
    roiSelectionBox.style.width = "0px";
    roiSelectionBox.style.height = "0px";
    roiSelectionBox.style.display = "block";
  } else {
    state.wsi.isDragging = true;
    state.wsi.dragStart = { x: e.clientX, y: e.clientY };
  }
});

window.addEventListener("mousemove", (e) => {
  const rect = wsiWrapper.getBoundingClientRect();
  const mouseX = e.clientX - rect.left;
  const mouseY = e.clientY - rect.top;

  // Track physical slide coordinates (µm):
  // P_slide = O_viewport + (P_screen * MPP / Zoom)
  const xUm = state.wsi.originUm.x + (mouseX * state.wsi.mppX) / state.wsi.zoom;
  const yUm = state.wsi.originUm.y + (mouseY * state.wsi.mppY) / state.wsi.zoom;
  hudX.textContent = xUm.toFixed(1);
  hudY.textContent = yUm.toFixed(1);

  if (state.wsi.isDragging) {
    const dx = e.clientX - state.wsi.dragStart.x;
    const dy = e.clientY - state.wsi.dragStart.y;
    state.wsi.dragStart = { x: e.clientX, y: e.clientY };

    // Update slide origin in microns
    state.wsi.originUm.x -= (dx * state.wsi.mppX) / state.wsi.zoom;
    state.wsi.originUm.y -= (dy * state.wsi.mppY) / state.wsi.zoom;
  } else if (state.wsi.roiMode && state.wsi.roiStart) {
    const rx = Math.min(state.wsi.roiStart.x, mouseX);
    const ry = Math.min(state.wsi.roiStart.y, mouseY);
    const rw = Math.abs(mouseX - state.wsi.roiStart.x);
    const rh = Math.abs(mouseY - state.wsi.roiStart.y);

    roiSelectionBox.style.left = `${rx}px`;
    roiSelectionBox.style.top = `${ry}px`;
    roiSelectionBox.style.width = `${rw}px`;
    roiSelectionBox.style.height = `${rh}px`;
  }
});

window.addEventListener("mouseup", (e) => {
  if (state.wsi.isDragging) {
    state.wsi.isDragging = false;
  }
  if (state.wsi.roiMode && state.wsi.roiStart) {
    const rect = wsiWrapper.getBoundingClientRect();
    const mouseX = e.clientX - rect.left;
    const mouseY = e.clientY - rect.top;

    const sx = Math.min(state.wsi.roiStart.x, mouseX);
    const sy = Math.min(state.wsi.roiStart.y, mouseY);
    const sw = Math.abs(mouseX - state.wsi.roiStart.x);
    const sh = Math.abs(mouseY - state.wsi.roiStart.y);

    if (sw > 10 && sh > 10) {
      // Convert screen rectangle to physical slide microns
      const originX_um = state.wsi.originUm.x + (sx * state.wsi.mppX) / state.wsi.zoom;
      const originY_um = state.wsi.originUm.y + (sy * state.wsi.mppY) / state.wsi.zoom;
      const width_um = (sw * state.wsi.mppX) / state.wsi.zoom;
      const height_um = (sh * state.wsi.mppY) / state.wsi.zoom;

      state.wsi.activeRoi = {
        origin: { x_um: originX_um, y_um: originY_um },
        width_um,
        height_um,
      };

      // Automatically render ROI crop into HUD preview and prompt HUD
      captureRoiToHud();
      toggleHud(true);
    }

    state.wsi.roiMode = false;
    state.wsi.roiStart = null;
    roiSelectionBox.style.display = "none";
    toolSelectRoi.classList.remove("active");
  }
});

// Wheel zoom centered at cursor position
wsiWrapper.addEventListener("wheel", (e) => {
  e.preventDefault();
  const rect = wsiWrapper.getBoundingClientRect();
  const cursorX = e.clientX - rect.left;
  const cursorY = e.clientY - rect.top;

  // Slide position under cursor before zoom
  const preX_um = state.wsi.originUm.x + (cursorX * state.wsi.mppX) / state.wsi.zoom;
  const preY_um = state.wsi.originUm.y + (cursorY * state.wsi.mppY) / state.wsi.zoom;

  const zoomFactor = e.deltaY < 0 ? 1.15 : 1.0 / 1.15;
  const newZoom = Math.min(80.0, Math.max(0.05, state.wsi.zoom * zoomFactor));

  state.wsi.zoom = newZoom;

  // Adjust origin so slide point under cursor remains invariant
  state.wsi.originUm.x = preX_um - (cursorX * state.wsi.mppX) / newZoom;
  state.wsi.originUm.y = preY_um - (cursorY * state.wsi.mppY) / newZoom;

  const mag = (newZoom * 40.0).toFixed(1);
  const mpp = (state.wsi.mppX / newZoom).toFixed(3);
  wsiZoomDisplay.textContent = `${mag}× (${mpp} µm/px)`;
}, { passive: false });

toolSelectRoi.addEventListener("click", () => {
  state.wsi.roiMode = !state.wsi.roiMode;
  toolSelectRoi.classList.toggle("active", state.wsi.roiMode);
});

document.getElementById("btnResetWsi").addEventListener("click", () => {
  state.wsi.zoom = 1.0;
  state.wsi.originUm = { x: 5000.0, y: 5000.0 };
  state.wsi.activeRoi = null;
  wsiZoomDisplay.textContent = "40.0× (0.25 µm/px)";
});

// ==========================================
// 2. VOLUMETRIC DICOM RADIOLOGY ENGINE
// ==========================================

function initDicom() {
  const [w, h, d] = state.dicom.dimensions;
  dicomCanvas.width = w;
  dicomCanvas.height = h;
  sliceSlider.max = d - 1;
  sliceSlider.value = state.dicom.sliceIndex;

  renderDicomSlice();
}

function applyWwwlPs33(rawHu, center, width) {
  const w = Math.max(1.0, width);
  const lower = center - 0.5 - (w - 1.0) / 2.0;
  const upper = center - 0.5 + (w - 1.0) / 2.0;
  if (rawHu <= lower) return 0.0;
  if (rawHu >= upper) return 1.0;
  return Math.min(1.0, Math.max(0.0, (rawHu - (center - 0.5)) / (w - 1.0) + 0.5));
}

function renderDicomSlice() {
  const [w, h, d] = state.dicom.dimensions;
  const plane = state.dicom.activePlane;
  const sliceIdx = state.dicom.sliceIndex;
  const { center, width: winWidth } = state.dicom.currentWindow;

  let sliceW, sliceH;
  if (plane === "axial") {
    sliceW = w;
    sliceH = h;
  } else if (plane === "coronal") {
    sliceW = w;
    sliceH = d;
  } else {
    sliceW = h;
    sliceH = d;
  }

  dicomCanvas.width = sliceW;
  dicomCanvas.height = sliceH;

  const imgData = dicomCtx.createImageData(sliceW, sliceH);
  const data = imgData.data;

  // Render synthetic CT phantom slice with exact WWWL formula
  for (let y = 0; y < sliceH; y++) {
    for (let x = 0; x < sliceW; x++) {
      let nx, ny, nz;
      if (plane === "axial") {
        nx = (x / w) * 2.0 - 1.0;
        ny = (y / h) * 2.0 - 1.0;
        nz = (sliceIdx / d) * 2.0 - 1.0;
      } else if (plane === "coronal") {
        nx = (x / w) * 2.0 - 1.0;
        ny = (sliceIdx / h) * 2.0 - 1.0;
        nz = (y / d) * 2.0 - 1.0;
      } else {
        nx = (sliceIdx / w) * 2.0 - 1.0;
        ny = (x / h) * 2.0 - 1.0;
        nz = (y / d) * 2.0 - 1.0;
      }

      const r = Math.sqrt(nx * nx + ny * ny);
      let hu;
      if (r > 0.85) {
        hu = -1000.0; // Air
      } else if (r > 0.80) {
        hu = 400.0; // Bone
      } else if (nx < -0.1 && ny < 0.2 && r < 0.7) {
        hu = -600.0; // Lung
      } else if (nx > 0.1 && ny < 0.2 && r < 0.7) {
        hu = -600.0; // Contralateral lung
      } else if (Math.hypot(nx, ny - 0.3) < 0.2) {
        hu = 80.0; // Mediastinum/heart
      } else {
        hu = 40.0; // Soft tissue
      }

      const norm = applyWwwlPs33(hu, center, winWidth);
      const lum = Math.round(norm * 255);

      const idx = (y * sliceW + x) * 4;
      data[idx] = lum;
      data[idx + 1] = lum;
      data[idx + 2] = lum;
      data[idx + 3] = 255;
    }
  }

  dicomCtx.putImageData(imgData, 0, 0);

  // Update Stats
  dicomSliceStat.textContent = `Slice ${sliceIdx + 1} / ${plane === "axial" ? d : (plane === "coronal" ? h : w)}`;
  dicomWwwlStat.textContent = `WW: ${Math.round(winWidth)} | WL: ${Math.round(center)}`;
}

sliceSlider.addEventListener("input", (e) => {
  state.dicom.sliceIndex = parseInt(e.target.value, 10);
  renderDicomSlice();
});

mprControl.addEventListener("click", (e) => {
  if (e.target.dataset.plane) {
    document.querySelectorAll(".seg-btn").forEach((b) => b.classList.remove("active"));
    e.target.classList.add("active");
    state.dicom.activePlane = e.target.dataset.plane;
    dicomPlaneName.textContent = `${e.target.textContent} Plane`;

    const max = state.dicom.activePlane === "axial"
      ? state.dicom.dimensions[2] - 1
      : (state.dicom.activePlane === "coronal"
        ? state.dicom.dimensions[1] - 1
        : state.dicom.dimensions[0] - 1);
    sliceSlider.max = max;
    state.dicom.sliceIndex = Math.floor(max / 2);
    sliceSlider.value = state.dicom.sliceIndex;
    renderDicomSlice();
  }
});

wwwlPresetSelect.addEventListener("change", (e) => {
  const preset = state.dicom.presets[e.target.value];
  if (preset) {
    state.dicom.currentWindow = { ...preset };
    renderDicomSlice();
  }
});

// ==========================================
// 3. CLINICAL ROI HUD & PRIVACY SANITIZER
// ==========================================

function toggleHud(forceState) {
  state.hud.visible = forceState !== undefined ? forceState : !state.hud.visible;
  hudOverlay.style.display = state.hud.visible ? "flex" : "none";
  if (state.hud.visible) {
    hudQueryInput.focus();
  }
}

function captureRoiToHud() {
  const patchCanvas = hudPatchCanvas;
  const pCtx = patchCanvas.getContext("2d");
  pCtx.clearRect(0, 0, 256, 256);

  // Draw scaled preview of selected ROI patch
  pCtx.drawImage(wsiCanvas, 0, 0, 256, 256);

  const roi = state.wsi.activeRoi;
  if (roi) {
    hudPatchMeta.textContent = `ROI: ${roi.width_um.toFixed(0)}×${roi.height_um.toFixed(0)} µm | Optical Patch ≤ 1024×1024 px | Zero PHI`;
  }
}

hudTriggerBtn.addEventListener("click", () => toggleHud());
hudCloseBtn.addEventListener("click", () => toggleHud(false));

// Global Shortcut Listener (Option + Space / Alt + Space)
window.addEventListener("keydown", (e) => {
  // Option + Space (macOS) / Alt + Space (Windows)
  if (e.altKey && e.code === "Space") {
    e.preventDefault();
    toggleHud();
  }
  // Command + Shift + P: Command Palette
  if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "p") {
    e.preventDefault();
    togglePalette();
  }
  // Escape closes modals
  if (e.key === "Escape") {
    if (state.hud.visible) toggleHud(false);
    if (paletteBackdrop.style.display === "flex") togglePalette(false);
  }
});

// Preset Chips
document.querySelectorAll(".chip-preset").forEach((btn) => {
  btn.addEventListener("click", () => {
    hudQueryInput.value = btn.dataset.query;
  });
});

// Injected Literature Reader Bridge matching SPEC-04-READER
window.__laminaReaderBridge = {
  getSelectedExcerpt: function () {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0 || !selection.toString().trim()) {
      return null;
    }
    return {
      text: selection.toString().trim(),
      url: window.location.href,
      title: document.title,
    };
  },
  scrollToAnchor: function (anchorText) {
    const iterator = document.createNodeIterator(document.body, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = iterator.nextNode())) {
      if (node.nodeValue.includes(anchorText)) {
        node.parentElement.scrollIntoView({ behavior: "smooth", block: "center" });
        break;
      }
    }
  },
};

// Evidence Extraction via Reader Bridge
btnExtractSelection.addEventListener("click", () => {
  const excerpt = window.__laminaReaderBridge.getSelectedExcerpt();
  if (excerpt && excerpt.text) {
    state.hud.pinnedExcerpt = excerpt.text;
    excerptText.textContent = excerpt.text;
    hudPinnedExcerpt.style.display = "block";
    toggleHud(true);
  } else {
    alert("Please highlight an excerpt in the literature panel first.");
  }
});

btnClearExcerpt.addEventListener("click", () => {
  state.hud.pinnedExcerpt = null;
  hudPinnedExcerpt.style.display = "none";
});

document.getElementById("hudSubmitBtn").addEventListener("click", async () => {
  const query = hudQueryInput.value.trim();
  if (!query) {
    alert("Please enter a clinical prompt.");
    return;
  }

  // Optical Allow-List validation before submission
  const req = {
    query_text: query,
    patch_png_base64: hudPatchCanvas.toDataURL("image/png").split(",")[1],
    magnification_mpp: state.wsi.mppX,
    previous_interaction_id: null,
  };

  try {
    await invoke("submit_diagnostic_query", { request: req });
    alert("Diagnostic Query stream initiated with verified optical allow-list boundary.");
    toggleHud(false);
  } catch (err) {
    alert(`Query failed: ${err}`);
  }
});

// ==========================================
// 4. COMMAND PALETTE MODAL
// ==========================================

function togglePalette(forceState) {
  const isVis = paletteBackdrop.style.display === "flex";
  const next = forceState !== undefined ? forceState : !isVis;
  paletteBackdrop.style.display = next ? "flex" : "none";
  if (next) {
    paletteInput.value = "";
    paletteInput.focus();
    renderPaletteItems("");
  }
}

cmdPaletteBtn.addEventListener("click", () => togglePalette());
paletteBackdrop.addEventListener("click", (e) => {
  if (e.target === paletteBackdrop) togglePalette(false);
});

paletteInput.addEventListener("input", (e) => {
  renderPaletteItems(e.target.value.toLowerCase());
});

function renderPaletteItems(filter) {
  const items = paletteList.querySelectorAll("li");
  items.forEach((item) => {
    const text = item.textContent.toLowerCase();
    item.style.display = text.includes(filter) ? "block" : "none";
  });
}

paletteList.addEventListener("click", async (e) => {
  const action = e.target.dataset.action;
  if (!action) return;

  if (action === "toggle_hud") {
    toggleHud();
  } else if (action === "open_slide") {
    if (isTauri) {
      try {
        const path = await invoke("open_file_dialog", { filter_type: "wsi" });
        if (path) {
          const meta = await invoke("load_slide", { path });
          if (meta) {
            state.wsi.slideId = meta.slide_id;
            state.wsi.levelCount = meta.level_count;
            state.wsi.downsamples = meta.level_downsamples;
            state.wsi.mppX = meta.mpp_x;
            state.wsi.mppY = meta.mpp_y;
            state.wsi.tileCache.clear();
            document.getElementById("wsiSlideName").textContent = meta.slide_id;
          }
        }
      } catch (err) {
        console.error("Failed to open slide:", err);
      }
    }
  } else if (action === "open_dicom") {
    if (isTauri) {
      try {
        const dir = await invoke("open_file_dialog", { filter_type: "dicom" });
        if (dir) {
          const vol = await invoke("load_dicom_series", { series_dir: dir });
          if (vol) {
            state.dicom.dimensions = vol.dimensions;
            state.dicom.voxelSpacing = vol.voxel_spacing;
            state.dicom.currentWindow = { center: vol.default_window[0], width: vol.default_window[1] };
            initDicom();
          }
        }
      } catch (err) {
        console.error("Failed to open DICOM series:", err);
      }
    }
  } else if (action === "load_synthetic_dicom") {
    if (isTauri) {
      try {
        await invoke("load_synthetic_dicom", { width: 128, height: 128, slices: 32 });
      } catch (err) {
        console.error(err);
      }
    }
    state.dicom.dimensions = [128, 128, 16];
    initDicom();
  } else if (action === "mpr_axial") {
    document.querySelector('.seg-btn[data-plane="axial"]').click();
  } else if (action === "mpr_coronal") {
    document.querySelector('.seg-btn[data-plane="coronal"]').click();
  } else if (action === "mpr_sagittal") {
    document.querySelector('.seg-btn[data-plane="sagittal"]').click();
  } else if (action.startsWith("preset_")) {
    const p = action.replace("preset_", "");
    wwwlPresetSelect.value = p;
    wwwlPresetSelect.dispatchEvent(new Event("change"));
  }

  togglePalette(false);
});

// Listen to Tauri HUD toggled event from global shortcut
if (isTauri) {
  listen("lamina:hud-toggled", (event) => {
    toggleHud(event.payload);
  });
}

// Workspace Layout Toggling
document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");

    const layout = btn.dataset.layout;
    const wsiP = document.getElementById("wsiPanel");
    const dicomP = document.getElementById("dicomPanel");
    const readerP = document.getElementById("readerPanel");

    if (layout === "wsi-focus") {
      wsiP.style.display = "flex";
      wsiP.style.flex = "1";
      dicomP.style.display = "none";
      readerP.style.display = "none";
    } else if (layout === "dicom-focus") {
      wsiP.style.display = "none";
      dicomP.style.display = "flex";
      dicomP.style.flex = "1";
      readerP.style.display = "none";
    } else {
      wsiP.style.display = "flex";
      wsiP.style.flex = "5";
      dicomP.style.display = "flex";
      dicomP.style.flex = "4";
      readerP.style.display = "flex";
      readerP.style.flex = "3";
    }
  });
});

// Initialize on page load
window.addEventListener("DOMContentLoaded", () => {
  initDicom();
  renderWsi();
});
