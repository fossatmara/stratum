use crate::target::hash_rate_from_target;
use bitcoin::Target;
use tracing::debug;

/// Default minimum hashrate (H/s) if not specified.
const DEFAULT_MIN_HASHRATE: f32 = 1.0;

/// Most the believed hashrate may fall below the last belief that had evidence behind it.
///
/// *Displacement* is that ratio, `d = H_true / H_believed`. A belief `d` times too low serves a
/// difficulty `d` times too easy, so a miner asked for `r*` shares a minute sends `r*·d` when it
/// returns. Bounding displacement bounds the flood.
///
/// A ratio rather than a count of eases, because a count only equals a ratio for one step size.
/// On this revision the two agree: a silent channel still takes the `÷3` arm, so two eases and a
/// ratio of `9` are the same bound. They part company once that arm goes and silence eases through
/// the estimator instead, where two eases is `1.14×`. Changing the form now, while both give `9×`,
/// makes the change checkable as a no-op — the existing bound test passes untouched.
/// `min_allowed_hashrate` is no substitute: from a 200 TH belief its `1.0` H/s default is some
/// thirty eases away.
///
/// Binds only while evidence is absent: [`VardiffState::evidenced_hashrate`] re-anchors whenever
/// shares arrive, so a genuinely declining miner, which still submits, is never held back by it.
const MAX_SILENT_DISPLACEMENT: f32 = 9.0;

/// Time constant of the EWMA estimator, in seconds.
///
/// Old evidence decays on the clock at `e^(−Δt/tau)`, which is what makes the window unable to
/// entrench: it forgets whether or not the controller ever decides to act.
const EWMA_TAU_SECS: u64 = 360;

use super::{
    clock::{Clock, SystemClock},
    error::VardiffError,
    Vardiff,
};
use std::sync::Arc;

/// Represents the dynamic state for a variable difficulty (Vardiff) connection.
///
/// Tracks performance and adjusts the mining target to achieve a desired share rate.
///
/// Construct with [`VardiffState::new`] or [`VardiffState::new_with_min`]. The struct is
/// `#[non_exhaustive]` so that controller state can be added without a breaking change:
/// every field of a constructible `pub` struct is part of its public API, whether the field
/// itself is `pub` or private, so without this attribute each added field forces a major
/// version. Nothing is constructing this by struct literal — the only literal is in
/// `new_with_min` below — so the attribute costs callers nothing.
#[derive(Debug)]
#[non_exhaustive]
pub struct VardiffState {
    /// Count of shares received since the last difficulty adjustment.
    pub shares_since_last_update: u32,
    /// Unix timestamp (seconds) of the last difficulty adjustment.
    pub timestamp_of_last_update: u64,
    /// The lowest hashrate (H/s) the system will allow; values below this are clamped.
    pub min_allowed_hashrate: f32,
    /// Source of "current time". Defaults to [`SystemClock`], so production behaviour is
    /// unchanged; tests substitute a [`MockClock`] to drive elapsed time explicitly.
    ///
    /// [`MockClock`]: super::clock::MockClock
    clock: Arc<dyn Clock>,
    /// The belief as of the last evaluation that saw shares, i.e. the last belief backed by
    /// evidence. The ease is not allowed below `evidenced_hashrate / MAX_SILENT_DISPLACEMENT`.
    /// `0.0` means "not yet anchored"; the first evaluation anchors it to the opening belief.
    evidenced_hashrate: f32,
    /// Unix timestamp (seconds) of the last *evaluation*, as distinct from the last retarget.
    ///
    /// The estimator needs its own clock reference. `timestamp_of_last_update` advances only when
    /// the controller retargets, so it measures the window the decision boundary cares about. The
    /// EWMA consumes the pending share count on every evaluation, so its interval is the gap since
    /// the previous evaluation; dividing one by the other would pair a single evaluation's shares
    /// with the time since the last retarget and under-read the rate by exactly the factor by
    /// which the channel had gone unretargeted.
    timestamp_of_last_evaluation: u64,
    /// EWMA-smoothed share rate, in shares per minute. Difficulty-relative: it is rescaled on
    /// every retarget by [`VardiffState::rescale_ewma`].
    rate: f64,
    /// Whether `rate` holds an observation yet. An unseeded filter takes its first observation
    /// whole rather than blending it against a zero prior, which would start every channel with a
    /// fictitious idle period.
    rate_seeded: bool,
}

impl VardiffState {
    /// Creates a new `VardiffState` with the default minimum hashrate.
    ///
    /// # Arguments
    /// * `estimated_hashrate` - The initial hashrate estimate.
    pub fn new() -> Result<Self, VardiffError> {
        Self::new_with_min(DEFAULT_MIN_HASHRATE)
    }

    /// Creates a new `VardiffState` with a specific minimum hashrate.
    ///
    /// # Arguments
    /// * `min_allowed_hashrate` - The minimum hashrate to enforce. A non-positive or non-finite
    ///   value is meaningless as a floor (and would reintroduce the division-by-zero that
    ///   [`Vardiff::try_vardiff`] guards against), so it falls back to the default.
    pub fn new_with_min(min_allowed_hashrate: f32) -> Result<Self, VardiffError> {
        Self::new_with_clock(min_allowed_hashrate, Arc::new(SystemClock))
    }

    /// Creates a new `VardiffState` reading time from `clock`.
    ///
    /// The controller's behaviour is a function of elapsed time — which window has passed, how
    /// long a channel has been silent, how many evaluations have gone by without acting — so
    /// testing it means controlling the clock rather than waiting on one. [`VardiffState::new`]
    /// and [`VardiffState::new_with_min`] supply [`SystemClock`] and behave exactly as before.
    ///
    /// # Arguments
    /// * `min_allowed_hashrate` - As [`VardiffState::new_with_min`].
    /// * `clock` - Time source. Shared, because the algorithm reads it while a test driver
    ///   advances it.
    pub fn new_with_clock(
        min_allowed_hashrate: f32,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, VardiffError> {
        let timestamp_secs = clock.now_secs()?;

        let min_allowed_hashrate = if min_allowed_hashrate.is_finite() && min_allowed_hashrate > 0.0
        {
            min_allowed_hashrate
        } else {
            DEFAULT_MIN_HASHRATE
        };

        Ok(VardiffState {
            shares_since_last_update: 0,
            timestamp_of_last_update: timestamp_secs,
            min_allowed_hashrate,
            clock,
            evidenced_hashrate: 0.0,
            timestamp_of_last_evaluation: timestamp_secs,
            rate: 0.0,
            rate_seeded: false,
        })
    }

    /// Sets the count of shares since the last update.
    pub fn set_shares_since_last_update(&mut self, shares_since_last_update: u32) {
        self.shares_since_last_update = shares_since_last_update;
    }

    /// Test-only: the EWMA's smoothed rate, in shares per minute.
    #[cfg(test)]
    pub(crate) fn ewma_rate(&self) -> f64 {
        self.rate
    }

    /// Decay factor for an interval of `dt_secs`: `e^(−Δt/tau)`.
    ///
    /// Derived from the *measured* interval rather than a nominal period, so a late, early or
    /// missed evaluation changes how much the filter forgets by the right amount instead of
    /// silently altering its memory. This is the same reason `tau` is parameterised in seconds
    /// rather than in evaluations.
    fn ewma_alpha(dt_secs: u64) -> f64 {
        (-(dt_secs as f64) / (EWMA_TAU_SECS as f64)).exp()
    }

    /// Folds the pending share count into the EWMA and returns
    /// `(realized shares per minute, implied hashrate)`.
    ///
    /// Replaces the cumulative mean `shares / elapsed`, whose window only ever reset when the
    /// controller decided to retarget. That coupling is why the old estimator entrenched: a window
    /// that grows whenever the controller declines to act is least able to notice that it should.
    /// An EWMA holds its memory in the coefficient instead, so evidence ages on the clock and the
    /// estimator cannot be starved of forgetting by a run of no-fire decisions.
    ///
    /// Consumes the pending count, so callers that need it — the silence anchor, the log line —
    /// must read it first.
    fn estimate(
        &mut self,
        dt_secs: u64,
        hashrate: f32,
        target: &Target,
        shares_per_minute: f32,
    ) -> (f64, f32) {
        // Smooth a *rate*, not a raw count. `try_vardiff` is not called on a fixed cadence, so
        // folding the count directly would make the estimate scale with however long the caller
        // happened to wait — five shares over five minutes would read the same as five over one.
        let observed_spm = if dt_secs > 0 {
            self.shares_since_last_update as f64 / (dt_secs as f64 / 60.0)
        } else {
            0.0
        };

        // An unseeded EWMA takes the first observation whole; blending against a zero prior would
        // start every channel with a fictitious idle period.
        self.rate = if self.rate_seeded {
            let alpha = Self::ewma_alpha(dt_secs);
            alpha * self.rate + (1.0 - alpha) * observed_spm
        } else {
            observed_spm
        };
        self.rate_seeded = true;
        self.shares_since_last_update = 0;

        let realized_share_per_min = self.rate;

        let h_estimate = match hash_rate_from_target(
            target.to_le_bytes().into(),
            realized_share_per_min,
        ) {
            Ok(h) => h as f32,
            Err(e) => {
                debug!(
                    target: "vardiff",
                    "Target->Hashrate conversion failed: {:?}. Falling back using previous hashrate and realized_shares_per_minute", e
                );
                hashrate * realized_share_per_min as f32 / shares_per_minute
            }
        };

        (realized_share_per_min, h_estimate)
    }

    /// Rescales the EWMA after a retarget so its stored rate refers to the new difficulty.
    ///
    /// `rate` counts shares against whatever target was in force when they were found, so it is a
    /// difficulty-relative quantity. Retargeting changes that reference: after moving the belief by
    /// `ratio`, the same physical hashrate produces `rate / ratio` shares per period. Without this
    /// the next evaluation would compare pre-retarget evidence against a post-retarget target and
    /// read a deviation that is an artefact of the controller's own action.
    fn rescale_ewma(&mut self, new_hashrate: f32, old_hashrate: f32) {
        if old_hashrate <= 0.0 || new_hashrate <= 0.0 {
            self.rate = 0.0;
            self.rate_seeded = false;
            return;
        }
        let ratio = new_hashrate as f64 / old_hashrate as f64;
        if ratio > 0.0 && ratio.is_finite() {
            self.rate /= ratio;
        } else {
            self.rate = 0.0;
            self.rate_seeded = false;
        }
    }
}

impl Vardiff for VardiffState {
    fn last_update_timestamp(&self) -> u64 {
        self.timestamp_of_last_update
    }

    fn shares_since_last_update(&self) -> u32 {
        self.shares_since_last_update
    }

    fn min_allowed_hashrate(&self) -> f32 {
        self.min_allowed_hashrate
    }

    /// Sets the timestamp of the last update.
    fn set_timestamp_of_last_update(&mut self, timestamp_of_last_update: u64) {
        self.timestamp_of_last_update = timestamp_of_last_update;
        // Re-anchoring the decision window re-anchors the estimator's clock with it. Leaving the
        // two references to disagree would let the estimator measure its pending shares against a
        // different interval than the one the caller just declared, which reads as a rate error
        // proportional to the gap between them.
        self.timestamp_of_last_evaluation = timestamp_of_last_update;
    }

    /// Increments the share counter by one.
    fn increment_shares_since_last_update(&mut self) {
        self.shares_since_last_update += 1;
    }

    /// Resets the share counter and updates the timestamp to now.
    fn reset_counter(&mut self) -> Result<(), VardiffError> {
        let timestamp_secs = self.clock.now_secs()?;
        self.set_timestamp_of_last_update(timestamp_secs);
        self.set_shares_since_last_update(0);
        // The EWMA is discarded rather than rescaled here: `reset_counter` is the "start over"
        // path (a backwards clock step), where there is no difficulty ratio to rescale by.
        // Rescaling belongs to `rescale_ewma`, on the retarget path.
        self.rate = 0.0;
        self.rate_seeded = false;
        self.timestamp_of_last_evaluation = timestamp_secs;
        Ok(())
    }

    /// Checks channel performance and potentially updates the hashrate and target.
    ///
    /// It calculates the realized share rate since the last update. If the
    /// deviation from the target rate is significant enough (based on internal,
    /// time-sensitive thresholds), it estimates a new hashrate and applies it.
    ///
    /// It returns `Ok(Some(new_hashrate))` when an update occurs,
    /// `Ok(None)` when conditions don't warrant an update, and
    /// `Err` for actual processing errors.
    fn try_vardiff(
        &mut self,
        hashrate: f32,
        target: &Target,
        shares_per_minute: f32,
    ) -> Result<Option<f32>, VardiffError> {
        let now = self.clock.now_secs()?;

        let delta_time = match now.checked_sub(self.timestamp_of_last_update) {
            Some(delta_time) => delta_time,
            None => {
                // The clock stepped backwards (e.g. NTP correction), leaving the recorded
                // timestamp in the future, so elapsed time is unmeasurable. Re-anchor the
                // window to `now` and skip just this round: the `delta_time <= 15` guard
                // below returns before `reset_counter()` runs, so without re-anchoring here
                // every later round would measure against the same stale future timestamp
                // and vardiff would stall for the whole length of the backwards step.
                debug!(
                    target: "vardiff",
                    "Clock stepped backwards (recorded {}, now {}); re-anchoring vardiff window",
                    self.timestamp_of_last_update,
                    now
                );
                self.reset_counter()?;
                return Ok(None);
            }
        };

        if delta_time <= 15 {
            return Ok(None);
        }

        // `min_allowed_hashrate` is validated in `new_with_min`, but the field is `pub`,
        // so direct assignment can still plant a non-finite or non-positive floor;
        // sanitize it here as well before relying on it.
        let min_hashrate =
            if self.min_allowed_hashrate.is_finite() && self.min_allowed_hashrate > 0.0 {
                self.min_allowed_hashrate
            } else {
                DEFAULT_MIN_HASHRATE
            };

        // `hashrate` is the channel's nominal hashrate, which originates from the
        // miner-supplied `nominal_hash_rate` and is only checked for negativity upstream.
        // It is the divisor for the delta percentage below and the base every special-case
        // cap scales (`hashrate * 10.0`, `hashrate / 1.5`), so a value at or below the floor
        // just pins the result back to that floor instead of converging, and `0.0`/`NaN`
        // produce `inf`/`NaN` percentages that disable every `should_update` arm outright.
        // The output is already clamped to the floor below, so clamp the baseline to the
        // same floor. `is_finite` is still needed: `inf` survives a plain `max()` and makes
        // the percentage `inf / inf == NaN`.
        let hashrate = if hashrate.is_finite() {
            hashrate.max(min_hashrate)
        } else {
            debug!(
                target: "vardiff",
                "Prior hashrate {hashrate} unusable; using minimum {min_hashrate}",
            );
            min_hashrate
        };

        // Interval the estimator measures over: since the previous evaluation, not since the
        // previous retarget. Saturates rather than wrapping if the clock stepped backwards between
        // the two references; `delta_time` above already handles that case for the window.
        let eval_dt = now.saturating_sub(self.timestamp_of_last_evaluation).max(1);
        self.timestamp_of_last_evaluation = now;

        // Anchor the silence bound before the estimator consumes the pending count. Evidence
        // re-anchors it to the belief that evidence was measured against; an unanchored channel
        // anchors to its opening belief, so a channel that is silent from the start is bounded
        // relative to where it started rather than being allowed to walk to the floor.
        //
        // The anchor is the belief *entering* this evaluation, not the one leaving it, so after a
        // retarget the floor still refers to the previous belief and lags by one fire. Deliberate:
        // it keeps the anchor to a value some evidence was actually measured against, where the
        // post-retarget belief is a value the controller has only just proposed.
        let pending_shares = self.shares_since_last_update;
        if pending_shares > 0 || self.evidenced_hashrate <= 0.0 {
            self.evidenced_hashrate = hashrate;
        }

        let (realized_share_per_min, estimated_hashrate) =
            self.estimate(eval_dt, hashrate, target, shares_per_minute);

        debug!(
            target: "vardiff",
            "Hashrate update check triggered:
            - Elapsed time: {}s
            - Shares since last update: {}
            - Realized shares per minute: {:.4}
            - Current miner target: {:?}",
            delta_time,
            pending_shares,
            realized_share_per_min,
            target
        );

        let mut new_hashrate = estimated_hashrate;

        let hashrate_delta = new_hashrate - hashrate;
        let hashrate_delta_percentage = (hashrate_delta.abs() / hashrate) * 100.0;

        debug!(
            target: "vardiff",
            "Calculated new hashrate: {:.2} H/s (Δ {:.2}%, previous {:.2} H/s)",
            new_hashrate,
            hashrate_delta_percentage,
            hashrate,
        );

        let should_update = match hashrate_delta_percentage {
            pct if pct >= 100.0 => true,
            pct if pct >= 60.0 && delta_time >= 60 => true,
            pct if pct >= 50.0 && delta_time >= 120 => true,
            pct if pct >= 45.0 && delta_time >= 180 => true,
            pct if pct >= 30.0 && delta_time >= 240 => true,
            pct if pct >= 15.0 && delta_time >= 300 => true,
            _ => false,
        };

        if !should_update {
            return Ok(None);
        }

        // Reachable whenever the smoothed rate is exactly zero: blending zero into zero leaves
        // zero, so this covers the entire life of a channel that has never delivered a share, not
        // merely its first evaluation. Such a channel therefore still eases `÷3` per evaluation
        // exactly as before this patch, bounded by the displacement floor below.
        //
        // Retained deliberately: without it a zero estimate would drive the belief straight at
        // `min_allowed_hashrate` in a single step, because the `>= 100.0` rung fires on a zero
        // estimate. It becomes unreachable once the decision boundary stops firing at 100%, and
        // should be removed then rather than now.
        if realized_share_per_min == 0.0 {
            new_hashrate = match delta_time {
                dt if dt <= 30 => hashrate / 1.5,
                dt if dt < 60 => hashrate / 2.0,
                _ => hashrate / 3.0,
            };
        } else if hashrate_delta_percentage > 1000.0 {
            new_hashrate = match delta_time {
                dt if dt <= 30 => hashrate * 10.0,
                dt if dt < 60 => hashrate * 5.0,
                _ => hashrate * 3.0,
            };
        }

        // Bound the ease on absent evidence. Applied as a floor on the value rather than as a gate
        // on the decision, which is what makes the bound exact: a gate lets the step that crosses
        // it through, so the realized displacement depends on the step size, while a floor pins it
        // at `MAX_SILENT_DISPLACEMENT` for any estimator. Only downward movement is constrained,
        // and only while `evidenced_hashrate` is stale — any share re-anchors it above.
        if pending_shares == 0 && self.evidenced_hashrate > 0.0 {
            let silent_floor = self.evidenced_hashrate / MAX_SILENT_DISPLACEMENT;
            if new_hashrate < silent_floor {
                debug!(
                    target: "vardiff",
                    "Ease bounded at {:.0}x below the last evidenced belief: {:.2} -> {:.2} H/s",
                    MAX_SILENT_DISPLACEMENT,
                    new_hashrate,
                    silent_floor,
                );
                new_hashrate = silent_floor;
            }
        }

        if new_hashrate < min_hashrate {
            debug!(
                target: "vardiff",
                "New hashrate {:.2} H/s below minimum threshold {:.2} H/s — clamping",
                new_hashrate,
                min_hashrate
            );
            new_hashrate = min_hashrate;
        }

        // Re-anchor the evaluation window directly rather than through `reset_counter`, which also
        // discards the EWMA. The estimator's memory must survive a retarget — rescaled, not
        // dropped — or the controller would forget its evidence every time it acted, which is the
        // coupling this patch exists to remove. `estimate` has already consumed the share count.
        //
        // Nothing left to say when the bound absorbed the whole move: stay off the wire rather
        // than resending the difficulty the channel already has.
        if new_hashrate == hashrate {
            self.set_timestamp_of_last_update(now);
            return Ok(None);
        }

        self.rescale_ewma(new_hashrate, hashrate);
        self.set_timestamp_of_last_update(now);

        Ok(Some(new_hashrate))
    }
}
