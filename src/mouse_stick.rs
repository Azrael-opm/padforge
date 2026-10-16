//! Mouse velocity -> analog stick deflection.
//!
//! A mouse reports *displacement* while a stick encodes *velocity*, so we
//! estimate mouse speed over a short sliding window and map it through a
//! response curve. Key properties:
//! * Adaptive window: never shorter than ~1.5x the mouse's report interval,
//!   so 125 Hz and 8 kHz mice both produce a steady signal.
//! * Anti-deadzone: games ignore the inner ~10-25 % of stick travel; we start
//!   every non-zero output beyond that so micro-adjustments still register.
//! * Release ramp: brief gaps between slow mouse reports don't snap the stick
//!   to centre (which feels like stutter).
//! * No heap allocation; fixed ring buffer.

use crate::config::MouseConfig;
use std::time::{Duration, Instant};

const RING: usize = 256;

#[derive(Copy, Clone)]
struct Sample {
    t: Instant,
    dx: f32,
    dy: f32,
}

pub struct MouseStick {
    ring: [Option<Sample>; RING],
    head: usize, // next write
    len: usize,
    sum_x: f32,
    sum_y: f32,
    last_sample: Option<Instant>,
    /// Smoothed estimate of the device report interval in ms.
    interval_ms: f32,
    /// Last non-zero output and when the window became empty.
    last_out: (f32, f32),
    empty_since: Option<Instant>,
}

impl Default for MouseStick {
    fn default() -> Self {
        Self::new()
    }
}

impl MouseStick {
    pub fn new() -> Self {
        MouseStick {
            ring: [None; RING],
            head: 0,
            len: 0,
            sum_x: 0.0,
            sum_y: 0.0,
            last_sample: None,
            interval_ms: 1.0,
            last_out: (0.0, 0.0),
            empty_since: None,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn push(&mut self, now: Instant, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        if let Some(prev) = self.last_sample {
            let dt = now.saturating_duration_since(prev).as_secs_f32() * 1000.0;
            if dt > 0.05 && dt < 40.0 {
                self.interval_ms += (dt - self.interval_ms) * 0.1;
            }
        }
        self.last_sample = Some(now);
        if self.len == RING {
            // Overwrite oldest.
            let tail = self.head;
            if let Some(old) = self.ring[tail].take() {
                self.sum_x -= old.dx;
                self.sum_y -= old.dy;
            }
            self.len -= 1;
        }
        self.ring[self.head] = Some(Sample { t: now, dx, dy });
        self.head = (self.head + 1) % RING;
        self.len += 1;
        self.sum_x += dx;
        self.sum_y += dy;
        self.empty_since = None;
    }

    fn window_ms(&self, cfg: &MouseConfig) -> f32 {
        cfg.smoothing_ms.max(self.interval_ms * 1.5).max(1.0)
    }

    fn prune(&mut self, now: Instant, window: Duration) {
        while self.len > 0 {
            let tail = (self.head + RING - self.len) % RING;
            let s = self.ring[tail].expect("ring invariant");
            if now.saturating_duration_since(s.t) >= window {
                self.ring[tail] = None;
                self.sum_x -= s.dx;
                self.sum_y -= s.dy;
                self.len -= 1;
            } else {
                break;
            }
        }
        if self.len == 0 {
            // Kill float drift.
            self.sum_x = 0.0;
            self.sum_y = 0.0;
        }
    }

    /// True while the output may still change without new input (needs ticks).
    pub fn is_animating(&self) -> bool {
        self.len > 0 || self.last_out != (0.0, 0.0)
    }

    /// Current stick output in [-1,1]^2 (circle), +y = up.
    pub fn output(&mut self, now: Instant, cfg: &MouseConfig) -> (f32, f32) {
        let win_ms = self.window_ms(cfg);
        self.prune(now, Duration::from_secs_f32(win_ms / 1000.0));

        if self.len == 0 {
            // Release ramp from the last output towards centre, measured from
            // the moment the newest sample left the window (not from when we
            // happen to observe it, which depends on scheduling).
            let since = match (self.empty_since, self.last_sample) {
                (Some(t), _) => t,
                (None, Some(last)) => {
                    let t = last + Duration::from_secs_f32(win_ms / 1000.0);
                    self.empty_since = Some(t);
                    t
                }
                (None, None) => now,
            };
            let el = now.saturating_duration_since(since).as_secs_f32() * 1000.0;
            if cfg.release_ms <= 0.0 || el >= cfg.release_ms {
                self.last_out = (0.0, 0.0);
                return (0.0, 0.0);
            }
            let k = 1.0 - el / cfg.release_ms;
            return (self.last_out.0 * k, self.last_out.1 * k);
        }

        let vx = self.sum_x / win_ms; // counts per ms
        let vy = self.sum_y / win_ms;
        let out = shape(vx, vy, cfg);
        self.last_out = out;
        out
    }
}

/// Apply sensitivity, axis ratio/inversion, response curve and anti-deadzone.
/// Input: mouse velocity in counts/ms (screen coords, +y = down).
pub fn shape(vx: f32, vy: f32, cfg: &MouseConfig) -> (f32, f32) {
    let sy_sign = if cfg.invert_y { 1.0 } else { -1.0 };
    let sx = vx * cfg.sensitivity;
    let sy = vy * cfg.sensitivity * cfg.y_ratio * sy_sign;
    let m = (sx * sx + sy * sy).sqrt();
    if m < 1e-6 {
        return (0.0, 0.0);
    }
    let curved = m.min(1.0).powf(cfg.exponent);
    let adz = cfg.anti_deadzone.clamp(0.0, 0.95);
    let out_m = (adz + (1.0 - adz) * curved).min(1.0);
    (sx / m * out_m, sy / m * out_m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MouseConfig {
        MouseConfig { anti_deadzone: 0.2, exponent: 1.0, sensitivity: 0.1, smoothing_ms: 4.0, release_ms: 10.0, ..Default::default() }
    }

    #[test]
    fn idle_is_centered() {
        let mut m = MouseStick::new();
        assert_eq!(m.output(Instant::now(), &cfg()), (0.0, 0.0));
        assert!(!m.is_animating());
    }

    #[test]
    fn direction_and_y_inversion() {
        let c = cfg();
        let t0 = Instant::now();
        let mut m = MouseStick::new();
        for i in 0..4 {
            m.push(t0 + Duration::from_millis(i), 2.0, 0.0);
        }
        let (x, y) = m.output(t0 + Duration::from_millis(3), &c);
        assert!(x > 0.2 && y.abs() < 1e-6, "{x} {y}");
        // Mouse moving down (+dy) pushes the stick down (-y) by default.
        let mut m = MouseStick::new();
        m.push(t0, 0.0, 5.0);
        let (_, y) = m.output(t0, &c);
        assert!(y < 0.0);
        let mut ci = c.clone();
        ci.invert_y = true;
        let mut m = MouseStick::new();
        m.push(t0, 0.0, 5.0);
        assert!(m.output(t0, &ci).1 > 0.0);
    }

    #[test]
    fn anti_deadzone_floor_and_saturation() {
        let c = cfg();
        let (x, _) = shape(0.001, 0.0, &c);
        assert!((0.2..0.21).contains(&x));
        let (x, y) = shape(1000.0, 1000.0, &c);
        assert!(((x * x + y * y).sqrt() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn returns_to_center_after_window_and_release() {
        let c = cfg();
        let t0 = Instant::now();
        let mut m = MouseStick::new();
        m.push(t0, 10.0, 0.0);
        assert!(m.output(t0, &c).0 > 0.0);
        // Window (4ms) elapsed -> release ramp starts, still non-zero.
        let mid = m.output(t0 + Duration::from_millis(5), &c).0;
        assert!(mid > 0.0);
        let mid2 = m.output(t0 + Duration::from_millis(10), &c).0;
        assert!(mid2 < mid);
        assert_eq!(m.output(t0 + Duration::from_millis(30), &c), (0.0, 0.0));
        assert!(!m.is_animating());
    }

    #[test]
    fn adaptive_window_for_slow_polling_mouse() {
        // 125 Hz mouse (8ms) with a 4ms smoothing setting must not drop to 0
        // between reports.
        let c = MouseConfig { release_ms: 0.0, ..cfg() };
        let t0 = Instant::now();
        let mut m = MouseStick::new();
        let mut zeros = 0;
        for i in 0..200u64 {
            let t = t0 + Duration::from_millis(i * 8);
            m.push(t, 8.0, 0.0);
            for k in 1..8 {
                if m.output(t + Duration::from_millis(k), &c).0 == 0.0 {
                    zeros += 1;
                }
            }
        }
        // Allow the first few while the interval estimate converges.
        assert!(zeros < 60, "zeros={zeros}");
    }
}
