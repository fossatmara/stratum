//! Scalable virtual clock for vardiff, FOR SIMULATION ONLY.
//!
//! All vardiff implementations read time through [`now_secs`] instead of
//! `SystemTime` directly. At the default scale of 1.0 this is exactly the
//! wall clock. A simulator running in the same process can call [`set_scale`]
//! to compress time (e.g. scale 8.0 makes one wall second count as eight
//! vardiff seconds), which accelerates measurement windows, threshold ladders
//! and integral accumulation consistently — provided the share generator and
//! the evaluation loop are scaled by the same factor.
//!
//! The virtual clock is continuous across scale changes: changing the scale
//! re-anchors at the current virtual instant, so timestamps never jump.

use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

struct ClockState {
    anchor_wall: f64,
    anchor_virtual: f64,
    scale: f64,
}

static STATE: Mutex<Option<ClockState>> = Mutex::new(None);

fn wall_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Sets the time scale factor (clamped to `[0.01, 1024]`). 1.0 = wall clock.
pub fn set_scale(scale: f64) {
    let scale = scale.clamp(0.01, 1024.0);
    let mut state = STATE.lock().expect("sim clock lock poisoned");
    let wall = wall_now();
    let virtual_now = match state.as_ref() {
        Some(s) => s.anchor_virtual + (wall - s.anchor_wall) * s.scale,
        None => wall,
    };
    *state = Some(ClockState {
        anchor_wall: wall,
        anchor_virtual: virtual_now,
        scale,
    });
}

/// Current time scale factor (1.0 unless a simulator changed it).
pub fn scale() -> f64 {
    STATE
        .lock()
        .expect("sim clock lock poisoned")
        .as_ref()
        .map(|s| s.scale)
        .unwrap_or(1.0)
}

/// Current virtual time as fractional seconds since the Unix epoch.
pub fn now_secs_f64() -> f64 {
    let state = STATE.lock().expect("sim clock lock poisoned");
    let wall = wall_now();
    match state.as_ref() {
        Some(s) => s.anchor_virtual + (wall - s.anchor_wall) * s.scale,
        None => wall,
    }
}

/// Current virtual time in whole seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    now_secs_f64() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scale_tracks_wall_clock() {
        // Do not set_scale here: other tests may run concurrently and the
        // clock is process-global, so only check coarse agreement.
        let wall = wall_now() as u64;
        let virt = now_secs();
        assert!(virt.abs_diff(wall) < 24 * 3600 * 1024);
    }

    #[test]
    fn scale_is_clamped() {
        set_scale(0.0);
        assert!(scale() >= 0.01);
        set_scale(f64::INFINITY);
        assert!(scale() <= 1024.0);
        set_scale(1.0);
    }
}
