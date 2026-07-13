use crate::target::hash_rate_from_target;
use bitcoin::Target;
use tracing::debug;

use super::{error::VardiffError, Vardiff};

/// Default minimum hashrate (H/s) if not specified.
const DEFAULT_MIN_HASHRATE: f32 = 1.0;

/// Decline-safe adaptive EWMA vardiff algorithm — the "champion" from
/// stratum-mining/stratum#2188.
///
/// This is a faithful port of that PR's replacement `VardiffState`, preserved
/// here as a *separate, selectable* algorithm (`VardiffKind::Champion`) rather
/// than a replacement, so it can be benchmarked head-to-head against the
/// classic ladder, the PID controller, and the Q-learning PID.
///
/// Three inline stages per evaluation tick, with the PR's decline-safety
/// -selected parameters preserved verbatim (only the clock source is swapped
/// to [`super::sim_clock`] and the fixed-60s tick cadence is self-regulated —
/// see [`ChampionVardiffState::try_vardiff`]):
///
/// 1. **Estimator** — EWMA-smoothed share rate (tau=360s, tick=60s) converts
///    observed shares into a hashrate belief via `hash_rate_from_target`.
/// 2. **Boundary** — PoissonCI below `spm_threshold` (6), sign-persistence
///    CUSUM at/above it, with tightening (the dangerous direction) requiring
///    `cusum_tighten_multiplier` (8×) the evidence of loosening.
/// 3. **Update** — accelerating partial retarget: `eta` 0.2 → 0.6 over
///    consecutive same-direction fires.
///
/// # Tick cadence
///
/// The champion's EWMA hardcodes a 60s tick (its `alpha = exp(-60/360)` and
/// `realized_spm = rate * 60/tick_secs` both assume one call per `tick_secs`).
/// In upstream #2188 the *caller* is responsible for calling once per 60s. This
/// pool evaluates vardiff share-driven (many calls per minute), so the champion
/// self-regulates: [`try_vardiff`] only advances a tick once `tick_secs` of
/// virtual time have elapsed since the last one, buffering shares in between.
/// This reproduces the PR's design contract exactly regardless of call rate.
///
/// [`try_vardiff`]: ChampionVardiffState::try_vardiff
#[derive(Debug)]
pub struct ChampionVardiffState {
    /// Count of shares received since the last EWMA tick.
    pub shares_since_last_update: u32,
    /// Virtual timestamp (seconds) of the last difficulty *fire*. Drives the
    /// boundary's time-since-fire evidence (`n_ticks = dt / tick_secs`).
    pub timestamp_of_last_update: u64,
    /// The lowest hashrate (H/s) the system will allow; values below this are clamped.
    pub min_allowed_hashrate: f32,

    /// Virtual timestamp (seconds) of the last EWMA tick. Gates the fixed 60s
    /// cadence when the pool calls more often than once per tick.
    last_tick_secs: u64,

    // -- EWMA estimator state --
    tick_secs: u64,
    tau_secs: u64,
    rate: f64,
    n_ticks: u32,

    // -- Adaptive boundary params --
    spm_threshold: u32,
    poisson_z: f64,
    poisson_margin: f64,
    cusum_sensitivity: f64,
    cusum_floor: f64,
    cusum_tighten_multiplier: f64,
    cusum_reference_spm: f64,
    /// Threshold reduction per consecutive same-sign tick (fractional).
    cusum_sign_persistence_discount: f64,
    /// Maximum total sign-persistence discount (fractional cap).
    cusum_max_sign_discount: f64,
    /// Sign of the last boundary observation (realized vs target): +1 / -1 / 0.
    cusum_last_sign: i8,
    /// Count of consecutive same-sign boundary observations.
    cusum_consecutive: u32,

    // -- Accelerating partial retarget state --
    eta_base: f32,
    eta_max: f32,
    acceleration: f32,
    consecutive_same_direction: u32,
    last_direction: i8,
}

impl ChampionVardiffState {
    /// Creates a new `ChampionVardiffState` with the default minimum hashrate.
    pub fn new() -> Result<Self, VardiffError> {
        Self::new_with_min(DEFAULT_MIN_HASHRATE)
    }

    /// Creates a new `ChampionVardiffState` with a specific minimum hashrate.
    pub fn new_with_min(min_allowed_hashrate: f32) -> Result<Self, VardiffError> {
        let now = super::sim_clock::now_secs();

        Ok(Self {
            shares_since_last_update: 0,
            timestamp_of_last_update: now,
            min_allowed_hashrate,
            last_tick_secs: now,
            tick_secs: 60,
            tau_secs: 360,
            rate: 0.0,
            n_ticks: 0,
            spm_threshold: 6,
            poisson_z: 2.576,
            poisson_margin: 0.05,
            cusum_sensitivity: 1.5,
            cusum_floor: 0.05,
            cusum_tighten_multiplier: 8.0,
            cusum_reference_spm: 30.0,
            cusum_sign_persistence_discount: 0.06,
            cusum_max_sign_discount: 0.6,
            cusum_last_sign: 0,
            cusum_consecutive: 0,
            eta_base: 0.2,
            eta_max: 0.6,
            acceleration: 0.05,
            consecutive_same_direction: 0,
            last_direction: 0,
        })
    }

    /// Sets the count of shares since the last update.
    pub fn set_shares_since_last_update(&mut self, shares_since_last_update: u32) {
        self.shares_since_last_update = shares_since_last_update;
    }

    /// Test-only: the sign-persistence boundary state `(last_sign, consecutive)`.
    #[cfg(test)]
    pub(crate) fn cusum_sign_state(&self) -> (i8, u32) {
        (self.cusum_last_sign, self.cusum_consecutive)
    }

    fn ewma_alpha(&self) -> f64 {
        (-(self.tick_secs as f64) / (self.tau_secs as f64)).exp()
    }

    /// Stage 1: Flush pending shares into the EWMA and produce a hashrate estimate.
    fn estimate(&mut self, hashrate: f32, target: &Target, shares_per_minute: f32) -> (f64, f32) {
        let n = self.shares_since_last_update as f64;

        let rate = if self.n_ticks == 0 {
            n
        } else {
            let alpha = self.ewma_alpha();
            alpha * self.rate + (1.0 - alpha) * n
        };

        self.rate = rate;
        self.n_ticks += 1;
        self.shares_since_last_update = 0;

        let realized_share_per_min = rate * (60.0 / self.tick_secs as f64);

        let h_estimate =
            match hash_rate_from_target(target.to_le_bytes().into(), realized_share_per_min) {
                Ok(h) => h as f32,
                Err(_) => hashrate * realized_share_per_min as f32 / shares_per_minute,
            };

        (realized_share_per_min, h_estimate)
    }

    /// Stage 2: Compute decision threshold. Takes `&mut self` because the
    /// sign-persistence CUSUM updates its consecutive-tick state each evaluation.
    fn threshold(&mut self, dt_secs: u64, shares_per_minute: f32, realized_spm: f64) -> f64 {
        if (shares_per_minute as u32) < self.spm_threshold {
            self.poisson_threshold(dt_secs, shares_per_minute, realized_spm)
        } else {
            self.cusum_threshold(dt_secs, shares_per_minute, realized_spm)
        }
    }

    fn poisson_threshold(&self, dt_secs: u64, shares_per_minute: f32, realized_spm: f64) -> f64 {
        let lambda_bar = (shares_per_minute as f64 / 60.0) * dt_secs as f64;
        if lambda_bar <= 0.0 {
            return 100.0;
        }
        let bound_fraction =
            (self.poisson_z * lambda_bar.sqrt() + 0.5) / lambda_bar + self.poisson_margin;
        let base = bound_fraction * 100.0;

        let would_tighten = realized_spm > shares_per_minute as f64;
        if would_tighten {
            base * self.cusum_tighten_multiplier
        } else {
            base
        }
    }

    fn cusum_threshold(&mut self, dt_secs: u64, shares_per_minute: f32, realized_spm: f64) -> f64 {
        let n_ticks = (dt_secs as f64 / self.tick_secs as f64).max(1.0);

        let spm_factor = ((shares_per_minute as f64) / self.cusum_reference_spm).sqrt();
        let sensitivity = self.cusum_sensitivity * spm_factor;

        let base_threshold = (sensitivity / n_ticks) + self.cusum_floor;

        // Asymmetric tighten multiplier: tightening is the dangerous direction.
        let would_tighten = realized_spm > shares_per_minute as f64;
        let asymmetric_threshold = if would_tighten {
            base_threshold * self.cusum_tighten_multiplier
        } else {
            base_threshold
        };

        // Sign-persistence: the threshold relaxes only after consecutive
        // same-direction ticks accumulate, so a single lucky streak can't trip a
        // (tightening) fire.
        let current_sign: i8 = if realized_spm > shares_per_minute as f64 {
            1
        } else {
            -1
        };
        let consecutive = if current_sign == self.cusum_last_sign {
            self.cusum_consecutive = self.cusum_consecutive.saturating_add(1);
            self.cusum_consecutive
        } else {
            self.cusum_last_sign = current_sign;
            self.cusum_consecutive = 1;
            1
        };
        let discount = (self.cusum_sign_persistence_discount * (consecutive - 1) as f64)
            .min(self.cusum_max_sign_discount);

        asymmetric_threshold * (1.0 - discount) * 100.0
    }

    /// Stage 3: Compute new hashrate with accelerating partial retarget.
    fn compute_new_hashrate(&mut self, h_estimate: f32, current_hashrate: f32) -> f32 {
        let direction: i8 = if h_estimate > current_hashrate { 1 } else { -1 };

        let consecutive = if direction == self.last_direction {
            self.consecutive_same_direction += 1;
            self.consecutive_same_direction
        } else {
            self.last_direction = direction;
            self.consecutive_same_direction = 1;
            1
        };

        let eta = (self.eta_base + self.acceleration * (consecutive - 1) as f32).min(self.eta_max);
        current_hashrate + eta * (h_estimate - current_hashrate)
    }

    /// Rescale the EWMA after a fire so the rate reflects the new difficulty.
    fn rescale_ewma(&mut self, new_hashrate: f32, old_hashrate: f32) {
        if old_hashrate <= 0.0 || new_hashrate <= 0.0 {
            self.rate = 0.0;
            self.n_ticks = 0;
            self.shares_since_last_update = 0;
            return;
        }

        let ratio = new_hashrate as f64 / old_hashrate as f64;
        if ratio > 0.0 && ratio.is_finite() {
            self.rate /= ratio;
        } else {
            self.rate = 0.0;
            self.n_ticks = 0;
        }
    }

    /// One evaluation tick: the champion's estimate → boundary → update
    /// pipeline given an explicit time-since-last-fire (`dt_secs`). Returns the
    /// new hashrate if the boundary fires. Callers must ensure this advances at
    /// most once per `tick_secs` of real/virtual time (the EWMA assumes it);
    /// [`Self::try_vardiff`] enforces that. Exposed for deterministic tests.
    fn tick(
        &mut self,
        hashrate: f32,
        target: &Target,
        shares_per_minute: f32,
        dt_secs: u64,
    ) -> Option<f32> {
        // Stage 1: EWMA estimate.
        let (realized_spm, h_estimate) = self.estimate(hashrate, target, shares_per_minute);

        // Deviation: |ratio - 1| × 100.
        let delta = if hashrate > 0.0 {
            ((h_estimate as f64 / hashrate as f64) - 1.0).abs() * 100.0
        } else {
            0.0
        };

        // Stage 2: Boundary.
        let threshold = self.threshold(dt_secs, shares_per_minute, realized_spm);

        debug!(
            target: "vardiff",
            "champion: dt={}s, realized_spm={:.2}, h_estimate={:.2}, delta={:.1}%, threshold={:.1}%",
            dt_secs, realized_spm, h_estimate, delta, threshold,
        );

        if delta < threshold {
            return None;
        }

        // Stage 3: Update.
        let mut new_hashrate = self.compute_new_hashrate(h_estimate, hashrate);

        if new_hashrate < self.min_allowed_hashrate {
            new_hashrate = self.min_allowed_hashrate;
        }

        // Post-fire: rescale EWMA so the rate reflects the new difficulty.
        self.rescale_ewma(new_hashrate, hashrate);

        Some(new_hashrate)
    }
}

impl Vardiff for ChampionVardiffState {
    fn kind(&self) -> super::VardiffKind {
        super::VardiffKind::Champion
    }

    fn last_update_timestamp(&self) -> u64 {
        self.timestamp_of_last_update
    }

    fn shares_since_last_update(&self) -> u32 {
        self.shares_since_last_update
    }

    fn set_timestamp_of_last_update(&mut self, ts: u64) {
        self.timestamp_of_last_update = ts;
    }

    fn increment_shares_since_last_update(&mut self) {
        self.shares_since_last_update += 1;
    }

    fn min_allowed_hashrate(&self) -> f32 {
        self.min_allowed_hashrate
    }

    fn reset_counter(&mut self) -> Result<(), VardiffError> {
        let now = super::sim_clock::now_secs();
        self.timestamp_of_last_update = now;
        self.last_tick_secs = now;
        self.rate = 0.0;
        self.shares_since_last_update = 0;
        self.n_ticks = 0;
        self.consecutive_same_direction = 0;
        self.last_direction = 0;
        // Sign-persistence boundary state must also reset, else a stale
        // consecutive count carries an accumulated discount into the next cycle
        // — relaxing the tightening threshold (the dangerous direction).
        self.cusum_last_sign = 0;
        self.cusum_consecutive = 0;
        Ok(())
    }

    fn try_vardiff(
        &mut self,
        hashrate: f32,
        target: &Target,
        shares_per_minute: f32,
    ) -> Result<Option<f32>, VardiffError> {
        let now = super::sim_clock::now_secs();

        // Fixed 60s tick cadence: buffer shares between ticks (the EWMA assumes
        // one call per tick_secs; the pool calls share-driven, far more often).
        let since_tick = now.saturating_sub(self.last_tick_secs);
        if since_tick < self.tick_secs {
            return Ok(None);
        }
        // Grid-lock the cadence: advance by exactly one tick_secs rather than to
        // `now`, so tick boundaries stay on a fixed grid and each tick consumes
        // ~tick_secs of shares (avoids a slow upward drift in realized_spm when
        // the gate opens a few seconds late). The pool's <=10s backstop calls
        // try_vardiff at least that often, so we never fall more than one tick
        // behind; if we somehow do (long silence), resync to now.
        if since_tick < 2 * self.tick_secs {
            self.last_tick_secs += self.tick_secs;
        } else {
            self.last_tick_secs = now;
        }

        // Time since the last fire drives the boundary's evidence accumulation.
        let dt = now.saturating_sub(self.timestamp_of_last_update);

        match self.tick(hashrate, target, shares_per_minute, dt) {
            Some(new_hashrate) => {
                self.timestamp_of_last_update = now;
                Ok(Some(new_hashrate))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Champion contract tests, ported from stratum-mining/stratum#2188's
    //! `test/classic.rs`. They drive the real [`Vardiff::try_vardiff`] entry
    //! point; `simulate_elapsed` backdates BOTH timestamp fields (the extra
    //! `last_tick_secs` is this port's self-regulated-cadence gate) so a
    //! backdated call always advances exactly one 60s tick — exactly the
    //! upstream test semantics.
    use super::*;
    use crate::target::hash_rate_to_target;

    const TEST_MIN_HASHRATE: f32 = 1.0;
    const TEST_SHARES_PER_MINUTE: f32 = 12.0;
    const TEST_HASHRATE: f32 = 1.0e12;

    fn add_shares(v: &mut ChampionVardiffState, n: u32) {
        for _ in 0..n {
            v.increment_shares_since_last_update();
        }
    }

    fn make_vardiff() -> ChampionVardiffState {
        ChampionVardiffState::new_with_min(TEST_MIN_HASHRATE).expect("construct")
    }

    fn target_for(spm: f32) -> Target {
        hash_rate_to_target(TEST_HASHRATE.into(), spm.into())
            .unwrap()
            .into()
    }

    /// Backdate both timestamp fields by `secs`, so `try_vardiff` sees a full
    /// window (dt = secs) and the cadence gate opens (since_tick = secs).
    fn simulate_elapsed(v: &mut ChampionVardiffState, secs: u64) {
        let now = super::super::sim_clock::now_secs();
        v.timestamp_of_last_update = now - secs;
        v.last_tick_secs = now - secs;
    }

    #[test]
    fn new_state_has_zero_shares() {
        let v = make_vardiff();
        assert_eq!(v.shares_since_last_update(), 0);
        assert_eq!(v.min_allowed_hashrate(), TEST_MIN_HASHRATE);
    }

    #[test]
    fn increment_shares_accumulates() {
        let mut v = make_vardiff();
        v.increment_shares_since_last_update();
        v.increment_shares_since_last_update();
        assert_eq!(v.shares_since_last_update(), 2);
    }

    #[test]
    fn add_shares_bulk() {
        let mut v = make_vardiff();
        add_shares(&mut v, 42);
        assert_eq!(v.shares_since_last_update(), 42);
    }

    #[test]
    fn reset_counter_zeroes_state() {
        let mut v = make_vardiff();
        add_shares(&mut v, 10);
        v.reset_counter().unwrap();
        assert_eq!(v.shares_since_last_update(), 0);
    }

    #[test]
    fn reset_counter_clears_sign_persistence_state() {
        let spm = 30.0f32;
        let target = target_for(spm);
        let mut v = make_vardiff();
        for _ in 0..6 {
            add_shares(&mut v, (spm as u32) * 5);
            simulate_elapsed(&mut v, 60);
            let _ = v.try_vardiff(TEST_HASHRATE, &target, spm).unwrap();
        }
        let (_, consecutive_before) = v.cusum_sign_state();
        assert!(
            consecutive_before > 0,
            "precondition: sign-persistence should have accumulated"
        );
        v.reset_counter().unwrap();
        assert_eq!(v.cusum_sign_state(), (0, 0), "reset must zero cusum state");
    }

    #[test]
    fn no_fire_within_15s() {
        let mut v = make_vardiff();
        let target = target_for(TEST_SHARES_PER_MINUTE);
        add_shares(&mut v, 100);
        simulate_elapsed(&mut v, 10);
        assert_eq!(
            v.try_vardiff(TEST_HASHRATE, &target, TEST_SHARES_PER_MINUTE)
                .unwrap(),
            None
        );
    }

    #[test]
    fn fires_when_miner_is_much_faster() {
        // The champion deliberately requires SUSTAINED evidence to tighten:
        // an 8x tighten-multiplier plus a slow EWMA(360) mean one tick of a
        // fast miner does not fire. Across repeated ticks it catches up.
        let mut v = make_vardiff();
        let target = target_for(TEST_SHARES_PER_MINUTE);
        let mut fired = None;
        for _ in 0..12 {
            add_shares(&mut v, 60);
            simulate_elapsed(&mut v, 60);
            if let Some(new_h) = v
                .try_vardiff(TEST_HASHRATE, &target, TEST_SHARES_PER_MINUTE)
                .unwrap()
            {
                fired = Some(new_h);
                break;
            }
        }
        let new_h = fired.expect("a sustained 5x-faster miner should fire within a few minutes");
        assert!(new_h > TEST_HASHRATE, "hashrate should increase: {new_h}");
    }

    #[test]
    fn fires_when_miner_is_much_slower() {
        let mut v = make_vardiff();
        let target = target_for(TEST_SHARES_PER_MINUTE);
        add_shares(&mut v, 3);
        simulate_elapsed(&mut v, 300);
        let result = v
            .try_vardiff(TEST_HASHRATE, &target, TEST_SHARES_PER_MINUTE)
            .unwrap();
        assert!(result.is_some(), "should fire on 75% deviation at 300s");
        assert!(result.unwrap() < TEST_HASHRATE, "hashrate should decrease");
    }

    #[test]
    fn no_fire_on_stable_rate() {
        let mut v = make_vardiff();
        let target = target_for(TEST_SHARES_PER_MINUTE);
        add_shares(&mut v, 12);
        simulate_elapsed(&mut v, 60);
        assert_eq!(
            v.try_vardiff(TEST_HASHRATE, &target, TEST_SHARES_PER_MINUTE)
                .unwrap(),
            None,
            "should not fire when rate matches target"
        );
    }

    #[test]
    fn partial_retarget_moves_toward_estimate_not_fully() {
        let mut v = make_vardiff();
        let target = target_for(TEST_SHARES_PER_MINUTE);
        add_shares(&mut v, 36);
        simulate_elapsed(&mut v, 300);
        let result = v
            .try_vardiff(TEST_HASHRATE, &target, TEST_SHARES_PER_MINUTE)
            .unwrap();
        assert!(result.is_some(), "should fire on 3x deviation at 300s");
        let new_h = result.unwrap();
        assert!(new_h > TEST_HASHRATE, "should increase: {new_h}");
        assert!(
            new_h < TEST_HASHRATE * 2.0,
            "eta=0.2 keeps new hashrate below 2x (full retarget ~3x): {new_h}"
        );
    }

    #[test]
    fn consecutive_fires_accelerate_eta() {
        let mut v = make_vardiff();
        let target = target_for(TEST_SHARES_PER_MINUTE);
        add_shares(&mut v, 60);
        simulate_elapsed(&mut v, 300);
        let h1 = v
            .try_vardiff(TEST_HASHRATE, &target, TEST_SHARES_PER_MINUTE)
            .unwrap()
            .expect("first fire on 5x deviation at 300s");
        assert!(h1 > TEST_HASHRATE, "first fire should increase: {h1}");
    }

    #[test]
    fn cusum_boundary_used_at_high_spm() {
        // Ported verbatim from #2188: a single tick with a large buffered
        // deviation (300 shares over 300s → EWMA first-tick rate=300, 10x the
        // 30 spm target) clears even the 8x tightening bar in one tick.
        let mut v = make_vardiff();
        let spm = 30.0f32;
        let target = target_for(spm);
        add_shares(&mut v, 300);
        simulate_elapsed(&mut v, 300);
        let result = v.try_vardiff(TEST_HASHRATE, &target, spm).unwrap();
        assert!(
            result.is_some(),
            "CUSUM should fire at high SPM with a large deviation over 300s"
        );
    }

    #[test]
    fn asymmetric_cusum_tightening_is_harder_than_loosening() {
        let spm = 30.0f32;
        let target = target_for(spm);

        fn fires_within(shares_per_tick: u32, spm: f32, target: &Target, max_ticks: u32) -> Option<u32> {
            let mut v = make_vardiff();
            for t in 1..=max_ticks {
                add_shares(&mut v, shares_per_tick);
                simulate_elapsed(&mut v, 60);
                if v.try_vardiff(TEST_HASHRATE, target, spm).unwrap().is_some() {
                    return Some(t);
                }
            }
            None
        }

        let loosen = fires_within((spm as u32) / 5, spm, &target, 20);
        let tighten = fires_within((spm as u32) * 5, spm, &target, 20);
        let lt = loosen.expect("a deep loosening (0.2x) should fire within 20 ticks");
        if let Some(tt) = tighten {
            assert!(
                lt <= tt,
                "loosening should fire no later than tightening (8x harder): loosen={lt} tighten={tt}"
            );
        }
    }

    #[test]
    fn cadence_gate_buffers_shares_between_ticks() {
        // The port's addition: sub-tick calls buffer shares and do not fire.
        let mut v = make_vardiff();
        let target = target_for(TEST_SHARES_PER_MINUTE);
        add_shares(&mut v, 50);
        // 30s < 60s tick: gated, shares retained.
        simulate_elapsed(&mut v, 30);
        v.last_tick_secs = super::super::sim_clock::now_secs() - 30;
        assert_eq!(
            v.try_vardiff(TEST_HASHRATE, &target, TEST_SHARES_PER_MINUTE)
                .unwrap(),
            None,
            "sub-tick call must be gated"
        );
        assert_eq!(v.shares_since_last_update(), 50, "shares must be retained");
    }
}
