use crate::state::AppState;
use crate::wsi::engine::encode_rgba_to_png;
use crate::wsi::coordinates::TILE_SIZE;
use tauri::http::{Response, StatusCode};
use tauri::Manager;

/// Parses a lamina-tile:// URI into (slide_id, level, tile_x, tile_y).
/// Supports both:
/// - lamina-tile://localhost/{slide_id}/{level}/{x}/{y}
/// - lamina-tile://{slide_id}/{level}/{x}/{y}
pub fn parse_tile_uri(uri_str: &str) -> Option<(String, u32, u32, u32)> {
    let clean = uri_str.strip_prefix("lamina-tile://")?;
    let parts: Vec<&str> = clean
        .split('/')
        .filter(|s| !s.is_empty() && *s != "localhost" && *s != "tile")
        .collect();

    if parts.len() >= 4 {
        let len = parts.len();
        // Remove possible query parameters from the last segment
        let last = parts[len - 1].split('?').next().unwrap_or(parts[len - 1]);
        let y = last.parse::<u32>().ok()?;
        let x = parts[len - 2].parse::<u32>().ok()?;
        let level = parts[len - 3].parse::<u32>().ok()?;
        let slide_id = parts[..len - 3].join("/");
        Some((slide_id, level, x, y))
    } else {
        None
    }
}

#[inline]
fn hash2(x: i32, y: i32) -> (f32, f32) {
    let mut n = (x as u32).wrapping_mul(0x45d9f3b) ^ (y as u32).wrapping_mul(0x119de1f);
    n = n.wrapping_mul(0x45d9f3b);
    let h1 = ((n & 0xffff) as f32) / 65535.0;
    let h2 = (((n >> 16) & 0xffff) as f32) / 65535.0;
    (h1, h2)
}

/// Generates a realistic synthetic H&E histological tile with organic glandular architecture,
/// eosinophilic collagen stroma, and hematoxylin basophilic nuclei clusters.
pub fn generate_synthetic_he_tile(level: u32, tile_x: u32, tile_y: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((TILE_SIZE * TILE_SIZE * 4) as usize);
    let scale = 1u32 << level;
    let cell_size = 20.0f32;

    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            let gx = (tile_x * TILE_SIZE + x) * scale;
            let gy = (tile_y * TILE_SIZE + y) * scale;
            let fx = gx as f32;
            let fy = gy as f32;

            // Macro-level tissue envelope: creates glandular acini, lumen voids, and cellular sheets
            let macro1 = (fx * 0.0025).sin() * (fy * 0.003).cos();
            let macro2 = ((fx * 0.006 + fy * 0.004).sin() * 0.6) + ((fx * 0.012 - fy * 0.009).cos() * 0.3);
            let tissue_density = macro1 + macro2;

            if tissue_density < -0.38 {
                // Lumen void or glass slide background: pale creamy white
                let lumen_speck = ((fx * 0.1).sin() * (fy * 0.1).cos() * 2.0) as i16;
                let bg_val = (248 + lumen_speck).clamp(242, 252) as u8;
                rgba.extend_from_slice(&[bg_val, bg_val - 2, bg_val - 1, 255]);
                continue;
            }

            // Stroma fibrous texture
            let stroma_tex = (fx * 0.04 + (fy * 0.03).sin() * 2.5).sin() * 0.5 + 0.5;

            // Cellular nucleus lookup: check 3x3 surrounding jittered cell centers
            let cx_base = (fx / cell_size).floor() as i32;
            let cy_base = (fy / cell_size).floor() as i32;

            let mut min_dist_sq = 999.0f32;
            let mut nearest_nucleus_rad = 4.5f32;

            for dy in -1..=1 {
                for dx in -1..=1 {
                    let cx = cx_base + dx;
                    let cy = cy_base + dy;
                    let (h1, h2) = hash2(cx, cy);

                    // Skip cells in low tissue density areas to create natural luminal boundaries
                    let c_fx = (cx as f32 + 0.5) * cell_size;
                    let c_fy = (cy as f32 + 0.5) * cell_size;
                    let c_dens = (c_fx * 0.0025).sin() * (c_fy * 0.003).cos()
                        + ((c_fx * 0.006 + c_fy * 0.004).sin() * 0.6);
                    if c_dens < -0.35 && h1 > 0.35 {
                        continue;
                    }

                    let n_center_x = (cx as f32 + 0.2 + h1 * 0.6) * cell_size;
                    let n_center_y = (cy as f32 + 0.2 + h2 * 0.6) * cell_size;

                    let d_x = fx - n_center_x;
                    let d_y = fy - n_center_y;
                    let dist_sq = d_x * d_x + (d_y * 1.2) * (d_y * 1.2);

                    if dist_sq < min_dist_sq {
                        min_dist_sq = dist_sq;
                        nearest_nucleus_rad = 3.8 + h1 * 2.2;
                    }
                }
            }

            let nucleus_dist = min_dist_sq.sqrt();

            if nucleus_dist < nearest_nucleus_rad {
                // Basophilic hematoxylin nucleus (deep violet-purple with chromatin variation)
                let chromatin = ((fx * 0.4).sin() * (fy * 0.4).cos() * 8.0) as i16;
                let r = (60 + chromatin).clamp(45, 80) as u8;
                let g = (25 + chromatin / 2).clamp(18, 45) as u8;
                let b = (95 + chromatin).clamp(75, 125) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else if nucleus_dist < nearest_nucleus_rad + 3.0 {
                // Perinuclear cytoplasm (rich eosinophilic pink)
                let blend = (nucleus_dist - nearest_nucleus_rad) / 3.0;
                let r = (160.0 + blend * 55.0) as u8;
                let g = (65.0 + blend * 50.0) as u8;
                let b = (135.0 + blend * 30.0) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                // Eosinophilic collagen stroma / extracellular matrix
                let eosin_r = (215.0 + stroma_tex * 22.0) as u8;
                let eosin_g = (125.0 + stroma_tex * 25.0) as u8;
                let eosin_b = (165.0 + stroma_tex * 20.0) as u8;
                rgba.extend_from_slice(&[eosin_r, eosin_g, eosin_b, 255]);
            }
        }
    }
    rgba
}

/// Registers the `lamina-tile://` custom protocol on the Tauri Builder.
pub fn register_tile_protocol<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol(
        "lamina-tile",
        |ctx, request, responder| {
            let uri_str = request.uri().to_string();
            let app_handle = ctx.app_handle().clone();

            tauri::async_runtime::spawn_blocking(move || {
                let parsed = parse_tile_uri(&uri_str);
                let wants_raw = uri_str.contains("format=raw");

                match parsed {
                    Some((slide_id, level, tile_x, tile_y)) => {
                        // In-memory HIPAA Quarantine: reject requests for label, macro, barcode, overview
                        if crate::privacy::sanitizer::OpticalAllowList::validate_resource_name(&slide_id).is_err() {
                            let resp = Response::builder()
                                .status(StatusCode::FORBIDDEN)
                                .header("Content-Type", "text/plain")
                                .header("Access-Control-Allow-Origin", "*")
                                .body(b"Forbidden asset access: Whole-slide label, macro, or overview camera requested under HIPAA quarantine".to_vec())
                                .unwrap();
                            responder.respond(resp);
                            return;
                        }

                        let state = app_handle.state::<AppState>();

                        // Check if slide is in WsiEngine, or if synthetic/sample tile is requested
                        let tile_res = if slide_id == "sample" || slide_id.starts_with("synthetic") {
                            let rgba = generate_synthetic_he_tile(level, tile_x, tile_y);
                            if wants_raw {
                                Ok((rgba, "application/octet-stream"))
                            } else {
                                encode_rgba_to_png(&rgba, TILE_SIZE, TILE_SIZE)
                                    .map(|png| (png, "image/png"))
                                    .map_err(|e| e.to_string())
                            }
                        } else {
                            if wants_raw {
                                state
                                    .wsi
                                    .get_tile(&slide_id, level, tile_x, tile_y)
                                    .map(|arc| ((*arc).clone(), "application/octet-stream"))
                                    .map_err(|e| e.to_string())
                            } else {
                                state
                                    .wsi
                                    .get_tile_png(&slide_id, level, tile_x, tile_y)
                                    .map(|png| (png, "image/png"))
                                    .map_err(|e| e.to_string())
                            }
                        };

                        match tile_res {
                            Ok((body, content_type)) => {
                                let resp = Response::builder()
                                    .header("Content-Type", content_type)
                                    .header("Access-Control-Allow-Origin", "*")
                                    .header("Cache-Control", "public, max-age=3600")
                                    .body(body)
                                    .unwrap();
                                responder.respond(resp);
                            }
                            Err(err_msg) => {
                                let resp = Response::builder()
                                    .status(StatusCode::NOT_FOUND)
                                    .header("Content-Type", "text/plain")
                                    .header("Access-Control-Allow-Origin", "*")
                                    .body(err_msg.into_bytes())
                                    .unwrap();
                                responder.respond(resp);
                            }
                        }
                    }
                    None => {
                        let resp = Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .header("Content-Type", "text/plain")
                            .header("Access-Control-Allow-Origin", "*")
                            .body(b"Invalid lamina-tile URI format".to_vec())
                            .unwrap();
                        responder.respond(resp);
                    }
                }
            });
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tile_uri_formats() {
        // Standard format with localhost
        let uri1 = "lamina-tile://localhost/slide123/2/10/15";
        let parsed1 = parse_tile_uri(uri1).unwrap();
        assert_eq!(parsed1, ("slide123".to_string(), 2, 10, 15));

        // Format without localhost
        let uri2 = "lamina-tile://tumor.svs/0/1/4";
        let parsed2 = parse_tile_uri(uri2).unwrap();
        assert_eq!(parsed2, ("tumor.svs".to_string(), 0, 1, 4));

        // Format with query parameter
        let uri3 = "lamina-tile://localhost/slide_abc/3/5/8?format=raw";
        let parsed3 = parse_tile_uri(uri3).unwrap();
        assert_eq!(parsed3, ("slide_abc".to_string(), 3, 5, 8));

        // Invalid format
        assert!(parse_tile_uri("lamina-tile://localhost/invalid").is_none());
    }

    #[test]
    fn test_generate_synthetic_he_tile() {
        let tile = generate_synthetic_he_tile(0, 0, 0);
        assert_eq!(tile.len(), (TILE_SIZE * TILE_SIZE * 4) as usize);
    }

    #[test]
    fn test_quarantine_rejection_for_forbidden_tile_uris() {
        let uri_label = "lamina-tile://localhost/slide_label/0/0/0";
        let (slide_id, _, _, _) = parse_tile_uri(uri_label).unwrap();
        assert!(crate::privacy::sanitizer::OpticalAllowList::validate_resource_name(&slide_id).is_err());

        let uri_macro = "lamina-tile://localhost/macro_camera/0/0/0";
        let (slide_id_m, _, _, _) = parse_tile_uri(uri_macro).unwrap();
        assert!(crate::privacy::sanitizer::OpticalAllowList::validate_resource_name(&slide_id_m).is_err());

        let uri_ok = "lamina-tile://localhost/tumor_section_42/0/0/0";
        let (slide_id_ok, _, _, _) = parse_tile_uri(uri_ok).unwrap();
        assert!(crate::privacy::sanitizer::OpticalAllowList::validate_resource_name(&slide_id_ok).is_ok());
    }
}
