use anyhow::{anyhow, Context, Result};
use chrono::Local;
use image::{imageops::crop_imm, io::Reader as ImageReader, DynamicImage, GenericImageView, Rgb};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<()> {
    // 1) Resolve output directory
    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let out_dir = PathBuf::from("bin/chrome").join(format!("screenshot_{}", timestamp));
    fs::create_dir_all(&out_dir).context("Failed to create output directory")?;

    // 2) Ask Chrome for the front window bounds via AppleScript
    let (x, y, w, h) = get_chrome_front_window_bounds()
        .context("Failed to get Chrome front window bounds via AppleScript")?;

    // 3) Use screencapture to capture only that rect
    let original_path = out_dir.join("original.png");
    capture_rect_screenshot(x, y, w, h, &original_path).with_context(|| {
        format!(
            "Failed to capture screenshot to {}",
            original_path.display()
        )
    })?;

    // 4) Load the screenshot
    let img = ImageReader::open(&original_path)
        .with_context(|| format!("Failed to open {}", original_path.display()))?
        .decode()
        .context("Failed to decode image")?;

    // 5) Perform simple color-based segmentation (HSV-based mask + connected components)
    let segments_out = out_dir.clone();
    let min_area: u32 = 200; // filter very small components
    let hsv_s_threshold: f32 = 0.20;
    let hsv_v_min: f32 = 0.20;
    let hsv_v_max: f32 = 0.98;

    let bboxes = segment_color_regions(&img, hsv_s_threshold, hsv_v_min, hsv_v_max, min_area);

    // 6) Crop and save segments
    for (i, (bx, by, bw, bh)) in bboxes.iter().enumerate() {
        let crop = crop_imm(&img, *bx, *by, *bw, *bh).to_image();
        let seg_path = segments_out.join(format!("segment_{:03}.png", i));
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

fn get_chrome_front_window_bounds() -> Result<(u32, u32, u32, u32)> {
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
    let x: u32 = parts[0].trim().parse()?;
    let y: u32 = parts[1].trim().parse()?;
    let w: u32 = parts[2].trim().parse()?;
    let h: u32 = parts[3].trim().parse()?;
    Ok((x, y, w, h));
}

fn capture_rect_screenshot(x: u32, y: u32, w: u32, h: u32, out_path: &Path) -> Result<()> {
    // Use macOS screencapture with -R x,y,w,h to capture a rectangle
    let rect_arg = format!("{}, {}, {}, {}", x, y, w, h).replace(' ', "");
    let status = Command::new("screencapture")
        .args(["-x", "-R", &rect_arg, out_path.to_string_lossy().as_ref()])
        .status()
        .context("Failed to run screencapture")?;

    if !status.success() {
        return Err(anyhow!("screencapture failed with status: {:?}", status));
    }
    Ok(())
}

fn segment_color_regions(
    img: &DynamicImage,
    s_threshold: f32,
    v_min: f32,
    v_max: f32,
    min_area: u32,
) -> Vec<(u32, u32, u32, u32)> {
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();

    // Build a binary mask for pixels that are likely to be colorful UI elements
    let mut mask = vec![false; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let p = rgb.get_pixel(x, y);
            let (h, s, v) = rgb_to_hsv(p.0[0], p.0[1], p.0[2]);
            let _ = h; // hue is not used in this simple pass
            if s >= s_threshold && v >= v_min && v <= v_max {
                mask[(y * width + x) as usize] = true;
            }
        }
    }

    // Connected components (4-neighborhood) to get bounding boxes
    let mut visited = vec![false; mask.len()];
    let mut bboxes: Vec<(u32, u32, u32, u32)> = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            if !mask[idx] || visited[idx] {
                continue;
            }

            // BFS/DFS
            let mut queue = vec![(x, y)];
            visited[idx] = true;
            let mut min_x = x;
            let mut max_x = x;
            let mut min_y = y;
            let mut max_y = y;
            let mut area: u32 = 0;

            while let Some((cx, cy)) = queue.pop() {
                area += 1;
                if cx < min_x {
                    min_x = cx;
                }
                if cx > max_x {
                    max_x = cx;
                }
                if cy < min_y {
                    min_y = cy;
                }
                if cy > max_y {
                    max_y = cy;
                }

                // neighbors
                let neighbors = [
                    (cx.wrapping_sub(1), cy, cx > 0),
                    (cx + 1, cy, cx + 1 < width),
                    (cx, cy.wrapping_sub(1), cy > 0),
                    (cx, cy + 1, cy + 1 < height),
                ];

                for (nx, ny, ok) in neighbors {
                    if !ok {
                        continue;
                    }
                    let nidx = (ny * width + nx) as usize;
                    if mask[nidx] && !visited[nidx] {
                        visited[nidx] = true;
                        queue.push((nx, ny));
                    }
                }
            }

            let bw = max_x - min_x + 1;
            let bh = max_y - min_y + 1;
            if area >= min_area {
                bboxes.push((min_x, min_y, bw, bh));
            }
        }
    }

    // Optionally merge overlapping boxes (simple pass)
    merge_overlapping_bboxes(&mut bboxes);
    bboxes
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
