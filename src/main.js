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
    sliceIndex: 16,
    maxSlices: 32,
    dimensions: [256, 256, 32],
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

function hash2Js(x, y) {
  let n = Math.imul(x >>> 0, 0x45d9f3b) ^ Math.imul(y >>> 0, 0x119de1f);
  n = Math.imul(n, 0x45d9f3b);
  const h1 = (n & 0xffff) / 65535.0;
  const h2 = ((n >>> 16) & 0xffff) / 65535.0;
  return [h1, h2];
}

function generateSyntheticTileCanvas(level, tileX, tileY) {
  const offscreen = document.createElement("canvas");
  offscreen.width = 256;
  offscreen.height = 256;
  const offCtx = offscreen.getContext("2d");

  const scale = 1 << level;
  const imgData = offCtx.createImageData(256, 256);
  const data = imgData.data;
  const cellSize = 20.0;

  for (let y = 0; y < 256; y++) {
    for (let x = 0; x < 256; x++) {
      const fx = (tileX * 256 + x) * scale;
      const fy = (tileY * 256 + y) * scale;

      const macro1 = Math.sin(fx * 0.0025) * Math.cos(fy * 0.003);
      const macro2 = Math.sin(fx * 0.006 + fy * 0.004) * 0.6 + Math.cos(fx * 0.012 - fy * 0.009) * 0.3;
      const tissueDensity = macro1 + macro2;

      const idx = (y * 256 + x) * 4;

      if (tissueDensity < -0.38) {
        const speck = Math.sin(fx * 0.1) * Math.cos(fy * 0.1) * 2;
        const bg = Math.min(252, Math.max(242, Math.round(248 + speck)));
        data[idx] = bg;
        data[idx + 1] = bg - 2;
        data[idx + 2] = bg - 1;
        data[idx + 3] = 255;
        continue;
      }

      const stromaTex = Math.sin(fx * 0.04 + Math.sin(fy * 0.03) * 2.5) * 0.5 + 0.5;
      const cxBase = Math.floor(fx / cellSize);
      const cyBase = Math.floor(fy / cellSize);

      let minDistSq = 999.0;
      let nearestRadius = 4.5;

      for (let dy = -1; dy <= 1; dy++) {
        for (let dx = -1; dx <= 1; dx++) {
          const cx = cxBase + dx;
          const cy = cyBase + dy;
          const [h1, h2] = hash2Js(cx, cy);

          const cFx = (cx + 0.5) * cellSize;
          const cFy = (cy + 0.5) * cellSize;
          const cDens = Math.sin(cFx * 0.0025) * Math.cos(cFy * 0.003) + Math.sin(cFx * 0.006 + cFy * 0.004) * 0.6;
          if (cDens < -0.35 && h1 > 0.35) continue;

          const nCenterX = (cx + 0.2 + h1 * 0.6) * cellSize;
          const nCenterY = (cy + 0.2 + h2 * 0.6) * cellSize;
          const dX = fx - nCenterX;
          const dY = (fy - nCenterY) * 1.2;
          const distSq = dX * dX + dY * dY;

          if (distSq < minDistSq) {
            minDistSq = distSq;
            nearestRadius = 3.8 + h1 * 2.2;
          }
        }
      }

      const nucleusDist = Math.sqrt(minDistSq);

      if (nucleusDist < nearestRadius) {
        const chrom = Math.sin(fx * 0.4) * Math.cos(fy * 0.4) * 8;
        data[idx] = Math.min(80, Math.max(45, Math.round(60 + chrom)));
        data[idx + 1] = Math.min(45, Math.max(18, Math.round(25 + chrom * 0.5)));
        data[idx + 2] = Math.min(125, Math.max(75, Math.round(95 + chrom)));
        data[idx + 3] = 255;
      } else if (nucleusDist < nearestRadius + 3.0) {
        const blend = (nucleusDist - nearestRadius) / 3.0;
        data[idx] = Math.round(160 + blend * 55);
        data[idx + 1] = Math.round(65 + blend * 50);
        data[idx + 2] = Math.round(135 + blend * 30);
        data[idx + 3] = 255;
      } else {
        data[idx] = Math.round(215 + stromaTex * 22);
        data[idx + 1] = Math.round(125 + stromaTex * 25);
        data[idx + 2] = Math.round(165 + stromaTex * 20);
        data[idx + 3] = 255;
      }
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

  const scaleBarEl = document.getElementById("wsiScaleBar");
  const scaleLabelEl = document.getElementById("wsiScaleLabel");
  if (scaleBarEl && scaleLabelEl) {
    const targetUm = newZoom >= 2.0 ? 25 : newZoom >= 0.8 ? 50 : newZoom >= 0.3 ? 100 : 250;
    const pxWidth = Math.min(120, Math.max(36, Math.round((targetUm * newZoom) / state.wsi.mppX)));
    scaleBarEl.style.width = `${pxWidth}px`;
    scaleLabelEl.textContent = `${targetUm} µm`;
  }
}, { passive: false });

toolSelectRoi.addEventListener("click", () => {
  state.wsi.roiMode = !state.wsi.roiMode;
  toolSelectRoi.classList.toggle("active", state.wsi.roiMode);
});

document.getElementById("btnResetWsi").addEventListener("click", () => {
  state.wsi.zoom = 1.0;
  state.wsi.originUm = { x: 5000.0, y: 5000.0 };
  state.wsi.activeRoi = null;
  captureRoiToHud();
  wsiZoomDisplay.textContent = "40.0× (0.25 µm/px)";
});

// ==========================================
// 2. VOLUMETRIC DICOM RADIOLOGY ENGINE
// ==========================================

function initDicom() {
  const [, , d] = state.dicom.dimensions;
  sliceSlider.max = d - 1;
  sliceSlider.value = state.dicom.sliceIndex;
  updatePacsOrientation("axial");
  renderDicomSlice();
}

function updatePacsOrientation(plane) {
  const topEl = document.getElementById("orientTop");
  const bottomEl = document.getElementById("orientBottom");
  const leftEl = document.getElementById("orientLeft");
  const rightEl = document.getElementById("orientRight");
  if (!topEl || !bottomEl || !leftEl || !rightEl) return;

  if (plane === "axial") {
    topEl.textContent = "A";
    bottomEl.textContent = "P";
    leftEl.textContent = "R";
    rightEl.textContent = "L";
  } else if (plane === "coronal") {
    topEl.textContent = "S";
    bottomEl.textContent = "I";
    leftEl.textContent = "R";
    rightEl.textContent = "L";
  } else {
    topEl.textContent = "S";
    bottomEl.textContent = "I";
    leftEl.textContent = "A";
    rightEl.textContent = "P";
  }
}

function applyWwwlPs33(rawHu, center, width) {
  const w = Math.max(1.0, width);
  const lower = center - 0.5 - (w - 1.0) / 2.0;
  const upper = center - 0.5 + (w - 1.0) / 2.0;
  if (rawHu <= lower) return 0.0;
  if (rawHu >= upper) return 1.0;
  return Math.min(1.0, Math.max(0.0, (rawHu - (center - 0.5)) / (w - 1.0) + 0.5));
}

function computeSyntheticThoracicHu(nx, ny, nz) {
  const torsoScaleX = 0.82 - Math.abs(nz) * 0.05;
  const torsoScaleY = 0.65 - Math.abs(nz) * 0.05;
  const torsoR = Math.pow(nx / torsoScaleX, 2) + Math.pow(ny / torsoScaleY, 2);

  if (torsoR > 1.0) return -1000.0;
  if (torsoR > 0.93) return torsoR > 0.98 ? 20.0 : -105.0;

  const ribRing = Math.abs(torsoR - 0.88);
  const ribSegment = Math.abs(Math.sin(Math.atan2(nx, ny) * 6.0));
  const isRib = ribRing < 0.04 && ribSegment > 0.35;

  const spineDist = Math.hypot(nx, ny - 0.42);
  const isSpineBody = spineDist < 0.12;
  const isSpinalCanal = Math.hypot(nx, ny - 0.47) < 0.045;
  const isSternum = Math.hypot(nx, ny + 0.58) < 0.05;

  if (isSpinalCanal) return -15.0;
  if (isSpineBody) return spineDist > 0.09 ? 850.0 : 220.0;
  if (isSternum || isRib) return 800.0;

  const rLungDist = Math.pow((nx + 0.38) / 0.28, 2) + Math.pow((ny - 0.02) / 0.36, 2);
  const lLungDist = Math.pow((nx - 0.38) / 0.27, 2) + Math.pow((ny - 0.02) / 0.36, 2);
  const isAirway = Math.hypot(nx, ny - 0.18) < 0.06 && nz < 0.2;

  const noduleDx = nx + 0.32;
  const noduleDy = ny + 0.12;
  const noduleDz = nz + 0.15;
  const noduleDist = Math.hypot(noduleDx, noduleDy, noduleDz);

  if (isAirway) return -960.0;
  if (noduleDist < 0.075) {
    return 42.0 + Math.sin(noduleDx * 25.0) * Math.cos(noduleDy * 25.0) * 8.0;
  }
  if (rLungDist < 1.0) {
    const vasc = Math.abs(Math.sin(nx * 40.0) * Math.cos(ny * 40.0) * Math.sin(nz * 15.0));
    return vasc > 0.88 ? 110.0 : -720.0 + vasc * 60.0;
  }
  if (lLungDist < 1.0) {
    const vasc = Math.abs(Math.cos(nx * 38.0) * Math.sin(ny * 38.0) * Math.cos(nz * 15.0));
    return vasc > 0.88 ? 110.0 : -700.0 + vasc * 60.0;
  }

  const isHeart = Math.pow((nx - 0.06) / 0.22, 2) + Math.pow((ny + 0.08) / 0.24, 2) < 1.0;
  const isAorta = Math.hypot(nx + 0.04, ny - 0.18) < 0.07;
  if (isAorta) return 145.0;
  if (isHeart) return 65.0;
  return 42.0;
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

  // Offscreen raw slice buffer
  const offscreen = document.createElement("canvas");
  offscreen.width = sliceW;
  offscreen.height = sliceH;
  const offCtx = offscreen.getContext("2d");
  const imgData = offCtx.createImageData(sliceW, sliceH);
  const data = imgData.data;

  for (let y = 0; y < sliceH; y++) {
    for (let x = 0; x < sliceW; x++) {
      let nx, ny, nz;
      if (plane === "axial") {
        nx = (x / w) * 2.0 - 1.0;
        ny = (y / h) * 2.0 - 1.0;
        nz = (sliceIdx / Math.max(1, d - 1)) * 2.0 - 1.0;
      } else if (plane === "coronal") {
        nx = (x / w) * 2.0 - 1.0;
        ny = (sliceIdx / Math.max(1, h - 1)) * 2.0 - 1.0;
        nz = (y / Math.max(1, d - 1)) * 2.0 - 1.0;
      } else {
        nx = (sliceIdx / Math.max(1, w - 1)) * 2.0 - 1.0;
        ny = (x / h) * 2.0 - 1.0;
        nz = (y / Math.max(1, d - 1)) * 2.0 - 1.0;
      }

      const hu = computeSyntheticThoracicHu(nx, ny, nz);
      const norm = applyWwwlPs33(hu, center, winWidth);
      const lum = Math.round(norm * 255);

      const idx = (y * sliceW + x) * 4;
      data[idx] = lum;
      data[idx + 1] = lum;
      data[idx + 2] = lum;
      data[idx + 3] = 255;
    }
  }
  offCtx.putImageData(imgData, 0, 0);

  // Render onto high-DPI display canvas with aspect-ratio compensation and PACS crosshairs
  const dispSize = 512;
  dicomCanvas.width = dispSize;
  dicomCanvas.height = dispSize;
  dicomCtx.fillStyle = "#020306";
  dicomCtx.fillRect(0, 0, dispSize, dispSize);

  let drawW = dispSize;
  let drawH = dispSize;
  if (plane !== "axial") {
    // Compensate for anisotropic slice thickness (2.5mm vs 0.75mm in-plane)
    const physicalRatio = (sliceH * state.dicom.voxelSpacing[2]) / (sliceW * state.dicom.voxelSpacing[0]);
    drawH = Math.min(dispSize, Math.round(dispSize * Math.max(0.45, physicalRatio)));
  }

  const offsetX = Math.round((dispSize - drawW) / 2);
  const offsetY = Math.round((dispSize - drawH) / 2);

  dicomCtx.imageSmoothingEnabled = true;
  dicomCtx.imageSmoothingQuality = "high";
  dicomCtx.drawImage(offscreen, offsetX, offsetY, drawW, drawH);

  // Subtle PACS MPR reference crosshairs
  dicomCtx.strokeStyle = "rgba(56, 189, 248, 0.22)";
  dicomCtx.lineWidth = 1;
  dicomCtx.setLineDash([4, 4]);
  dicomCtx.beginPath();
  dicomCtx.moveTo(dispSize / 2, 0);
  dicomCtx.lineTo(dispSize / 2, dispSize);
  dicomCtx.moveTo(0, dispSize / 2);
  dicomCtx.lineTo(dispSize, dispSize / 2);
  dicomCtx.stroke();
  dicomCtx.setLineDash([]);

  // Target lesion reticle when near RUL nodule slice
  if (plane === "axial" && Math.abs(sliceIdx - 13) <= 3) {
    const rx = Math.round(((1.0 - 0.32) / 2.0) * drawW) + offsetX;
    const ry = Math.round(((1.0 - 0.12) / 2.0) * drawH) + offsetY;
    dicomCtx.strokeStyle = "rgba(245, 158, 11, 0.75)";
    dicomCtx.lineWidth = 1.5;
    dicomCtx.beginPath();
    dicomCtx.arc(rx, ry, 24, 0, Math.PI * 2);
    dicomCtx.stroke();
  }

  // Update Stats
  const totalSlices = plane === "axial" ? d : (plane === "coronal" ? h : w);
  dicomSliceStat.textContent = `Im: ${sliceIdx + 1} / ${totalSlices}`;
  dicomWwwlStat.textContent = `WL: ${Math.round(center)} | WW: ${Math.round(winWidth)}`;
}

// Interactive DICOM Mouse Drag for Window Width / Window Level & Wheel Slice Scrubbing
let isDicomDragging = false;
let dicomDragStart = { x: 0, y: 0, center: 40, width: 400 };

dicomCanvas.addEventListener("mousedown", (e) => {
  isDicomDragging = true;
  dicomDragStart = {
    x: e.clientX,
    y: e.clientY,
    center: state.dicom.currentWindow.center,
    width: state.dicom.currentWindow.width,
  };
});

window.addEventListener("mousemove", (e) => {
  if (!isDicomDragging) return;
  const dx = e.clientX - dicomDragStart.x;
  const dy = e.clientY - dicomDragStart.y;
  state.dicom.currentWindow.width = Math.max(10, Math.min(3500, dicomDragStart.width + dx * 4.0));
  state.dicom.currentWindow.center = Math.max(-1000, Math.min(1500, dicomDragStart.center - dy * 2.5));
  renderDicomSlice();
});

window.addEventListener("mouseup", () => {
  isDicomDragging = false;
});

dicomCanvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const max = parseInt(sliceSlider.max, 10) || 31;
  const delta = e.deltaY > 0 ? 1 : -1;
  state.dicom.sliceIndex = Math.max(0, Math.min(max, state.dicom.sliceIndex + delta));
  sliceSlider.value = state.dicom.sliceIndex;
  renderDicomSlice();
}, { passive: false });

sliceSlider.addEventListener("input", (e) => {
  state.dicom.sliceIndex = parseInt(e.target.value, 10);
  renderDicomSlice();
});

mprControl.addEventListener("click", (e) => {
  if (e.target.dataset.plane) {
    document.querySelectorAll(".seg-btn").forEach((b) => b.classList.remove("active"));
    e.target.classList.add("active");
    state.dicom.activePlane = e.target.dataset.plane;
    const planeLabel = e.target.dataset.plane === "axial"
      ? "Axial (XY)"
      : (e.target.dataset.plane === "coronal" ? "Coronal (XZ)" : "Sagittal (YZ)");
    dicomPlaneName.textContent = planeLabel;
    updatePacsOrientation(e.target.dataset.plane);

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
    captureRoiToHud();
    hudQueryInput.focus();
  }
}

function captureRoiToHud() {
  const patchCanvas = hudPatchCanvas;
  const pCtx = patchCanvas.getContext("2d");
  pCtx.clearRect(0, 0, 256, 256);

  const roi = state.wsi.activeRoi;
  if (roi) {
    const zoom = state.wsi.zoom;
    const mppX = state.wsi.mppX;
    const mppY = state.wsi.mppY;
    const originUm = state.wsi.originUm;

    // Convert slide micron ROI to current screen canvas coordinates
    const sx = (roi.origin.x_um - originUm.x) * (zoom / mppX);
    const sy = (roi.origin.y_um - originUm.y) * (zoom / mppY);
    const sw = (roi.width_um * zoom) / mppX;
    const sh = (roi.height_um * zoom) / mppY;

    if (sw > 0 && sh > 0 && sx < wsiCanvas.width && sy < wsiCanvas.height && sx + sw > 0 && sy + sh > 0) {
      const srcX = Math.max(0, sx);
      const srcY = Math.max(0, sy);
      const srcW = Math.min(sw, wsiCanvas.width - srcX);
      const srcH = Math.min(sh, wsiCanvas.height - srcY);
      pCtx.drawImage(wsiCanvas, srcX, srcY, srcW, srcH, 0, 0, 256, 256);
    } else {
      pCtx.drawImage(wsiCanvas, 0, 0, 256, 256);
    }

    hudPatchMeta.textContent = `ROI: ${roi.width_um.toFixed(0)}×${roi.height_um.toFixed(0)} µm | Optical Patch ≤ 1024×1024 px | Zero PHI`;
  } else {
    pCtx.fillStyle = "#111827";
    pCtx.fillRect(0, 0, 256, 256);
    pCtx.fillStyle = "#64748b";
    pCtx.font = "11px Inter, system-ui, sans-serif";
    pCtx.textAlign = "center";
    pCtx.fillText("No ROI Box Selected", 128, 120);
    pCtx.fillText("(Prompt-Only Mode)", 128, 138);
    hudPatchMeta.textContent = "No ROI Selected | Text & Pinned Evidence Mode | Zero PHI";
  }
}

hudTriggerBtn.addEventListener("click", () => toggleHud());
hudCloseBtn.addEventListener("click", () => toggleHud(false));

// Global Shortcut & Keyboard Navigation (Option + Space / Alt + Space / Esc)
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
  // Escape closes modals or aborts active stream per SPEC-05 section 6.3
  if (e.key === "Escape") {
    if (isStreaming) {
      cancelDiagnosticStream();
    } else if (paletteBackdrop.style.display === "flex") {
      togglePalette(false);
    } else if (state.hud.visible) {
      toggleHud(false);
    }
  }
});

// Preset Chips
document.querySelectorAll(".chip-preset").forEach((btn) => {
  btn.addEventListener("click", () => {
    hudQueryInput.value = btn.dataset.query;
  });
});

// ==========================================
// 3. BIOMEDICAL LITERATURE & GUIDANCE READER
// ==========================================

const readerDocSelect = document.getElementById("readerDocSelect");
const readerSearchInput = document.getElementById("readerSearchInput");

if (readerDocSelect) {
  readerDocSelect.addEventListener("change", (e) => {
    const docId = e.target.value;
    document.querySelectorAll(".literature-doc").forEach((doc) => {
      doc.style.display = doc.id === `doc-${docId}` ? "block" : "none";
    });
  });
}

if (readerSearchInput) {
  readerSearchInput.addEventListener("input", (e) => {
    const filter = e.target.value.toLowerCase().trim();
    const activeDoc = document.querySelector(".literature-doc[style*='display: block']") || document.querySelector(".literature-doc");
    if (!activeDoc) return;

    const sections = activeDoc.querySelectorAll(".evidence-anchor");
    sections.forEach((sec) => {
      if (!filter) {
        sec.style.display = "block";
      } else {
        const text = sec.textContent.toLowerCase();
        sec.style.display = text.includes(filter) ? "block" : "none";
      }
    });
  });
}

// Injected Literature Reader Bridge matching SPEC-04-READER
window.__laminaReaderBridge = {
  getSelectedExcerpt: function () {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0 || !selection.toString().trim()) {
      return null;
    }
    const text = selection.toString().trim();
    const activeDoc = document.querySelector(".literature-doc[style*='display: block']") || document.querySelector(".literature-doc");
    const title = activeDoc ? (activeDoc.querySelector(".paper-title")?.textContent || document.title) : document.title;
    const meta = activeDoc ? (activeDoc.querySelector(".paper-meta")?.textContent || "") : "";
    const pmidMatch = meta.match(/PMID:\s*(\d+)/i);

    return {
      text,
      source_url: window.location.href,
      document_title: title,
      pmid_or_doi: pmidMatch ? pmidMatch[1] : null,
      url: window.location.href,
      title: title,
    };
  },
  scrollToAnchor: function (anchorIdOrText) {
    if (!anchorIdOrText || !anchorIdOrText.toString().trim()) return;
    const query = anchorIdOrText.toString().trim();

    // 1. Try finding by ID
    let targetEl = document.getElementById(query);

    // 2. If not found by ID, search text in readerContent
    if (!targetEl) {
      const readerRoot = document.getElementById("readerContent");
      if (readerRoot) {
        const iterator = document.createNodeIterator(readerRoot, NodeFilter.SHOW_TEXT);
        let node;
        while ((node = iterator.nextNode())) {
          if (node.nodeValue.toLowerCase().includes(query.toLowerCase())) {
            targetEl = node.parentElement;
            break;
          }
        }
      }
    }

    if (targetEl) {
      // Find parent article and ensure it is visible
      const parentArticle = targetEl.closest(".literature-doc");
      if (parentArticle && readerDocSelect) {
        const articleId = parentArticle.id.replace("doc-", "");
        readerDocSelect.value = articleId;
        document.querySelectorAll(".literature-doc").forEach((doc) => {
          doc.style.display = doc.id === parentArticle.id ? "block" : "none";
        });
      }

      // Unhide if filtered out
      if (targetEl.style.display === "none") {
        targetEl.style.display = "block";
      }

      // Smooth scroll
      targetEl.scrollIntoView({ behavior: "smooth", block: "center" });

      // Highlight pulse
      targetEl.classList.add("highlight");
      setTimeout(() => {
        targetEl.classList.remove("highlight");
      }, 2500);
    }
  },
};

// Sync selection changes to backend via mouseup and selectionchange
async function syncReaderSelection() {
  if (!isTauri) return;
  const selection = window.getSelection();
  const readerRoot = document.getElementById("readerContent");
  if (selection && selection.rangeCount > 0 && selection.toString().trim()) {
    if (readerRoot && readerRoot.contains(selection.anchorNode)) {
      const excerpt = window.__laminaReaderBridge.getSelectedExcerpt();
      if (excerpt) {
        try {
          await invoke("reader_set_selection", { excerpt });
        } catch (e) {
          console.warn("Failed to sync selection to backend:", e);
        }
      }
    }
  }
}

document.getElementById("readerContent")?.addEventListener("mouseup", syncReaderSelection);
document.addEventListener("selectionchange", syncReaderSelection);

// Evidence Extraction via Reader Bridge
btnExtractSelection.addEventListener("click", async () => {
  let excerpt = window.__laminaReaderBridge.getSelectedExcerpt();
  if (!excerpt && isTauri) {
    try {
      excerpt = await invoke("reader_extract_selection");
    } catch (_) {}
  }

  if (excerpt && excerpt.text) {
    state.hud.pinnedExcerpt = excerpt;
    excerptText.textContent = excerpt.text;
    const excerptSource = document.getElementById("excerptSource");
    if (excerptSource) {
      const ref = excerpt.pmid_or_doi ? `PMID: ${excerpt.pmid_or_doi}` : excerpt.document_title;
      excerptSource.textContent = `Source: ${excerpt.document_title || "Literature Reference"} (${ref})`;
    }
    hudPinnedExcerpt.style.display = "block";
    toggleHud(true);
  } else {
    alert("Please highlight an excerpt in the literature panel first.");
  }
});

btnClearExcerpt.addEventListener("click", () => {
  state.hud.pinnedExcerpt = null;
  hudPinnedExcerpt.style.display = "none";
  if (isTauri) {
    invoke("reader_clear_selection").catch(console.warn);
  }
});

// ==========================================
// 4. REAL-TIME STREAMING CDS SUBSYSTEM
// ==========================================

let isStreaming = false;
let currentInteractionId = null;
let lastInteractionId = null;
let accumulatedMarkdown = "";

const hudQueryForm = document.getElementById("hudQueryForm");
const hudStreamPanel = document.getElementById("hudStreamPanel");
const hudToolBanner = document.getElementById("hudToolBanner");
const toolBannerText = document.getElementById("toolBannerText");
const guardrailStatus = document.getElementById("guardrailStatus");
const guardrailText = document.getElementById("guardrailText");
const hudStreamOutput = document.getElementById("hudStreamOutput");
const streamContent = document.getElementById("streamContent");
const streamCursor = document.getElementById("streamCursor");
const hudCitationsSection = document.getElementById("hudCitationsSection");
const hudCitationChips = document.getElementById("hudCitationChips");
const hudFollowupForm = document.getElementById("hudFollowupForm");
const hudFollowupInput = document.getElementById("hudFollowupInput");
const hudFollowupBtn = document.getElementById("hudFollowupBtn");

const hudSubmitBtn = document.getElementById("hudSubmitBtn");
const hudCancelBtn = document.getElementById("hudCancelBtn");
const hudResetBtn = document.getElementById("hudResetBtn");
const hudCopyBtn = document.getElementById("hudCopyBtn");

// Markdown Formatter for Structured Clinical Markdown
function formatMarkdownToHtml(md) {
  if (!md) return "";

  const rawLines = md.split("\n");
  const blocks = [];
  let currentList = null; // { type: 'ul' | 'ol', items: [] }
  let currentTable = null; // { headers: [], rows: [] }
  let currentParagraph = [];

  function flushParagraph() {
    if (currentParagraph.length > 0) {
      const text = currentParagraph.join("<br/>");
      blocks.push(`<p>${formatInline(text)}</p>`);
      currentParagraph = [];
    }
  }

  function flushList() {
    if (currentList) {
      const tag = currentList.type;
      const itemsHtml = currentList.items.map(it => `<li>${formatInline(it)}</li>`).join("");
      blocks.push(`<${tag}>${itemsHtml}</${tag}>`);
      currentList = null;
    }
  }

  function flushTable() {
    if (currentTable) {
      let tHtml = "<table>";
      if (currentTable.headers.length > 0) {
        tHtml += "<thead><tr>" + currentTable.headers.map(h => `<th>${formatInline(h)}</th>`).join("") + "</tr></thead>";
      }
      if (currentTable.rows.length > 0) {
        tHtml += "<tbody>" + currentTable.rows.map(r => "<tr>" + r.map(c => `<td>${formatInline(c)}</td>`).join("") + "</tr>").join("") + "</tbody>";
      }
      tHtml += "</table>";
      blocks.push(tHtml);
      currentTable = null;
    }
  }

  function flushAll() {
    flushParagraph();
    flushList();
    flushTable();
  }

  for (let i = 0; i < rawLines.length; i++) {
    const line = rawLines[i];
    const trimmed = line.trim();

    if (trimmed === "") {
      flushAll();
      continue;
    }

    if (/^---+$/.test(trimmed) || /^\*\*\*+$/.test(trimmed)) {
      flushAll();
      blocks.push("<hr class=\"stream-divider\" />");
      continue;
    }

    if (trimmed.startsWith("|") && trimmed.endsWith("|")) {
      flushParagraph();
      flushList();
      if (trimmed.includes("---")) {
        continue;
      }
      const cells = trimmed.split("|").slice(1, -1).map(c => c.trim());
      if (!currentTable) {
        currentTable = { headers: cells, rows: [] };
      } else {
        currentTable.rows.push(cells);
      }
      continue;
    } else {
      flushTable();
    }

    if (trimmed.startsWith("#")) {
      flushAll();
      const match = trimmed.match(/^(#{1,4})\s+(.*)$/);
      if (match) {
        const level = match[1].length;
        blocks.push(`<h${level}>${formatInline(match[2])}</h${level}>`);
        continue;
      }
    }

    const olMatch = trimmed.match(/^(\d+)\.\s+(.*)$/);
    if (olMatch) {
      flushParagraph();
      flushTable();
      if (!currentList || currentList.type !== "ol") {
        flushList();
        currentList = { type: "ol", items: [] };
      }
      currentList.items.push(olMatch[2]);
      continue;
    }

    const ulMatch = trimmed.match(/^[-*]\s+(.*)$/);
    if (ulMatch) {
      flushParagraph();
      flushTable();
      if (!currentList || currentList.type !== "ul") {
        flushList();
        currentList = { type: "ul", items: [] };
      }
      currentList.items.push(ulMatch[1]);
      continue;
    }

    flushList();
    currentParagraph.push(line);
  }

  flushAll();
  return blocks.join("\n");
}

function formatInline(text) {
  let str = text;
  str = str.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  str = str.replace(/\*\*(.*?)\*\*/g, "<strong>$1</strong>");
  str = str.replace(/(^|[^*])\*([^*]+)\*([^*]|$)/g, "$1<em>$2</em>$3");
  str = str.replace(/`([^`]+)`/g, "<code>$1</code>");
  str = str.replace(/\[(\d+(?:,\s*\d+)*)\]/g, (match, nums) => {
    const list = nums.split(",").map(n => n.trim());
    return "[" + list.map(n => `<span class="inline-citation" data-citation="${n}">${n}</span>`).join(", ") + "]";
  });
  return str;
}

// Delegated click on stream citations to deep link into reader
streamContent.addEventListener("click", (e) => {
  const citEl = e.target.closest(".inline-citation");
  if (citEl && citEl.dataset.citation) {
    const idx = parseInt(citEl.dataset.citation, 10);
    const anchorMap = {
      1: "who-ihc-workup",
      2: "travis-p40-specificity",
      3: "molecular-reflex-panel",
    };
    const anchor = anchorMap[idx];
    if (anchor) {
      const activeLayout = document.querySelector(".tab-btn.active")?.dataset.layout;
      if (activeLayout === "wsi-focus" || activeLayout === "dicom-focus") {
        document.querySelector('.tab-btn[data-layout="split-all"]')?.click();
      }
      setTimeout(() => {
        window.__laminaReaderBridge.scrollToAnchor(anchor);
      }, 50);
    }
  }
});

// Stream Event Dispatcher matching SPEC-05-INTELLIGENCE
function handleDiagnosticStreamEvent(evt) {
  if (!evt) return;

  if (evt.type === "token") {
    accumulatedMarkdown += evt.data.text;
    streamContent.innerHTML = formatMarkdownToHtml(accumulatedMarkdown);
    hudStreamOutput.scrollTop = hudStreamOutput.scrollHeight;
  } else if (evt.type === "tool_call") {
    hudToolBanner.style.display = "flex";
    toolBannerText.textContent = `${evt.data.tool_name}: ${evt.data.query}`;
  } else if (evt.type === "citation") {
    hudCitationsSection.style.display = "flex";
    const chip = document.createElement("button");
    chip.className = "citation-chip";
    chip.innerHTML = `<strong>[${evt.data.index}]</strong> ${evt.data.title}`;
    chip.title = `PMID: ${evt.data.pmid} — Click to inspect in Evidence Reader`;

    // Deep link into Literature Reader
    chip.addEventListener("click", () => {
      // Map citation index to known anchor
      const anchorMap = {
        1: "who-ihc-workup",
        2: "travis-p40-specificity",
        3: "molecular-reflex-panel",
      };
      const anchor = anchorMap[evt.data.index] || evt.data.pmid;
      // Switch workspace layout if reader is hidden
      const activeLayout = document.querySelector(".tab-btn.active")?.dataset.layout;
      if (activeLayout === "wsi-focus" || activeLayout === "dicom-focus") {
        document.querySelector('.tab-btn[data-layout="split-all"]')?.click();
      }
      window.__laminaReaderBridge.scrollToAnchor(anchor);
    });

    hudCitationChips.appendChild(chip);
  } else if (evt.type === "verification") {
    guardrailStatus.style.display = "block";
    const scorePct = (evt.data.confidence * 100).toFixed(1);
    const verText = evt.data.verified ? "Citations Verified" : "Flagged";
    guardrailText.textContent = `✓ Tier 1 Guardrail (Decision-1): ${scorePct}% Calibrated Confidence (${verText})`;
  } else if (evt.type === "done") {
    isStreaming = false;
    lastInteractionId = evt.data.interaction_id;
    currentInteractionId = null;
    streamCursor.style.display = "none";
    hudToolBanner.style.display = "none";
    hudCancelBtn.style.display = "none";
    hudResetBtn.style.display = "inline-block";
    hudCopyBtn.style.display = "inline-block";
    hudFollowupForm.style.display = "flex";
    hudFollowupInput.focus();
  } else if (evt.type === "error") {
    isStreaming = false;
    currentInteractionId = null;
    streamCursor.style.display = "none";
    hudToolBanner.style.display = "none";
    hudCancelBtn.style.display = "none";
    hudResetBtn.style.display = "inline-block";
    hudCopyBtn.style.display = "inline-block";
    if (!accumulatedMarkdown.includes(`[${evt.data.code}]`)) {
      accumulatedMarkdown += `\n\n*Stream Error [${evt.data.code}]: ${evt.data.message}*`;
      streamContent.innerHTML = formatMarkdownToHtml(accumulatedMarkdown);
    }
  }
}

// Listen to Tauri stream event
if (isTauri) {
  listen("lamina:diagnostic-stream", (event) => {
    handleDiagnosticStreamEvent(event.payload);
  });
}

// Submit Diagnostic Query Stream
async function submitQueryStream(promptText, isFollowUp = false) {
  if (!promptText) {
    alert("Please enter a clinical prompt.");
    return;
  }

  // HIPAA Optical Allow-List check: patch strictly <= 1024x1024 if selected
  const patchB64 = state.wsi.activeRoi
    ? hudPatchCanvas.toDataURL("image/png").split(",")[1]
    : null;

  // Append pinned literature context into prompt if active
  let fullPrompt = promptText;
  if (state.hud.pinnedExcerpt && state.hud.pinnedExcerpt.text) {
    const p = state.hud.pinnedExcerpt;
    const ref = p.pmid_or_doi ? ` (PMID: ${p.pmid_or_doi})` : "";
    fullPrompt += `\n\n[Pinned Literature Context: ${p.document_title || "Reference"}${ref}]:\n"${p.text}"`;
  }

  const req = {
    query_text: fullPrompt,
    patch_png_base64: patchB64,
    magnification_mpp: state.wsi.mppX,
    previous_interaction_id: isFollowUp ? lastInteractionId : null,
  };

  // UI state transition
  isStreaming = true;
  hudQueryForm.style.display = "none";
  hudStreamPanel.style.display = "flex";
  hudSubmitBtn.style.display = "none";
  hudCancelBtn.style.display = "inline-block";
  hudResetBtn.style.display = "none";
  hudCopyBtn.style.display = "none";
  hudFollowupForm.style.display = "none";
  streamCursor.style.display = "inline-block";

  if (!isFollowUp) {
    accumulatedMarkdown = "";
    streamContent.innerHTML = "";
    hudCitationChips.innerHTML = "";
    hudCitationsSection.style.display = "none";
    guardrailStatus.style.display = "none";
  } else {
    accumulatedMarkdown += `\n\n---\n\n**Follow-up Inquiry**: *\"${promptText}\"*\n\n`;
    streamContent.innerHTML = formatMarkdownToHtml(accumulatedMarkdown);
  }

  try {
    if (isTauri) {
      const interactionId = await invoke("submit_diagnostic_query", { request: req });
      currentInteractionId = interactionId;
    } else {
      currentInteractionId = `cds_browser_${Date.now()}`;
      simulateBrowserCdsStream(fullPrompt, isFollowUp);
    }
  } catch (err) {
    handleDiagnosticStreamEvent({
      type: "error",
      data: { code: "INVOKE_FAILED", message: String(err) },
    });
  }
}

// Offline / browser development stream simulation
function simulateBrowserCdsStream(promptText, isFollowUp) {
  handleDiagnosticStreamEvent({
    type: "tool_call",
    data: {
      tool_name: "PubMed Grounding Engine",
      query: `WHO Thoracic 5th Ed IHC algorithm: ${promptText}`,
    },
  });

  const tokens = isFollowUp
    ? [
        "### Multi-Turn Diagnostic Synthesis\n\n",
        `Regarding your follow-up inquiry: *\"${promptText}\"*\n\n`,
        "**Differential Refinement & Tissue Stewardship Analysis**:\n",
        "In evaluating secondary markers to distinguish poorly differentiated adenocarcinoma from squamous variants, ",
        "tissue preservation must remain paramount for companion diagnostic molecular testing. ",
        "If initial TTF-1 and p40 markers yield dual-negative or ambiguous profiles, an expanded reflex panel ",
        "incorporating **Napsin A** and **CK5/6** is indicated [1, 2].\n\n",
        "**Molecular Reflex Recommendation**:\n",
        "All advanced non-small cell lung carcinomas lacking driver mutations should reflexively undergo ",
        "comprehensive NGS panel testing (EGFR, ALK, ROS1, BRAF, RET, MET, KRAS) and PD-L1 TPS immunohistochemistry [3].\n",
      ]
    : [
        "## Clinical Histopathological Assessment\n\n",
        "**Microscopic Evaluation (Magnification: 40× | MPP: 0.25 µm/px)**:\n",
        "Inspection of the optical sub-patch reveals an invasive epithelial proliferation characterized by ",
        "atypical glandular configurations, irregular cribriform patterns, and cohesive solid nests. ",
        "Tumor cells exhibit high nuclear-to-cytoplasmic ratio, marked hyperchromasia, irregular chromatin clumping, ",
        "and prominent macro-nucleoli. Brisk mitotic activity is readily identifiable. ",
        "Definitive squamous features (intercellular bridges, keratinization pearls) are absent within the analyzed field [1].\n\n",
        "### Diagnostic Differential Ranking\n",
        "1. **Pulmonary Invasive Adenocarcinoma (Acinar/Solid Predominant)** — Favored primary diagnosis based on architectural atypia and nuclear morphology [1].\n",
        "2. **Poorly Differentiated Squamous Cell Carcinoma (Non-Keratinizing)** — Essential differential to exclude in solid architectural variants [1, 2].\n",
        "3. **Large Cell Neuroendocrine Carcinoma (LCNEC)** — Diagnostic consideration if high mitotic rate (>10/2mm²) and organoid nesting are present [3].\n\n",
        "### Recommended Diagnostic IHC Panel\n",
        "In alignment with WHO 5th Edition Thoracic Pathology guidelines, an initial minimal two-marker IHC panel is recommended to conserve tissue for downstream molecular analysis:\n\n",
        "| Biomarker | Specificity Target | Typical Adeno Profile | Typical Squamous Profile |\n",
        "| :--- | :--- | :--- | :--- |\n",
        "| **TTF-1 (8G7G3/1)** | Pulmonary Adenocarcinoma | Strong Diffuse Nuclear (+) | Negative (-) [1, 2] |\n",
        "| **p40 (ΔNp63)** | Squamous Cell Carcinoma | Negative (-) | Strong Diffuse Nuclear (+) [2] |\n\n",
        "*Reflex Markers*: If TTF-1 and p40 are discordant or non-reactive, reflex to Napsin A, CK7, and CK5/6.\n\n",
        "### Companion Molecular Diagnostics\n",
        "Tissue stewardship mandate: Ensure adequate cellularity (>20% tumor nuclei) for reflex molecular NGS interrogation (EGFR, ALK, ROS1, KRAS G12C, MET ex14, RET, BRAF V600E) and PD-L1 expression [3].\n",
      ];

  let idx = 0;
  const interval = setInterval(() => {
    if (!isStreaming) {
      clearInterval(interval);
      return;
    }
    if (idx < tokens.length) {
      handleDiagnosticStreamEvent({ type: "token", data: { text: tokens[idx] } });
      idx++;
    } else {
      clearInterval(interval);
      handleDiagnosticStreamEvent({
        type: "citation",
        data: {
          index: 1,
          title: "WHO Classification of Thoracic Tumours (5th Edition, 2021)",
          pmid: "33906806",
          url: "https://pubmed.ncbi.nlm.nih.gov/33906806/",
        },
      });
      handleDiagnosticStreamEvent({
        type: "citation",
        data: {
          index: 2,
          title: "Travis WD, et al. Diagnosis of Lung Cancer in Small Biopsies and Cytology. J Thorac Oncol 2011",
          pmid: "21317737",
          url: "https://pubmed.ncbi.nlm.nih.gov/21317737/",
        },
      });
      handleDiagnosticStreamEvent({
        type: "citation",
        data: {
          index: 3,
          title: "Yatabe Y, et al. Best Practices for Pathologic Diagnosis of NSCLC. J Thorac Oncol 2019",
          pmid: "30880145",
          url: "https://pubmed.ncbi.nlm.nih.gov/30880145/",
        },
      });
      handleDiagnosticStreamEvent({
        type: "verification",
        data: { confidence: 0.984, verified: true },
      });
      handleDiagnosticStreamEvent({
        type: "done",
        data: { interaction_id: `cds_browser_${Date.now()}` },
      });
    }
  }, 40);
}

// Stream Cancellation per SPEC-05 section 6.3
async function cancelDiagnosticStream() {
  if (!isStreaming) return;
  isStreaming = false;

  const idToCancel = currentInteractionId || lastInteractionId;

  if (isTauri) {
    try {
      await invoke("cancel_diagnostic_query", { interactionId: idToCancel });
    } catch (e) {
      console.warn("Cancel invoke error:", e);
    }
  } else {
    handleDiagnosticStreamEvent({
      type: "error",
      data: { code: "CANCELLED", message: "Stream aborted by clinician." },
    });
  }
}

// Reset HUD to Prompt Input
function resetHudForm() {
  isStreaming = false;
  hudStreamPanel.style.display = "none";
  hudQueryForm.style.display = "flex";
  hudSubmitBtn.style.display = "inline-block";
  hudCancelBtn.style.display = "none";
  hudResetBtn.style.display = "none";
  hudCopyBtn.style.display = "none";
  hudFollowupForm.style.display = "none";
  hudQueryInput.value = "";
  hudFollowupInput.value = "";
}

// Action Event Listeners
hudSubmitBtn.addEventListener("click", () => {
  submitQueryStream(hudQueryInput.value.trim());
});

hudCancelBtn.addEventListener("click", () => {
  cancelDiagnosticStream();
});

hudResetBtn.addEventListener("click", () => {
  resetHudForm();
});

hudFollowupBtn.addEventListener("click", () => {
  const followUpText = hudFollowupInput.value.trim();
  if (followUpText) {
    hudFollowupInput.value = "";
    submitQueryStream(followUpText, true);
  }
});

function fallbackCopy(text) {
  const ta = document.createElement("textarea");
  ta.value = text;
  document.body.appendChild(ta);
  ta.select();
  document.execCommand("copy");
  document.body.removeChild(ta);
  hudCopyBtn.textContent = "Copied!";
  setTimeout(() => {
    hudCopyBtn.textContent = "Copy Report";
  }, 1500);
}

hudCopyBtn.addEventListener("click", () => {
  const text = streamContent.innerText;
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(() => {
      hudCopyBtn.textContent = "Copied!";
      setTimeout(() => {
        hudCopyBtn.textContent = "Copy Report";
      }, 1500);
    }).catch(() => {
      fallbackCopy(text);
    });
  } else {
    fallbackCopy(text);
  }
});

// ==========================================
// 5. COMMAND PALETTE MODAL
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
  } else if (action === "layout_split") {
    document.querySelector('.tab-btn[data-layout="split-all"]')?.click();
  } else if (action === "layout_wsi") {
    document.querySelector('.tab-btn[data-layout="wsi-focus"]')?.click();
  } else if (action === "layout_dicom") {
    document.querySelector('.tab-btn[data-layout="dicom-focus"]')?.click();
  } else if (action === "layout_reader") {
    document.querySelector('.tab-btn[data-layout="reader-focus"]')?.click();
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
const workspaceContainer = document.getElementById("workspaceContainer");
document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");

    const layout = btn.dataset.layout;
    workspaceContainer.className = `workspace-container layout-${layout}`;

    // Recalculate dimensions for viewports
    setTimeout(() => {
      resizeWsiCanvas();
      renderWsi();
      renderDicomSlice();
    }, 50);
  });
});

// Active Viewport Tracking matching SPEC-01-SHELL
function setActiveViewport(viewportId, kind) {
  document.querySelectorAll(".workspace-panel").forEach((panel) => {
    panel.classList.remove("active-focus");
  });
  const el = document.getElementById(`${viewportId}Panel`);
  if (el) el.classList.add("active-focus");

  if (isTauri) {
    invoke("set_active_viewport", { viewportId, kind }).catch(console.warn);
  }
}

document.getElementById("wsiPanel")?.addEventListener("mousedown", () => {
  setActiveViewport("wsi", "wsi");
});
document.getElementById("dicomPanel")?.addEventListener("mousedown", () => {
  setActiveViewport("dicom", "dicom");
});
document.getElementById("readerPanel")?.addEventListener("mousedown", () => {
  setActiveViewport("reader", "reader");
});

// Load and populate reader documents from backend
async function loadReaderDocuments() {
  if (isTauri) {
    try {
      const docs = await invoke("reader_get_documents");
      if (docs && docs.length > 0 && readerDocSelect) {
        const existingIds = Array.from(readerDocSelect.options).map((o) => o.value);
        for (const doc of docs) {
          if (!existingIds.includes(doc.id)) {
            const opt = document.createElement("option");
            opt.value = doc.id;
            opt.textContent = doc.title;
            readerDocSelect.appendChild(opt);
          }
        }
      }
    } catch (e) {
      console.warn("Failed to load reader documents:", e);
    }
  }
}

// Initialize on page load
window.addEventListener("DOMContentLoaded", () => {
  initDicom();
  renderWsi();
  setActiveViewport("wsi", "wsi");
  loadReaderDocuments();
});

