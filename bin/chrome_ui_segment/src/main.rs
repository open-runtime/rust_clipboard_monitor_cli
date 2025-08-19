use active_win_pos_rs::get_active_window;
use anyhow::{anyhow, Context, Result};
use chrono::Local;
use image::{self, DynamicImage, GenericImageView, ImageReader, Rgba, RgbaImage};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<()> {
    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    // Resolve app folder from active window (fallback to "unknown")
    let app_folder = get_active_window()
        .ok()
        .map(|w| sanitize_name(&w.app_name))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let out_dir =
        PathBuf::from(format!("../{}", app_folder)).join(format!("screenshot_{}", timestamp));
    fs::create_dir_all(&out_dir).context("Failed to create output directory")?;

    // Cross-app: try cross-platform active window geometry first
    let (x, y, w, h) = get_active_bounds()
        .or_else(|_| get_chrome_front_window_bounds())
        .context("Failed to get active window bounds")?;

    let original_path = out_dir.join("original.png");
    // Capture active window region via OS tools (with rustautogui fallback)
    capture_rect(x, y, w, h, &original_path)?;

    let img = ImageReader::open(&original_path)
        .with_context(|| format!("Failed to open {}", original_path.display()))?
        .decode()
        .context("Failed to decode image")?;

    // Horizontal probes: detect vertical boundaries from long uniform color runs
    let (probe_cuts, axis_overlay) = compute_horizontal_probe_debug(&img, 50, 28.0, 6);
    let _ = axis_overlay.save(out_dir.join("debug_axis.png"));

    // Replace pixel-level color blobs with large layout blocks
    let bboxes = segment_layout_blocks(&img);
    // Also save a best-effort visual hierarchy set
    let _ = write_hierarchy_crops(&img, &bboxes, &out_dir);

    for (i, (bx, by, bw, bh)) in bboxes.iter().enumerate() {
        let crop = image::imageops::crop_imm(&img, *bx, *by, *bw, *bh).to_image();
        let seg_path = out_dir.join(format!("segment_{:03}.png", i));
        crop.save(&seg_path)
            .with_context(|| format!("Failed to save {}", seg_path.display()))?;
    }

    println!(
        "Saved {} segment(s) to {}\nOriginal: {}",
        bboxes.len(),
        out_dir.display(),
        original_path.display()
    );

    Ok(())
}

// Compute vertical boundaries by scanning a few horizontal probe rows. A boundary exists
// where N consecutive pixels are near a color A, followed by N consecutive pixels near color B.
// Returns clustered cut x-positions and a debug overlay image with lines and probe hits.
fn compute_horizontal_probe_debug(
    img: &DynamicImage,
    run_len: u32,
    tol: f32,
    num_scans: usize,
) -> (Vec<u32>, RgbaImage) {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut events: Vec<(u32, u32)> = Vec::new();
    if w < run_len * 2 || h == 0 || num_scans == 0 {
        return (vec![], rgba);
    }

    let sample_rows: Vec<u32> = (0..num_scans)
        .map(|i| ((i as f32 + 0.5) / num_scans as f32 * h as f32).round() as u32)
        .map(|y| y.min(h.saturating_sub(1)))
        .collect();

    for &y in &sample_rows {
        for x in probe_row(&rgba, y, run_len, tol) {
            events.push((x, y));
        }
    }
    let mut xs: Vec<u32> = events.iter().map(|(x, _)| *x).collect();
    xs = cluster_positions(xs, ((w as f32) * 0.015).round() as u32, 2, w);

    // Build overlay: screenshot as background, red dots for raw events, yellow lines for cuts
    let mut overlay = rgba.clone();
    for (x, y) in events {
        let p = overlay.get_pixel_mut(x.min(w - 1), y.min(h - 1));
        // red dot
        p[0] = 255;
        p[1] = p[1].max(40);
        p[2] = p[2].max(40);
    }
    for x in xs.iter().copied() {
        let xx = x.min(w.saturating_sub(1));
        for y in 0..h {
            let p = overlay.get_pixel_mut(xx, y);
            // yellow line
            p[0] = 255;
            p[1] = 255;
        }
    }
    (xs, overlay)
}

fn probe_row(img: &RgbaImage, y: u32, run_len: u32, tol: f32) -> Vec<u32> {
    let w = img.width();
    let mut out = Vec::new();
    if w < run_len * 2 {
        return out;
    }
    let mut x: u32 = 0;
    while x + run_len * 2 <= w {
        let (ok_a, mean_a) = color_run_ok(img, y, x, run_len, tol);
        if !ok_a {
            x += 1;
            continue;
        }
        let (ok_b, mean_b) = color_run_ok(img, y, x + run_len, run_len, tol);
        if ok_b && rgb_distance(mean_a, mean_b) > tol {
            out.push(x + run_len);
            x += run_len; // skip forward to avoid dense duplicates
        } else {
            x += 1;
        }
    }
    out
}

fn color_run_ok(img: &RgbaImage, y: u32, start_x: u32, len: u32, tol: f32) -> (bool, [f32; 3]) {
    let mut sum = [0f32; 3];
    let mut count = 0u32;
    let w = img.width();
    for x in start_x..(start_x + len).min(w) {
        let p = img.get_pixel(x, y);
        let rgb = [p[0] as f32, p[1] as f32, p[2] as f32];
        sum[0] += rgb[0];
        sum[1] += rgb[1];
        sum[2] += rgb[2];
        count += 1;
    }
    if count < len {
        return (false, [0.0, 0.0, 0.0]);
    }
    let mean = [
        sum[0] / count as f32,
        sum[1] / count as f32,
        sum[2] / count as f32,
    ];
    for x in start_x..(start_x + len).min(w) {
        let p = img.get_pixel(x, y);
        let rgb = [p[0] as f32, p[1] as f32, p[2] as f32];
        if rgb_distance(rgb, mean) > tol {
            return (false, mean);
        }
    }
    (true, mean)
}

fn rgb_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dr = a[0] - b[0];
    let dg = a[1] - b[1];
    let db = a[2] - b[2];
    (dr * dr + dg * dg + db * db).sqrt()
}

fn cluster_positions(mut xs: Vec<u32>, radius: u32, min_votes: usize, max_edge: u32) -> Vec<u32> {
    if xs.is_empty() {
        return vec![];
    }
    xs.sort_unstable();
    let mut clusters: Vec<Vec<u32>> = Vec::new();
    for x in xs {
        if let Some(last) = clusters.last_mut() {
            let last_val = *last.last().unwrap();
            if (x as i64 - last_val as i64).unsigned_abs() <= radius as u64 {
                last.push(x);
            } else {
                clusters.push(vec![x]);
            }
        } else {
            clusters.push(vec![x]);
        }
    }
    let mut out = Vec::new();
    for c in clusters {
        if c.len() >= min_votes {
            let avg = (c.iter().map(|v| *v as u64).sum::<u64>() / c.len() as u64) as u32;
            if avg > 0 && avg < max_edge {
                out.push(avg);
            }
        }
    }
    out
}

fn get_active_bounds() -> Result<(i32, i32, u32, u32)> {
    match get_active_window() {
        Ok(win) => Ok((
            win.position.x as i32,
            win.position.y as i32,
            win.position.width as u32,
            win.position.height as u32,
        )),
        Err(_) => Err(anyhow!("No active window")),
    }
}

fn sanitize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let ok = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.';
        out.push(if ok { ch } else { '-' });
    }
    out.trim_matches('-').to_lowercase()
}

fn get_chrome_front_window_bounds() -> Result<(i32, i32, u32, u32)> {
    let script = r#"
        tell application "Google Chrome"
            activate
            if (count of windows) = 0 then return ""
            set b to bounds of front window
            return (item 1 of b as string) & "," & (item 2 of b as string) & "," & (item 3 of b as string) & "," & (item 4 of b as string)
        end tell
    "#;

    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .context("Failed to execute osascript")?;

    if !output.status.success() {
        return Err(anyhow!(
            "osascript error: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() || stdout == "missing value" {
        return Err(anyhow!("No Chrome window detected"));
    }

    let parts: Vec<&str> = stdout.split(',').collect();
    if parts.len() != 4 {
        return Err(anyhow!("Unexpected bounds format: {}", stdout));
    }
    let left: i32 = parts[0].trim().parse()?;
    let top: i32 = parts[1].trim().parse()?;
    let right: i32 = parts[2].trim().parse()?;
    let bottom: i32 = parts[3].trim().parse()?;

    let w = (right - left).max(0) as u32;
    let h = (bottom - top).max(0) as u32;
    Ok((left, top, w, h))
}

fn capture_rect(x: i32, y: i32, w: u32, h: u32, out_path: &Path) -> Result<()> {
    // First, try precise OS region capture for multi-monitor correctness
    if let Err(e) = capture_rect_via_screencapture(x, y, w, h, out_path) {
        eprintln!(
            "screencapture failed, falling back to rustautogui crop: {}",
            e
        );
        capture_rect_with_rustautogui(x, y, w, h, out_path)?;
    }
    Ok(())
}

fn capture_rect_via_screencapture(x: i32, y: i32, w: u32, h: u32, out_path: &Path) -> Result<()> {
    let rect_arg = format!("{}, {}, {}, {}", x, y, w, h).replace(' ', "");
    let status = Command::new("screencapture")
        .args(["-x", "-R", &rect_arg, out_path.to_string_lossy().as_ref()])
        .status()
        .context("Failed to run screencapture")?;
    if !status.success() {
        return Err(anyhow!("screencapture exited with {:?}", status));
    }
    Ok(())
}

fn capture_rect_with_rustautogui(x: i32, y: i32, w: u32, h: u32, out_path: &Path) -> Result<()> {
    // Use rustautogui to capture full screen, then crop the requested region.
    // On macOS Retina, account for scaling factor between logical (bounds from AppleScript)
    // and physical pixel buffer captured by rustautogui.
    let mut gui =
        rustautogui::RustAutoGui::new(false).context("Failed to initialize RustAutoGui")?;

    // Save a full screenshot to a temp file, then crop in memory.
    let tmp_full = out_path.with_file_name("__full_tmp.png");
    gui.save_screenshot(tmp_full.to_string_lossy().as_ref())
        .context("Failed to capture full screenshot with rustautogui")?;

    // Determine scaling using gui.get_screen_size() vs loaded image size
    let (logical_w, logical_h) = gui.get_screen_size();
    let full_img = ImageReader::open(&tmp_full)
        .with_context(|| format!("Failed to open {}", tmp_full.display()))?
        .decode()
        .context("Failed to decode full screenshot")?;
    let (phys_w, phys_h) = full_img.dimensions();

    let scale_x = phys_w as f32 / logical_w.max(1) as f32;
    let scale_y = phys_h as f32 / logical_h.max(1) as f32;

    // Scale the Chrome bounds to physical pixels and crop
    let sx = (x as f32 * scale_x).round() as i32;
    let sy = (y as f32 * scale_y).round() as i32;
    let sw = (w as f32 * scale_x).round() as i32;
    let sh = (h as f32 * scale_y).round() as i32;

    let mut bx = sx.max(0) as u32;
    let mut by = sy.max(0) as u32;
    let mut bw = sw.max(1) as u32;
    let mut bh = sh.max(1) as u32;
    if bx >= phys_w {
        bx = phys_w.saturating_sub(1);
    }
    if by >= phys_h {
        by = phys_h.saturating_sub(1);
    }
    bw = bw.min(phys_w.saturating_sub(bx));
    bh = bh.min(phys_h.saturating_sub(by));

    let cropped = image::imageops::crop_imm(&full_img, bx, by, bw.max(1), bh.max(1)).to_image();
    cropped
        .save(out_path)
        .with_context(|| format!("Failed to save {}", out_path.display()))?;

    // Clean up temp
    let _ = std::fs::remove_file(&tmp_full);
    Ok(())
}

// removed xcap/display-info path; we rely on OS capture + rustautogui fallback

fn segment_layout_blocks(img: &DynamicImage) -> Vec<(u32, u32, u32, u32)> {
    // Grid-based color-mean clustering to form large layout blocks
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();

    // Choose grid so that cell is ~24-32 px wide
    let target_cell = 28u32;
    let cols = ((width.max(1) + target_cell - 1) / target_cell).clamp(16, 96);
    let cell_w = (width.max(1) + cols - 1) / cols;
    let rows = ((height.max(1) + cell_w - 1) / cell_w).clamp(12, 96);
    let cell_h = (height.max(1) + rows - 1) / rows;

    #[inline]
    fn color_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
        let dr = a[0] - b[0];
        let dg = a[1] - b[1];
        let db = a[2] - b[2];
        (dr * dr + dg * dg + db * db).sqrt()
    }

    let mut means = vec![[0f32; 3]; (rows * cols) as usize];
    let mut counts = vec![0u32; (rows * cols) as usize];
    for y in 0..height {
        let gy = (y / cell_h).min(rows - 1);
        for x in 0..width {
            let gx = (x / cell_w).min(cols - 1);
            let idx = (gy * cols + gx) as usize;
            let p = rgb.get_pixel(x, y);
            means[idx][0] += p[0] as f32;
            means[idx][1] += p[1] as f32;
            means[idx][2] += p[2] as f32;
            counts[idx] += 1;
        }
    }
    for i in 0..means.len() {
        let c = counts[i].max(1) as f32;
        means[i][0] /= c;
        means[i][1] /= c;
        means[i][2] /= c;
    }

    // BFS merge neighboring grid cells by color similarity to form large blocks
    let mut visited = vec![false; (rows * cols) as usize];
    let mut blocks = Vec::new();
    let tol = 26.0; // color tolerance in RGB space
    for gy in 0..rows {
        for gx in 0..cols {
            let start = (gy * cols + gx) as usize;
            if visited[start] {
                continue;
            }
            visited[start] = true;
            let seed = means[start];

            let mut q = vec![(gx, gy)];
            let mut min_x = gx;
            let mut max_x = gx;
            let mut min_y = gy;
            let mut max_y = gy;
            let mut cell_count = 0u32;
            while let Some((cx, cy)) = q.pop() {
                cell_count += 1;
                min_x = min_x.min(cx);
                max_x = max_x.max(cx);
                min_y = min_y.min(cy);
                max_y = max_y.max(cy);
                let neigh = [
                    (cx.wrapping_sub(1), cy, cx > 0),
                    (cx + 1, cy, cx + 1 < cols),
                    (cx, cy.wrapping_sub(1), cy > 0),
                    (cx, cy + 1, cy + 1 < rows),
                ];
                for (nx, ny, ok) in neigh {
                    if !ok {
                        continue;
                    }
                    let nidx = (ny * cols + nx) as usize;
                    if visited[nidx] {
                        continue;
                    }
                    if color_dist(means[nidx], seed) <= tol {
                        visited[nidx] = true;
                        q.push((nx, ny));
                    }
                }
            }

            // Convert block in grid coords to pixel bbox, clamp safely
            if cell_count > 0 {
                let bx = (min_x * cell_w).min(width.saturating_sub(1));
                let by = (min_y * cell_h).min(height.saturating_sub(1));
                let max_w = width.saturating_sub(bx);
                let max_h = height.saturating_sub(by);
                let mut bw = ((max_x - min_x + 1) * cell_w).min(max_w);
                let mut bh = ((max_y - min_y + 1) * cell_h).min(max_h);
                if bw == 0 {
                    bw = 1;
                }
                if bh == 0 {
                    bh = 1;
                }
                blocks.push((bx, by, bw, bh));
            }
        }
    }

    // Keep only large blocks (e.g., >3% of image) to approximate header/sidebars/content
    let min_area = (width as u64 * height as u64) / 30; // ~3%
    let mut bboxes: Vec<(u32, u32, u32, u32)> = blocks
        .into_iter()
        .filter(|&(_, _, bw, bh)| (bw as u64 * bh as u64) >= min_area)
        .collect();

    merge_overlapping_bboxes(&mut bboxes);
    bboxes
}

fn write_hierarchy_crops(
    img: &DynamicImage,
    blocks: &[(u32, u32, u32, u32)],
    out_dir: &Path,
) -> Result<()> {
    let (w, h) = img.dimensions();
    let area = |b: &(u32, u32, u32, u32)| -> u64 { b.2 as u64 * b.3 as u64 };
    let mut header: Option<(u32, u32, u32, u32)> = None;
    let mut left: Option<(u32, u32, u32, u32)> = None;
    let mut right: Option<(u32, u32, u32, u32)> = None;
    let mut main: Option<(u32, u32, u32, u32)> = None;

    let top_thresh = (h as f32 * 0.15) as u32;
    let header_max_h = (h as f32 * 0.25) as u32;
    let wide_thresh = (w as f32 * 0.6) as u32;
    let side_thresh_x = (w as f32 * 0.15) as u32;
    let side_max_w = (w as f32 * 0.45) as u32;
    let side_min_h = (h as f32 * 0.4) as u32;

    for &b in blocks {
        let (bx, by, bw, bh) = b;
        if by <= top_thresh && bw >= wide_thresh && bh <= header_max_h {
            if header.as_ref().map(area).unwrap_or(0) < area(&b) {
                header = Some(b);
            }
        }
        if bx <= side_thresh_x && bh >= side_min_h && bw <= side_max_w {
            if left.as_ref().map(area).unwrap_or(0) < area(&b) {
                left = Some(b);
            }
        }
        if bx + bw >= w.saturating_sub(side_thresh_x) && bh >= side_min_h && bw <= side_max_w {
            if right.as_ref().map(area).unwrap_or(0) < area(&b) {
                right = Some(b);
            }
        }
    }

    // main content: largest block not header/sidebars with center inside remaining area
    let mut candidates: Vec<(u32, u32, u32, u32)> = blocks.to_vec();
    if let Some(hd) = header {
        candidates.retain(|b| !intersects(*b, hd));
    }
    if let Some(ls) = left {
        candidates.retain(|b| !intersects(*b, ls));
    }
    if let Some(rs) = right {
        candidates.retain(|b| !intersects(*b, rs));
    }
    if let Some(b) = candidates.into_iter().max_by_key(|b| area(b)) {
        main = Some(b);
    }

    let mut save_crop = |name: &str, b: Option<(u32, u32, u32, u32)>| -> Result<()> {
        if let Some((bx, by, bw, bh)) = b {
            let crop = image::imageops::crop_imm(
                img,
                bx.min(w.saturating_sub(1)),
                by.min(h.saturating_sub(1)),
                bw.min(w.saturating_sub(bx)),
                bh.min(h.saturating_sub(by)),
            )
            .to_image();
            crop.save(out_dir.join(name))
                .with_context(|| format!("Failed to save {}", out_dir.join(name).display()))?;
        }
        Ok(())
    };

    save_crop("header.png", header)?;
    save_crop("left_sidebar.png", left)?;
    save_crop("right_sidebar.png", right)?;
    save_crop("main_content.png", main)?;

    Ok(())
}

fn merge_overlapping_bboxes(boxes: &mut Vec<(u32, u32, u32, u32)>) {
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..boxes.len() {
            for j in (i + 1)..boxes.len() {
                if intersects(boxes[i], boxes[j]) {
                    let merged = merge(boxes[i], boxes[j]);
                    boxes.remove(j);
                    boxes[i] = merged;
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
}

fn intersects(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    let ar = (ax + aw) as i64;
    let ab = (ay + ah) as i64;
    let br = (bx + bw) as i64;
    let bb = (by + bh) as i64;
    !(ar as i64 <= bx as i64
        || br as i64 <= ax as i64
        || ab as i64 <= by as i64
        || bb as i64 <= ay as i64)
}

fn merge(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    let min_x = ax.min(bx);
    let min_y = ay.min(by);
    let max_x = (ax + aw).max(bx + bw);
    let max_y = (ay + ah).max(by + bh);
    (min_x, min_y, max_x - min_x, max_y - min_y)
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;
    let v = max;
    let s = if max == 0.0 { 0.0 } else { delta / max };
    let h = if delta == 0.0 {
        0.0
    } else if max == rf {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if max == gf {
        60.0 * (((bf - rf) / delta) + 2.0)
    } else {
        60.0 * (((rf - gf) / delta) + 4.0)
    };
    let h_norm = if h < 0.0 { h + 360.0 } else { h } / 360.0;
    (h_norm, s, v)
}
