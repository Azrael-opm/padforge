//! Procedurally rendered app / tray icon (no image assets, no decoder deps).
//! A rounded badge in a state colour with a white d-pad and two face buttons.

/// State colours shared by tray and GUI.
pub const COLOR_ON: [u8; 3] = [60, 190, 110];
pub const COLOR_PAUSED: [u8; 3] = [240, 170, 60];
pub const COLOR_OFF: [u8; 3] = [120, 120, 135];
pub const COLOR_BRAND: [u8; 3] = [90, 170, 255];

/// Signed-distance helpers in pixel units; return coverage 0..1 (anti-aliased).
#[inline]
fn cover(dist: f32) -> f32 {
    (0.5 - dist).clamp(0.0, 1.0)
}

fn rounded_rect_sd(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

fn circle_sd(px: f32, py: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r
}

/// Render a `size`×`size` straight-alpha RGBA icon.
pub fn render_rgba(size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let c = s / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Badge.
            let badge = cover(rounded_rect_sd(px, py, c, c, s * 0.47, s * 0.47, s * 0.22));
            // D-pad (cross) on the left.
            let dx = s * 0.33;
            let arm_l = s * 0.15;
            let arm_w = s * 0.055;
            let dpad = cover(rounded_rect_sd(px, py, dx, c, arm_l, arm_w, arm_w * 0.5))
                .max(cover(rounded_rect_sd(px, py, dx, c, arm_w, arm_l, arm_w * 0.5)));
            // Two face buttons on the right.
            let br = s * 0.075;
            let b1 = cover(circle_sd(px, py, s * 0.64, c + s * 0.09, br));
            let b2 = cover(circle_sd(px, py, s * 0.77, c - s * 0.07, br));
            let glyph = dpad.max(b1).max(b2);
            let i = ((y * size + x) * 4) as usize;
            for k in 0..3 {
                let v = rgb[k] as f32 * (1.0 - glyph) + 255.0 * glyph;
                out[i + k] = v.round() as u8;
            }
            out[i + 3] = (badge * 255.0).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn icon_shape() {
        let n = 32u32;
        let px = render_rgba(n, COLOR_ON);
        assert_eq!(px.len(), (n * n * 4) as usize);
        // Corner transparent, centre opaque.
        assert_eq!(px[3], 0);
        let mid = ((n / 2 * n + n / 2) * 4) as usize;
        assert_eq!(px[mid + 3], 255);
        // D-pad centre is white.
        let dp = (((n / 2) * n + (n as f32 * 0.33) as u32) * 4) as usize;
        assert!(px[dp] > 240 && px[dp + 1] > 240);
    }
}
