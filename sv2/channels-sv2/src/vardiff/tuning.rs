//! Live-tunable vardiff parameters, for interactive controller tuning.
//!
//! Process-global like [`super::sim_clock`]: a simulator embedding the pool
//! in its own process can turn these knobs at runtime and watch the
//! controller respond. Production pools simply never touch them and get the
//! defaults.

use std::sync::atomic::{AtomicU64, Ordering};

/// Default confidence shrinkage constant `K` in `w = N_eff / (N_eff + K)`.
/// Larger values discount thin measurement windows more aggressively.
pub const DEFAULT_CONFIDENCE_K: f64 = 2.0;

static CONFIDENCE_K: AtomicU64 = AtomicU64::new(u64::MAX);

/// Current confidence shrinkage constant.
pub fn confidence_k() -> f64 {
    let bits = CONFIDENCE_K.load(Ordering::Relaxed);
    if bits == u64::MAX {
        DEFAULT_CONFIDENCE_K
    } else {
        f64::from_bits(bits)
    }
}

/// Sets the confidence shrinkage constant (clamped to `[0.0, 64.0]`;
/// 0 disables confidence weighting entirely).
pub fn set_confidence_k(k: f64) {
    let k = if k.is_finite() { k.clamp(0.0, 64.0) } else { DEFAULT_CONFIDENCE_K };
    CONFIDENCE_K.store(k.to_bits(), Ordering::Relaxed);
}

static SIGNIFICANCE_Z: AtomicU64 = AtomicU64::new(u64::MAX);

/// Live override for the significance threshold Z (sigmas). `None` means
/// controllers use their configured [`super::pid::PidParams::significance_z`].
pub fn significance_z_override() -> Option<f64> {
    let bits = SIGNIFICANCE_Z.load(Ordering::Relaxed);
    if bits == u64::MAX {
        None
    } else {
        Some(f64::from_bits(bits))
    }
}

/// Sets the live significance-Z override (clamped to `[0.5, 10.0]`).
pub fn set_significance_z(z: f64) {
    if z.is_finite() {
        SIGNIFICANCE_Z.store(z.clamp(0.5, 10.0).to_bits(), Ordering::Relaxed);
    }
}

static SIGNIFICANCE_Z_DOWN: AtomicU64 = AtomicU64::new(u64::MAX);

/// Live override for the downward significance threshold. `None` means
/// controllers use their configured value. The effective down-bar is always
/// capped at the effective up-bar.
pub fn significance_z_down_override() -> Option<f64> {
    let bits = SIGNIFICANCE_Z_DOWN.load(Ordering::Relaxed);
    if bits == u64::MAX {
        None
    } else {
        Some(f64::from_bits(bits))
    }
}

/// Sets the live downward significance override (clamped to `[0.5, 10.0]`).
pub fn set_significance_z_down(z: f64) {
    if z.is_finite() {
        SIGNIFICANCE_Z_DOWN.store(z.clamp(0.5, 10.0).to_bits(), Ordering::Relaxed);
    }
}

static SHARES_PER_MINUTE: AtomicU64 = AtomicU64::new(u64::MAX);

/// Live override for the vardiff setpoint (shares per minute). `None` means
/// the pool uses its configured value. The setpoint is the control system's
/// sample rate: raising it buys reaction latency linearly at the cost of
/// share bandwidth.
pub fn shares_per_minute_override() -> Option<f64> {
    let bits = SHARES_PER_MINUTE.load(Ordering::Relaxed);
    if bits == u64::MAX {
        None
    } else {
        Some(f64::from_bits(bits))
    }
}

/// Sets the live setpoint override (clamped to `[0.5, 600.0]` spm).
pub fn set_shares_per_minute(spm: f64) {
    if spm.is_finite() {
        SHARES_PER_MINUTE.store(spm.clamp(0.5, 600.0).to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_then_sets_then_clamps() {
        assert_eq!(confidence_k(), DEFAULT_CONFIDENCE_K);
        set_confidence_k(5.0);
        assert_eq!(confidence_k(), 5.0);
        set_confidence_k(-3.0);
        assert_eq!(confidence_k(), 0.0);
        set_confidence_k(1e9);
        assert_eq!(confidence_k(), 64.0);
        set_confidence_k(DEFAULT_CONFIDENCE_K);
    }

    #[test]
    fn significance_override_defaults_to_none_and_clamps() {
        assert_eq!(significance_z_override(), None);
        set_significance_z(2.0);
        assert_eq!(significance_z_override(), Some(2.0));
        set_significance_z(100.0);
        assert_eq!(significance_z_override(), Some(10.0));
        // Restore process-global state for other tests.
        SIGNIFICANCE_Z.store(u64::MAX, Ordering::Relaxed);
    }
}
