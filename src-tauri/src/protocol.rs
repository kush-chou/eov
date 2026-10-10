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

/// Generates a synthetic H&E histological tile for testing or sample viewing.
pub fn generate_synthetic_he_tile(level: u32, tile_x: u32, tile_y: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((TILE_SIZE * TILE_SIZE * 4) as usize);
    let scale = 1u32 << level;
    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            let gx = (tile_x * TILE_SIZE + x) * scale;
            let gy = (tile_y * TILE_SIZE + y) * scale;

            // Synthetic H&E cellular pattern: pink eosin background with purple hematoxylin nuclei
            let freq = (gx as f32 * 0.05).sin() * (gy as f32 * 0.05).cos();
            let nucleus_dist = ((gx % 32) as f32 - 16.0).hypot((gy % 32) as f32 - 16.0);

            let (r, g, b) = if nucleus_dist < 6.0 {
                (70, 30, 110) // Deep purple hematoxylin (cell nucleus)
            } else if freq > 0.1 {
                (225, 140, 180) // Eosinophilic cytoplasm / stroma (pink)
            } else {
                (245, 235, 240) // Interstitial / lumen background
            };

            rgba.extend_from_slice(&[r, g, b, 255]);
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
