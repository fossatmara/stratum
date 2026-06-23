use crate::vardiff::clock::{Clock, SystemClock};
use bitcoin::Target;
use std::sync::Arc;

/// Default minimum hashrate (H/s) if not specified.
const DEFAULT_MIN_HASHRATE: f32 = 1.0;

use super::{error::VardiffError, Vardiff};

/// Variable difficulty controller — the production champion.
///
/// Delegates to [`composed::champion_composed`]: `EwmaEstimator(τ=360s) +
/// AdaptiveSignPersist(spm_threshold=6) + AcceleratingPartialRetarget(0.2,
/// 0.6, 0.05)`. This is the configuration the simulation framework selected
/// by minimax over the target share rate under a decline-safety constraint
/// (see `sim/docs/METRIC_DERIVATION.md`): the gentlest controller that stays
/// decline-safe across the rate band.
///
/// The classic algorithm's three-stage decomposition remains available as
/// [`composed::classic_composed`] and is what the simulation crate's
/// equivalence suite pins against the original monolith; it is no longer the
/// production path.
#[derive(Debug)]
pub struct VardiffState {
    inner: Box<dyn Vardiff>,
}

impl std::panic::UnwindSafe for VardiffState {}
impl std::panic::RefUnwindSafe for VardiffState {}

impl VardiffState {
    /// Creates a new `VardiffState` with the default minimum hashrate.
    pub fn new() -> Result<Self, VardiffError> {
        Self::new_with_min(DEFAULT_MIN_HASHRATE)
    }

    /// Creates a new `VardiffState` with a specific minimum hashrate.
    pub fn new_with_min(min_allowed_hashrate: f32) -> Result<Self, VardiffError> {
        Self::new_with_clock(min_allowed_hashrate, Arc::new(SystemClock))
    }

    /// Creates a new `VardiffState` with a specific minimum hashrate and
    /// a custom clock (for simulation/testing).
    pub fn new_with_clock(
        min_allowed_hashrate: f32,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, VardiffError> {
        Ok(VardiffState {
            inner: Box::new(crate::vardiff::composed::champion_composed(
                min_allowed_hashrate,
                clock,
            )),
        })
    }
}

impl Vardiff for VardiffState {
    fn last_update_timestamp(&self) -> u64 {
        self.inner.last_update_timestamp()
    }

    fn shares_since_last_update(&self) -> u32 {
        self.inner.shares_since_last_update()
    }

    fn min_allowed_hashrate(&self) -> f32 {
        self.inner.min_allowed_hashrate()
    }

    fn set_timestamp_of_last_update(&mut self, timestamp: u64) {
        self.inner.set_timestamp_of_last_update(timestamp);
    }

    fn increment_shares_since_last_update(&mut self) {
        self.inner.increment_shares_since_last_update();
    }

    fn add_shares(&mut self, n: u32) {
        self.inner.add_shares(n);
    }

    fn reset_counter(&mut self) -> Result<(), VardiffError> {
        self.inner.reset_counter()
    }

    fn try_vardiff(
        &mut self,
        hashrate: f32,
        target: &Target,
        shares_per_minute: f32,
    ) -> Result<Option<f32>, VardiffError> {
        self.inner.try_vardiff(hashrate, target, shares_per_minute)
    }
}
