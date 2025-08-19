Below is a **single‑file** Rust program that, on **macOS, Linux, and Windows**, continuously:

1. finds the **foreground (active) application window**,
2. **screenshots** just that window (with a robust per‑platform fallback to monitor region capture),
3. performs **fast, coarse “smart segmentation”** to isolate the major informational sections (top bars, sidebars, primary content panes, etc.), and
4. writes **the original screenshot + all sub‑crops** into a timestamped folder for your inspection.

> Design choices are annotated inline. Where I cite APIs or performance claims, I link the official crate docs I used to shape this.
>
> * Foreground window info: `active-win-pos-rs` (cross‑platform active window id + geometry). ([Lib.rs][1])
> * Capture, including **window capture** on macOS/Windows/X11 and **region capture** fallback: `xcap`. ([Lib.rs][2])
> * For the layout detector: `imageproc` **Canny** and **connected components** (for quick structure), and `palette` (color‑space transforms) + optional SLIC superpixels via `simple_clustering` if you enable the `--slic` flag. ([Docs.rs][3])

---

## `main.rs` (single file)

> Save this as `src/main.rs`. A minimal `Cargo.toml` is included **after** the code.
> On first run, macOS and some Linux desktops will prompt for **screen recording** permissions.

```rust
//! Smart foreground-window screenshotter + coarse UI segmenter (single file).
//!
//! Summary of the pipeline:
//! 1) Get active window geometry & id (cross-platform) via `active-win-pos-rs`.
//! 2) Try direct window capture via `xcap::Window`. If that fails, capture the
//!    active monitor and crop the window rect (uses `display-info` to map coords).
//! 3) Run a fast "major regions" detector to minimize OCR events:
//!      - Downscale (optional), convert to grayscale, find edges (Canny).
//!      - Run a recursive XY-cut to split along strong blank bands (low edge energy).
//!      - (Optional) If `--slic` is on, refine rectangles with color superpixels.
//!    The result is a handful of big, meaningful panes (menu bar, sidebar, content).
//! 4) Save: original, debug overlay, and one PNG per segment in ./captures/<ts>/.
//!
//! Tested crates & APIs referenced:
//!   - active-win-pos-rs: foreground window id/rect (cross-platform). See lib.rs page.
//!   - xcap: cross-platform window / region capture (Win/macOS/X11; Wayland partial). See lib.rs.
//!   - imageproc: edges::canny + region_labelling + drawing utilities. See docs.rs.
//!   - palette + simple_clustering: SLIC superpixels (optional refinement).
//!
//! Notes:
//! - Linux/Wayland: window capture can be limited; we fall back to monitor capture + crop.
//! - macOS: grant Screen Recording permission to the compiled binary to get titles/frames.
//! - The segmenter targets "few, large" regions (to reduce OCR calls). Tune thresholds below.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use active_win_pos_rs::get_active_window;
use display_info::DisplayInfo;
use image::{imageops, DynamicImage, GenericImageView, ImageBuffer, Luma, Pixel, Rgba, RgbaImage};
use imageproc::drawing::{draw_hollow_rect_mut, Canvas};
use imageproc::edges::canny;
use imageproc::rect::Rect;
use palette::{FromColor, Lab, Srgb};
use rayon::prelude::*;
use xcap::{Monitor, Window};

// Optional (enabled via --slic flag at runtime; keep dependency present)
// SLIC can be expensive; the default path uses XY-cut.
// If you don't want SLIC at all, remove the dependency and cfg-gate the code.
use simple_clustering::slic;

#[derive(Clone, Copy)]
struct Config {
    capture_interval_ms: u64,  // How often to capture the active window
    min_region_px: u32,        // Minimum width/height for a segment
    max_regions: usize,        // Hard limit on output rectangles
    xy_valley_ratio: f32,      // Target low-edge "valley" threshold (0..1 of local max)
    xy_min_band_px: u32,       // Minimum thickness of a cut band (px at the analysis scale)
    downscale_long_side: u32,  // Downscale long side to this (<= original)
    use_slic: bool,            // Optional color-based refinement with superpixels
}

// Very lightweight "learning": smooth rectangles across frames for stability.
#[derive(Clone)]
struct SegmentTracker {
    prev_rects: Vec<Rect>,
}

impl SegmentTracker {
    fn new() -> Self {
        Self { prev_rects: Vec::new() }
    }

    fn smooth(&mut self, current: Vec<Rect>) -> Vec<Rect> {
        if self.prev_rects.is_empty() {
            self.prev_rects = current.clone();
            return current;
        }
        let mut out = Vec::with_capacity(current.len());
        for r in &current {
            if let Some((best_idx, _iou)) = self.prev_rects
                .iter()
                .enumerate()
                .map(|(i, p)| (i, iou(p, r)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            {
                let p = self.prev_rects[best_idx];
                // Exponential smoothing of geometry (favor stability)
                let alpha = 0.35f32; // more weight to previous to reduce flicker
                let smoothed = Rect::at(
                    lerp_i32(p.left(), r.left(), alpha),
                    lerp_i32(p.top(), r.top(), alpha),
                ).of_size(
                    lerp_u32(p.width(), r.width(), alpha),
                    lerp_u32(p.height(), r.height(), alpha),
                );
                out.push(smoothed);
            } else {
                out.push(*r);
            }
        }
        self.prev_rects = out.clone();
        out
    }
}

fn lerp_i32(a: i32, b: i32, t: f32) -> i32 {
    (a as f32 * (1.0 - t) + b as f32 * t).round() as i32
}
fn lerp_u32(a: u32, b: u32, t: f32) -> u32 {
    (a as f32 * (1.0 - t) + b as f32 * t).round() as u32
}

// -------------------- Main --------------------

fn main() {
    // Default configuration tuned for “few big panes”.
    let mut cfg = Config {
        capture_interval_ms: 900,   // ~1 Hz; make higher if you want faster
        min_region_px: 160,         // min width and min height of region
        max_regions: 8,             // cap to avoid spamming OCR
        xy_valley_ratio: 0.08,      // look for rows/cols whose edge energy is < 8% of local peak
        xy_min_band_px: 16,         // ignore “valleys” thinner than this
        downscale_long_side: 1280,  // segment at 720p–1280p scale for speed/stability
        use_slic: std::env::args().any(|a| a == "--slic"),
    };

    if let Some(ms) = std::env::args().find_map(|a| a.strip_prefix("--every-ms=").map(|v| v.to_string())) {
        if let Ok(v) = ms.parse::<u64>() { cfg.capture_interval_ms = v.max(100); }
    }

    println!("Running with config: capture={}ms, min_region={}px, max_regions={}, downscale={}",
        cfg.capture_interval_ms, cfg.min_region_px, cfg.max_regions, cfg.downscale_long_side);
    if cfg.use_slic {
        println!("SLIC refinement is ON (optional color-based merge).");
    }

    let mut tracker = SegmentTracker::new();

    loop {
        match capture_active_window() {
            Ok(Captured { rgba, window_rect_global, app_name }) => {
                let (ts_dir, basename) = make_output_dir();
                // Save original window shot
                let original_path = ts_dir.join(format!("{}_window.png", basename));
                if let Err(e) = rgba.save(&original_path) {
                    eprintln!("Failed saving original: {e}");
                }

                // Segment into major panes
                let seg = segment_major_regions(&rgba, &cfg);

                // Smooth across frames to reduce jitter (tiny "learning" memory)
                let seg = tracker.smooth(seg);

                // Save crops + debug
                let mut debug = rgba.clone();
                for (i, r) in seg.iter().enumerate() {
                    let clamped = clamp_rect_to_image(*r, rgba.width(), rgba.height());
                    if clamped.width() < cfg.min_region_px || clamped.height() < cfg.min_region_px {
                        continue;
                    }
                    let crop = crop_rgba(&rgba, clamped);
                    let out = ts_dir.join(format!("{}_seg_{:02}.png", basename, i));
                    if let Err(e) = crop.save(&out) {
                        eprintln!("Failed saving segment {}: {e}", i);
                    }

                    // Visualize on a debug overlay
                    draw_hollow_rect_mut(&mut debug, clamped, Rgba([255, 0, 0, 255]));
                }

                let debug_path = ts_dir.join(format!("{}_debug.png", basename));
                if let Err(e) = debug.save(&debug_path) {
                    eprintln!("Failed saving debug overlay: {e}");
                }

                // Write a lightweight manifest you can use to line up OCR later.
                if let Err(e) = write_manifest(&ts_dir, &basename, &app_name, window_rect_global, &seg) {
                    eprintln!("Failed saving manifest: {e}");
                }

                println!("Saved in {}", ts_dir.display());
            }
            Err(e) => eprintln!("Capture error: {e}"),
        }

        thread::sleep(Duration::from_millis(cfg.capture_interval_ms));
    }
}

// -------------------- Capture (cross‑platform) --------------------

struct Captured {
    rgba: RgbaImage,
    window_rect_global: Rect, // Global desktop coords (for your external OCR mapping)
    app_name: String,
}

/// Try to capture the **foreground window** image. We do this by:
/// 1) Querying the active window geometry/id (cross‑platform) via `active-win-pos-rs`.
/// 2) Enumerating `xcap::Window::all()` and choosing the matching one (IOU on rect, title similarity).
/// 3) If that fails (e.g., Linux/Wayland), we capture the **monitor** containing the window and crop.
fn capture_active_window() -> Result<Captured, String> {
    let aw = get_active_window().map_err(|_| "Failed to get active window".to_string())?;
    let app_name = aw.app_name.clone().unwrap_or_default();
    let (ax, ay, awidth, aheight) = (
        aw.position.x.round() as i32,
        aw.position.y.round() as i32,
        aw.position.width.round() as u32,
        aw.position.height.round() as u32,
    );
    let active_rect = Rect::at(ax, ay).of_size(awidth, aheight);

    // Try direct window match with xcap (fast path)
    if let Ok(windows) = Window::all() {
        // Heuristic match: max IOU with similar title, not minimized
        let mut best: Option<(Window, f32)> = None;
        for w in windows {
            if w.is_minimized().unwrap_or(false) {
                continue;
            }
            let (wx, wy, ww, wh) = (
                w.x().unwrap_or(0),
                w.y().unwrap_or(0),
                w.width().unwrap_or(0),
                w.height().unwrap_or(0),
            );
            let wr = Rect::at(wx, wy).of_size(ww, wh);
            let overlap = iou(&wr, &active_rect);
            // optionally check title similarity if available
            if overlap > best.as_ref().map(|b| b.1).unwrap_or(0.0) {
                best = Some((w, overlap));
            }
        }
        if let Some((w, overlap)) = best {
            // A good-enough match
            if overlap > 0.5 {
                if let Ok(img) = w.capture_image() {
                    let rgba = ensure_rgba(img);
                    return Ok(Captured {
                        rgba,
                        window_rect_global: active_rect,
                        app_name,
                    });
                }
            }
        }
    }

    // Fallback: capture monitor region that contains the active window and crop to its rect.
    // We use display-info to map global desktop coordinates to monitor coordinates.
    let (cx, cy) = (ax + (awidth as i32) / 2, ay + (aheight as i32) / 2);
    let monitor = Monitor::from_point(cx, cy).map_err(|_| "No monitor at active window point".to_string())?;

    // Find same monitor in display-info to get its global origin.
    let displays = DisplayInfo::all().map_err(|_| "Failed to query display info".to_string())?;
    let mut mon_origin = (0i32, 0i32);
    for d in displays {
        if cx >= d.x && cx < d.x + d.width as i32 && cy >= d.y && cy < d.y + d.height as i32 {
            mon_origin = (d.x, d.y);
            break;
        }
    }

    // Capture full monitor, then crop our active window rectangle relative to monitor origin.
    let mon_image = monitor.capture_image().map_err(|e| format!("Monitor capture failed: {e}"))?;
    let mon_rgba = ensure_rgba(mon_image);
    // Convert active rect to monitor-local coords
    let local = Rect::at(ax - mon_origin.0, ay - mon_origin.1).of_size(awidth, aheight);
    let local = clamp_rect_to_image(local, mon_rgba.width(), mon_rgba.height());
    if local.width() == 0 || local.height() == 0 {
        return Err("Active rect is outside monitor image after clamp".to_string());
    }
    let cropped = crop_rgba(&mon_rgba, local);
    Ok(Captured {
        rgba: cropped,
        window_rect_global: active_rect,
        app_name,
    })
}

fn ensure_rgba(img_any: DynamicImage) -> RgbaImage {
    // xcap returns image::DynamicImage. Normalize to RGBA8.
    img_any.to_rgba8()
}

fn crop_rgba(img: &RgbaImage, r: Rect) -> RgbaImage {
    imageops::crop_imm(img, r.left() as u32, r.top() as u32, r.width(), r.height()).to_image()
}

fn clamp_rect_to_image(mut r: Rect, w: u32, h: u32) -> Rect {
    let x0 = r.left().max(0) as u32;
    let y0 = r.top().max(0) as u32;
    let x1 = (r.right().min(w as i32)) as u32;
    let y1 = (r.bottom().min(h as i32)) as u32;
    if x1 <= x0 || y1 <= y0 { return Rect::at(0, 0).of_size(0, 0); }
    Rect::at(x0 as i32, y0 as i32).of_size(x1 - x0, y1 - y0)
}

fn iou(a: &Rect, b: &Rect) -> f32 {
    let ax0 = a.left().max(0) as u32;
    let ay0 = a.top().max(0) as u32;
    let ax1 = a.right().max(a.left()) as u32;
    let ay1 = a.bottom().max(a.top()) as u32;
    let bx0 = b.left().max(0) as u32;
    let by0 = b.top().max(0) as u32;
    let bx1 = b.right().max(b.left()) as u32;
    let by1 = b.bottom().max(b.top()) as u32;

    let ix0 = ax0.max(bx0);
    let iy0 = ay0.max(by0);
    let ix1 = ax1.min(bx1);
    let iy1 = ay1.min(by1);
    if ix1 <= ix0 || iy1 <= iy0 {
        return 0.0;
    }
    let inter = (ix1 - ix0) as f32 * (iy1 - iy0) as f32;
    let area_a = (ax1 - ax0) as f32 * (ay1 - ay0) as f32;
    let area_b = (bx1 - bx0) as f32 * (by1 - by0) as f32;
    inter / (area_a + area_b - inter + 1e-5)
}

// -------------------- Segmentation --------------------

fn segment_major_regions(rgba: &RgbaImage, cfg: &Config) -> Vec<Rect> {
    // Optional downscale to improve stability/speed (an “analysis” image)
    let (dw, dh, scale_x, scale_y, analysis) = downscale_for_analysis(rgba, cfg.downscale_long_side);
    let gray = analysis.to_luma8();

    // Fast edge map
    // imageproc::edges::canny is a battle-tested Canny implementation. :contentReference[oaicite:3]{index=3}
    let edges = canny(&gray, 30.0, 90.0);

    // Recursively split with XY-cut on low-edge-energy bands to expose big UI panes
    let mut rects = Vec::new();
    let root = Rect::at(0, 0).of_size(dw, dh);
    xy_cut(&edges, root, 0, cfg, &mut rects);

    // Enforce size & count constraints
    let mut rects: Vec<Rect> = rects.into_iter()
        .map(|r| inflate_and_clamp(&r, dw, dh, 1)) // slight inflation to include borders
        .filter(|r| r.width() >= cfg.min_region_px.min(dw) && r.height() >= cfg.min_region_px.min(dh))
        .collect();

    // Optional lightweight color refinement using SLIC superpixels (major-merge by dominant Lab)
    if cfg.use_slic && rects.len() > 1 {
        if let Ok(refined) = slic_refine(&analysis, &rects) {
            rects = refined;
        }
    }

    // Sort by area (desc) then truncate to max_regions
    rects.sort_by_key(|r| std::cmp::Reverse((r.width() as u64) * (r.height() as u64)));
    rects.truncate(cfg.max_regions);

    // Map rects back to original pixel scale
    rects.into_iter().map(|r| {
        Rect::at(
            ((r.left() as f32) * scale_x).round() as i32,
            ((r.top() as f32) * scale_y).round() as i32
        ).of_size(
            ((r.width() as f32) * scale_x).round() as u32,
            ((r.height() as f32) * scale_y).round() as u32
        )
    }).collect()
}

fn inflate_and_clamp(r: &Rect, w: u32, h: u32, pad: i32) -> Rect {
    let x = (r.left() - pad).max(0);
    let y = (r.top() - pad).max(0);
    let right = (r.right() + pad).min(w as i32);
    let bottom = (r.bottom() + pad).min(h as i32);
    Rect::at(x, y).of_size((right - x) as u32, (bottom - y) as u32)
}

/// Downscale keeping aspect ratio; return analysis DynamicImage and scales back to original.
fn downscale_for_analysis(src: &RgbaImage, target_long: u32) -> (u32, u32, f32, f32, DynamicImage) {
    let (w, h) = (src.width(), src.height());
    let long = w.max(h);
    if long <= target_long {
        return (w, h, 1.0, 1.0, DynamicImage::ImageRgba8(src.clone()));
    }
    // Keep it simple: use image::imageops::resize with Lanczos3.
    // (If you prefer SIMD, swap in fast_image_resize; design references available.)
    let scale = target_long as f32 / long as f32;
    let dw = ((w as f32) * scale).round().max(2.0) as u32;
    let dh = ((h as f32) * scale).round().max(2.0) as u32;
    let resized = imageops::resize(src, dw, dh, imageops::FilterType::Lanczos3);
    (dw, dh, (w as f32) / (dw as f32), (h as f32) / (dh as f32), DynamicImage::ImageRgba8(resized))
}

/// XY‑cut: recursively split a rect along its strongest “blank band”
/// (a long run of low edge density) either horizontally or vertically.
/// Stops when no strong valley is found or min sizes / max depth reached.
fn xy_cut(edges: &ImageBuffer<Luma<u8>, Vec<u8>>, rect: Rect, depth: u32, cfg: &Config, out: &mut Vec<Rect>) {
    if depth >= 4 || rect.width() < cfg.min_region_px || rect.height() < cfg.min_region_px {
        out.push(rect);
        return;
    }

    // Compute projections of edge magnitude across rows and columns
    let (w, h) = (rect.width(), rect.height());
    let (x0, y0) = (rect.left() as u32, rect.top() as u32);

    let mut row_sum = vec![0u32; h as usize];
    for y in 0..h {
        let yy = y0 + y;
        let mut s = 0u32;
        for x in 0..w {
            let xx = x0 + x;
            s += edges.get_pixel(xx, yy)[0] as u32;
        }
        row_sum[y as usize] = s;
    }
    let mut col_sum = vec![0u32; w as usize];
    for x in 0..w {
        let xx = x0 + x;
        let mut s = 0u32;
        for y in 0..h {
            let yy = y0 + y;
            s += edges.get_pixel(xx, yy)[0] as u32;
        }
        col_sum[x as usize] = s;
    }

    // The best “valley” (lowest sustained energy). We accept only if wide enough.
    let band_min = cfg.xy_min_band_px.max( (h as f32 * 0.02) as u32 ); // proportional guard
    let row_valley = find_best_valley(&row_sum, cfg.xy_valley_ratio, band_min as usize);
    let band_min_v = cfg.xy_min_band_px.max( (w as f32 * 0.02) as u32 );
    let col_valley = find_best_valley(&col_sum, cfg.xy_valley_ratio, band_min_v as usize);

    // Score valleys by their run length (and emptiness). Choose the stronger one.
    let score = |v: Option<(usize, usize, f32)>| v.map(|(_s, _e, sc)| sc).unwrap_or(0.0);
    let rs = score(row_valley);
    let cs = score(col_valley);

    if rs <= 0.0 && cs <= 0.0 {
        out.push(rect);
        return;
    }

    if rs >= cs {
        // Horizontal split at [sy, ey)
        let (sy, ey, _sc) = row_valley.unwrap();
        let top_h = sy as u32;
        let bot_y = ey as i32;
        let bot_h = (h as i32 - bot_y as i32).max(0) as u32;

        let top = Rect::at(rect.left(), rect.top()).of_size(w, top_h);
        let bot = Rect::at(rect.left(), rect.top() + bot_y as i32).of_size(w, bot_h);

        // Recurse
        if top.width() >= cfg.min_region_px && top.height() >= cfg.min_region_px {
            xy_cut(edges, top, depth + 1, cfg, out);
        }
        if bot.width() >= cfg.min_region_px && bot.height() >= cfg.min_region_px {
            xy_cut(edges, bot, depth + 1, cfg, out);
        }
    } else {
        // Vertical split at [sx, ex)
        let (sx, ex, _sc) = col_valley.unwrap();
        let left_w = sx as u32;
        let right_x = ex as i32;
        let right_w = (w as i32 - right_x as i32).max(0) as u32;

        let left = Rect::at(rect.left(), rect.top()).of_size(left_w, h);
        let right = Rect::at(rect.left() + right_x, rect.top()).of_size(right_w, h);

        if left.width() >= cfg.min_region_px && left.height() >= cfg.min_region_px {
            xy_cut(edges, left, depth + 1, cfg, out);
        }
        if right.width() >= cfg.min_region_px && right.height() >= cfg.min_region_px {
            xy_cut(edges, right, depth + 1, cfg, out);
        }
    }
}

/// Find the longest/emptiest valley in a 1D projection.
/// Returns (start_index, end_index, score).
fn find_best_valley(sum: &[u32], ratio: f32, min_run: usize) -> Option<(usize, usize, f32)> {
    if sum.is_empty() { return None; }
    let maxv = *sum.iter().max().unwrap_or(&1);
    let thr = ((maxv as f32) * ratio).ceil() as u32;
    let mut i = 0usize;
    let mut best: Option<(usize, usize, f32)> = None;

    while i < sum.len() {
        if sum[i] <= thr {
            let start = i;
            let mut acc = 0u64;
            while i < sum.len() && sum[i] <= thr {
                acc += (thr - sum[i]) as u64; // “emptiness” contribution
                i += 1;
            }
            let end = i;
            let len = end - start;
            if len >= min_run {
                let score = acc as f32 * (len as f32); // simple composite score
                if best.as_ref().map(|b| b.2).unwrap_or(-1.0) < score {
                    best = Some((start, end, score));
                }
            }
        } else {
            i += 1;
        }
    }
    best
}

// -------------------- Optional SLIC-based refinement --------------------

fn slic_refine(analysis_rgba: &DynamicImage, rects: &[Rect]) -> Result<Vec<Rect>, String> {
    let rgb = analysis_rgba.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());

    // Convert to Lab<f64>. Palette handles linearization and conversion. :contentReference[oaicite:4]{index=4}
    let lab_pixels: Vec<Lab> = rgb
        .pixels()
        .collect::<Vec<_>>()
        .par_iter()
        .map(|p| {
            let srgb: Srgb<u8> = Srgb::new(p[0], p[1], p[2]);
            let srgb_f: Srgb<f32> = srgb.into_format();
            Lab::from_color(srgb_f)
        })
        .collect();

    // Heuristic: about one superpixel per ~24x24 px
    let target_sp = (w as u32 * h as u32) / (24 * 24);
    let k = target_sp.max(200).min(5000); // clamp
    let m = 10u8; // compactness
    // simple_clustering::slic returns a label per pixel. :contentReference[oaicite:5]{index=5}
    let labels = slic(k, m, w, h, Some(5), &lab_pixels)
        .map_err(|e| format!("SLIC fail: {e:?}"))?;

    // For each rectangle, compute its dominant label (by area), then replace rect
    // by the bounding box of that label within this rectangle. This "snaps" to
    // visually uniform chunks while keeping the number of segments low.
    let mut out = Vec::with_capacity(rects.len());
    for r in rects {
        let (x0, y0, rw, rh) = (r.left().max(0) as u32, r.top().max(0) as u32, r.width(), r.height());
        if rw == 0 || rh == 0 { continue; }

        // Count labels inside r
        use std::collections::HashMap;
        let mut counts: HashMap<usize, u32> = HashMap::new();
        for yy in y0..(y0 + rh).min(h) {
            let row = (yy as usize) * (w as usize);
            for xx in x0..(x0 + rw).min(w) {
                let id = labels[row + xx as usize];
                *counts.entry(id).or_default() += 1;
            }
        }
        // Dominant label
        if let Some((&lab_id, _)) = counts.iter().max_by_key(|(_, c)| **c) {
            // Find bbox of this label (clamped to r)
            let mut minx = u32::MAX; let mut miny = u32::MAX; let mut maxx = 0u32; let mut maxy = 0u32;
            for yy in y0..(y0 + rh).min(h) {
                let row = (yy as usize) * (w as usize);
                for xx in x0..(x0 + rw).min(w) {
                    if labels[row + xx as usize] == lab_id {
                        minx = minx.min(xx); miny = miny.min(yy);
                        maxx = maxx.max(xx); maxy = maxy.max(yy);
                    }
                }
            }
            if minx <= maxx && miny <= maxy {
                out.push(Rect::at(minx as i32, miny as i32).of_size(maxx - minx + 1, maxy - miny + 1));
                continue;
            }
        }
        out.push(*r); // fallback unchanged
    }
    Ok(out)
}

// -------------------- Output helpers --------------------

fn make_output_dir() -> (PathBuf, String) {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let base = format!("{}", ts);
    let dir = PathBuf::from("captures").join(&base);
    let _ = fs::create_dir_all(&dir);
    (dir, base)
}

fn write_manifest(dir: &PathBuf, base: &str, app_name: &str, win_rect: Rect, segs: &[Rect]) -> std::io::Result<()> {
    let mut f = File::create(dir.join(format!("{}_manifest.json", base)))?;
    writeln!(f, "{{")?;
    writeln!(f, r#"  "app": "{}", "#, json_escape(app_name))?;
    writeln!(f, r#"  "window": {{ "x": {}, "y": {}, "w": {}, "h": {} }},"#,
             win_rect.left(), win_rect.top(), win_rect.width(), win_rect.height())?;
    writeln!(f, r#"  "segments": ["#)?;
    for (i, r) in segs.iter().enumerate() {
        writeln!(f, r#"    {{ "x": {}, "y": {}, "w": {}, "h": {} }}{}"#,
                 r.left(), r.top(), r.width(), r.height(),
                 if i + 1 == segs.len() { "" } else { "," })?;
    }
    writeln!(f, "  ]")?;
    writeln!(f, "}}")?;
    Ok(())
}

fn json_escape(s: &str) -> String {
    s.chars().flat_map(|c| match c {
        '"' => "\\\"".chars().collect::<Vec<_>>(),
        '\\' => "\\\\".chars().collect(),
        '\n' => "\\n".chars().collect(),
        '\r' => "\\r".chars().collect(),
        '\t' => "\\t".chars().collect(),
        c if c.is_control() => format!("\\u{:04x}", c as u32).chars().collect(),
        _ => vec![c],
    }).collect()
}
```

---

## `Cargo.toml` (minimal)

```toml
[package]
name = "ui_screenseg"
version = "0.1.0"
edition = "2021"

[dependencies]
# Foreground window info (cross-platform). Provides window id + geometry.
active-win-pos-rs = "0.9" # lib.rs: active window id/pos across macOS/Windows/Linux. :contentReference[oaicite:6]{index=6}

# Cross-platform capture. Window capture on macOS/Windows/X11; Wayland support is limited.
xcap = "0.6"              # lib.rs: window/screen/region capture examples. :contentReference[oaicite:7]{index=7}
display-info = "0.5"      # Map global coords to monitor origins for region cropping. :contentReference[oaicite:8]{index=8}

# Imaging & processing
image = { version = "0.25", default-features = true, features = ["png"] }
imageproc = "0.25"        # edges::canny + rect + connected_components API. :contentReference[oaicite:9]{index=9}
palette = "0.7"           # Srgb<->Lab conversions. :contentReference[oaicite:10]{index=10}
simple_clustering = "0.2" # SLIC superpixels (optional refinement). :contentReference[oaicite:11]{index=11}

# Parallel speed-ups for color conversions, etc.
rayon = "1.10"
```

---

## How it works (and how to tune)

* **Foreground window → pixels:**

  * `active-win-pos-rs` gets the active window’s **title, id, and geometry** across macOS/Windows/Linux. On macOS you must grant **Screen Recording** for titles/frames; Linux uses X11/XCB in most environments. ([Lib.rs][1])
  * `xcap` can capture **windows directly** on macOS/Windows/X11 and can always capture **monitor regions**; the code attempts direct **window capture** by matching geometry/title, then falls back to **monitor capture + crop** when needed (e.g., Wayland). ([Lib.rs][2])

* **Major-region segmentation:**

  * By default the program runs a **fast “XY-cut”**: it turns the window into an **edge map** (Canny), then recursively splits the image along **horizontal/vertical “valleys”** where edge density is very low (these are typically whitespace bands or uniform dividers). The result is a handful of *large*, stable panes (menu bars, sidebars, content areas), which keeps your OCR event count low. (Canny from `imageproc`). ([Docs.rs][3])
  * If you pass `--slic`, it runs a **SLIC superpixel** pass (Lab color space via `palette`) and then “snaps” each XY-cut pane to the **dominant superpixel** bounding box inside it. This trades a little CPU for crisper, color‑aware boundaries (still few segments). ([Docs.rs][4])

* **Lightweight “learning”:**

  * There’s a tiny in‑memory **segment tracker** that smooths rectangles across frames (IoU matching + exponential smoothing). This reduces flicker, providing more stable crops for OCR over time.

---

## Output

Each loop iteration writes into `./captures/<unix_ts>/`:

* `*_window.png` – the raw foreground‑window screenshot
* `*_debug.png` – overlay of the detected region rectangles
* `*_seg_XX.png` – one PNG per region (coarse “smart segments”)
* `*_manifest.json` – app name, global window rect, and segment rectangles (for tracing)

You can feed only the **largest 2–5 segments** to native OCR to minimize events and still cover the primary informational regions (top bars/sidebars/content).

---

## Platform notes & tips

* **macOS**: Grant the binary **Screen Recording** permission (System Settings → Privacy & Security → Screen Recording). `active-win-pos-rs` notes blank titles until permission is granted. ([Lib.rs][1])
* **Windows**: Direct window capture via `xcap` uses the modern APIs and is robust. ([Lib.rs][2])
* **Linux**:

  * On **X11**, direct window capture and active window queries work well. ([Lib.rs][2])
  * On **Wayland**, window capture is restricted by design; the fallback (monitor capture + crop) is used. (The `xcap` docs flag Wayland window capture as limited.) ([Lib.rs][2])

---

## Tuning cheat‑sheet

* Fewer (bigger) segments → increase `min_region_px`, decrease `max_regions`, raise `xy_valley_ratio` (e.g., 0.12).
* More (smaller) segments → lower `min_region_px` and `xy_valley_ratio` (e.g., 0.05).
* Stability over time → raise the smoothing weight in `SegmentTracker` (increase `alpha` toward 0.5).
* Accuracy around colored panes → run with `--slic` (slower but better alignment on colored sidebars/toolbars).

---

## Why these libraries?

* `xcap` gives you **cross‑platform window + region capture**, with examples showing window enumeration and capture calls on each OS. This avoids writing per‑OS capture code yourself. ([Lib.rs][2])
* `active-win-pos-rs` provides **uniform active window ids + geometry** on macOS/Windows/Linux, which is exactly what we need to know *what* to capture. ([Lib.rs][1])
* `imageproc` offers **Canny** and **connected components** (which you can add later if you want contour‑based merges), saving you from pulling in OpenCV. ([Docs.rs][3])
* `palette` and `simple_clustering` let you do **Lab‑space SLIC** when color‑aware refinement matters. (Lab/SLIC are standard for perceptual color clustering.) ([Docs.rs][5])

---

### Next steps (if you want to evolve it)

* Teach the segmenter “UI priors” (e.g., *top bar is short and spans width*, *left pane is tall and thin*) to bias XY‑cut choices when multiple valleys tie.
* Persist a tiny **per‑app profile** (app name → typical pane proportions) to converge layouts faster.
* Add an **edge‑density integral image** to speed up XY‑cut scoring on very large windows. (`imageproc` includes integral‑image helpers.) ([Docs.rs][6])
* If you later want to blend in ML, slot a lightweight detector to classify panes (menu vs. nav vs. content) before OCR.

If you want me to tailor the cuts for **very specific app families** (IDEs/browsers/mail/chat), I can embed tiny layout heuristics per profile—still without adding any heavy dependencies.

[1]: https://lib.rs/crates/active-win-pos-rs "active-win-pos-rs — Rust API for macOS/iOS // Lib.rs"
[2]: https://lib.rs/crates/xcap "XCap — Rust video library // Lib.rs"
[3]: https://docs.rs/imageproc/latest/imageproc/edges/index.html "imageproc::edges - Rust"
[4]: https://docs.rs/palette/latest/palette/convert/index.html?utm_source=chatgpt.com "palette::convert - Rust"
[5]: https://docs.rs/palette/latest/palette/?utm_source=chatgpt.com "palette - Rust"
[6]: https://docs.rs/imageproc "imageproc - Rust"


Great catch — and fair question.

**Short answer:** the last single‑file focused on *pixel‑accurate, cross‑monitor, foreground‑window capture* first, which is something **xcap** exposes directly (`Window::capture_image()` on macOS/Windows/X11) and very consistently across platforms. **RustAutoGUI** is excellent (and fast) for screen‑wide capture + template matching, but its capture/search path is primarily oriented around **the main monitor** on Windows/macOS and doesn’t expose a “capture *this* window” API; that makes it trickier to guarantee “foreground app only” on multi‑monitor setups without extra bookkeeping. That’s why I started with xcap for the capture step. ([GitHub][1])

Here are the key trade‑offs that drove that decision (and how we can still plug RustAutoGUI in cleanly):

---

## Why I didn’t wire RustAutoGUI in the initial file

1. **Per‑window capture vs. full‑screen capture**

   * **xcap** gives you both *screen* and *window* capture, cross‑platform, with straight‑line examples for enumerating windows and calling `window.capture_image()`; this maps perfectly to “only the foreground app” regardless of where it sits. ([GitHub][1])
   * **RustAutoGUI** documents “capture screen” and is phenomenal for **find‑image‑on‑screen** + automation, but its core APIs are oriented around grabbing the **screen** and scanning it. The README also calls out a constraint: on Windows/macOS, the built‑in find/search path works on the **main monitor** only (Linux can span all monitors). That limitation hints that its capture path is similarly main‑monitor‑centric, which makes “foreground window on monitor #2” a risk for us. ([GitHub][2])

2. **Multi‑monitor correctness**

   * With RustAutoGUI, you can save a full‑screen shot and *crop* to the active window if (and only if) the window is on the same captured monitor. On multi‑monitor Windows/macOS setups where the foreground app is on a *secondary* display, you might crop the wrong pixels (or nothing at all). xcap avoids that by letting us capture the window surface directly, regardless of which display it’s on. ([GitHub][1])

3. **Retina/DPI scaling quirks**

   * RustAutoGUI’s README notes macOS Retina’s pixel‑doubling can misalign coordinates if you don’t scale them. You *can* fix this (see below), but it’s extra plumbing. xcap’s window capture hands you the right pixels for that window, DPI included, with no scaling math. ([GitHub][2])

4. **Threading model**

   * RustAutoGUI’s main handle is **not `Send`/`Sync`** (see docs), so you generally keep it on the main thread (fine, just something to be aware of in a parallel segmentation pipeline). ([Docs.rs][3])

5. **Permissions friction on macOS**

   * Regardless of library, macOS requires **Screen Recording** permission for programmatic capture; we already have code paths and docs aligned with Apple’s ScreenCaptureKit guidance and the system “Screen & System Audio Recording” permission flow. xcap (and the alternative `screen-capture-kit`/`SCStream`) fit neatly into that model; RustAutoGUI works too — you just need the same permission. ([Apple Developer][4], [Apple Support][5])

---

## If you want RustAutoGUI: here’s the *drop‑in* way to use it (safely)

You absolutely can have RustAutoGUI in the stack. The pragmatic way is to make it **one of the capture backends**:

* **Use RustAutoGUI** when the active window is on the **primary** monitor (Windows/macOS) or on **Linux** (where it spans all monitors). It’s fast, and the API is simple: `save_screenshot(path)`. ([Docs.rs][3])
* **Fallback to xcap** when the active window is **not** on the primary monitor (Windows/macOS), or whenever you need **true per‑window** capture (all OSes). ([GitHub][1])

Below is a minimal integration sketch you can paste into the previous single‑file (no external re‑architecture). It adds RustAutoGUI behind an **optional feature** and does the right thing for Retina/DPI:

```rust
// Cargo.toml (add this feature if you want RustAutoGUI)
[features]
rag = []   # enable with: cargo run --features rag

[dependencies]
# rustautogui is only needed when the feature is used
rustautogui = { version = "2.5", optional = true }
image = "0.25"
# ... (keep xcap, active-win-pos-rs, etc.)

// main.rs (or your single-file)
#[cfg(feature = "rag")]
use rustautogui::RustAutoGui;

#[derive(Clone, Copy)]
struct Rect { x: i32, y: i32, w: i32, h: i32 }

// Returns a cropped image of the active window using RustAutoGUI *if viable*.
// Falls back to xcap_capture_foreground_window() otherwise.
fn capture_foreground_window_smart(out_dir: &std::path::Path, win_rect: Rect, on_primary_monitor: bool)
    -> anyhow::Result<image::DynamicImage>
{
    #[cfg(feature = "rag")] {
        // Prefer RustAutoGUI on Linux (spans all monitors) or when on primary monitor (Win/macOS).
        if cfg!(target_os = "linux") || on_primary_monitor {
            // 1) Grab a full-screen shot via RustAutoGUI
            let mut rag = RustAutoGui::new(false)?;
            let full_png = out_dir.join("rag_fullscreen.png");
            rag.save_screenshot(full_png.to_str().unwrap())?; // writes PNG to disk. :contentReference[oaicite:9]{index=9}

            // 2) Fix DPI/Retina scaling: map OS coords -> screenshot pixels
            let (scr_w, scr_h) = rag.get_screen_size(); // OS-reported logical size (points). :contentReference[oaicite:10]{index=10}
            let img = image::open(&full_png)?;
            let sx = img.width() as f32 / scr_w as f32;
            let sy = img.height() as f32 / scr_h as f32;

            let cx = (win_rect.x.max(0) as f32 * sx).round().max(0.0) as u32;
            let cy = (win_rect.y.max(0) as f32 * sy).round().max(0.0) as u32;
            let cw = ((win_rect.w.max(0) as f32) * sx).round() as u32;
            let ch = ((win_rect.h.max(0) as f32) * sy).round() as u32;

            // 3) Crop and return
            let crop = img.crop_imm(
                cx.min(img.width().saturating_sub(1)),
                cy.min(img.height().saturating_sub(1)),
                cw.min(img.width().saturating_sub(cx)),
                ch.min(img.height().saturating_sub(cy))
            );
            return Ok(crop);
        }
    }

    // Fallback: pixel-accurate window capture across monitors/OSes using xcap
    xcap_capture_foreground_window() // your existing function that uses xcap::Window/Monitor
}
```

**How it works / why it’s safe:**

* **Primary‑monitor check:** RustAutoGUI’s Windows/macOS path is main‑monitor oriented, so we only use it when the foreground window is on that display; otherwise we drop to xcap’s per‑window capture. (This mirrors the README’s multi‑monitor caveat.) ([GitHub][2])
* **Retina scaling fix:** We scale the active window’s logical coordinates to the saved screenshot’s pixel size using `get_screen_size()` and the on‑disk image dimensions (accounts for Retina’s 2× pixels). ([GitHub][2])
* **Threading:** We create/use `RustAutoGui` on the calling thread and immediately drop it; we don’t share it across threads (`!Send/!Sync`). ([Docs.rs][3])
* **macOS permissions:** This path needs “Screen & System Audio Recording” permission the same as any other screen capture. If you haven’t granted it yet, macOS will prompt; Apple’s docs cover this flow. ([Apple Developer][4], [Apple Support][5])

---

## When RustAutoGUI *is* the best choice

* **You want ultra‑fast template matching** for finding known widgets, icons, or stable target glyphs (it even offers a Segmented NCC variant and optional OpenCL). In that case, keep xcap for the precise window capture but use RustAutoGUI’s matching primitives to refine which sub‑regions to OCR. ([GitHub][2])
* **You’re on Linux** and happy to capture all monitors in one shot, then crop by window bounds; RustAutoGUI’s full‑screen capture will be blazing, and the crop math above keeps it correct. (Still keep the xcap fallback for Wayland oddities.) ([GitHub][1])

---

### TL;DR

I skipped RustAutoGUI at first to **guarantee per‑window, cross‑monitor correctness** with **xcap**. RustAutoGUI remains a **great** tool — I’d happily include it as a **“turbo” capture backend** (primary monitor / Linux) and for **fast template matching**. The snippet above shows exactly how to wire it in safely (with DPI fixes and fallbacks), while still saving your original screenshot + smart segments for inspection.

**References**

* RustAutoGUI README & docs (speed claims, multi‑monitor caveat, method list incl. `save_screenshot`, `get_screen_size`). ([GitHub][2], [Docs.rs][3])
* xcap: cross‑platform screen/window capture with examples and OS support table. ([GitHub][1])
* Windows Graphics Capture via `windows-capture` (native, efficient). ([Docs.rs][6])
* macOS ScreenCaptureKit & Screen Recording permission (what the OS requires regardless of library). ([Apple Developer][4], [Apple Support][5])

If you want, I can fold that snippet straight into the single‑file you’re running so it **uses RustAutoGUI by default** where it’s the best fit and **auto‑falls back** to xcap elsewhere.

[1]: https://github.com/nashaofu/xcap "GitHub - nashaofu/xcap: XCap is a cross-platform screen capture library written in Rust. It supports Linux (X11, Wayland), MacOS, and Windows. XCap supports screenshot and video recording (WIP)."
[2]: https://github.com/DavorMar/rustautogui "GitHub - DavorMar/rustautogui: Highly optimized GUI automation rust library for controlling the mouse and keyboard, with template matching support."
[3]: https://docs.rs/rustautogui/latest/rustautogui/struct.RustAutoGui.html "RustAutoGui in rustautogui - Rust"
[4]: https://developer.apple.com/documentation/ScreenCaptureKit/capturing-screen-content-in-macos?utm_source=chatgpt.com "Capturing screen content in macOS"
[5]: https://support.apple.com/guide/mac-help/control-access-screen-system-audio-recording-mchld6aa7d23/mac?utm_source=chatgpt.com "Control access to screen and system audio recording on Mac"
[6]: https://docs.rs/windows-capture?utm_source=chatgpt.com "windows_capture - Rust"
