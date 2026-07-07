//! PID-based variable difficulty controller.
//!
//! Controls the channel's difficulty so the realized share rate tracks the
//! configured `shares_per_minute` setpoint, using a
//! proportional-integral-derivative loop in **log space**.
//!
//! # Why log space
//!
//! The share rate of a miner with hashrate `H` against difficulty `D` is
//! `λ = H / (D * 2^32)`, so `ln λ` responds to `ln D` linearly with slope -1:
//! in log space the plant is a unit-gain linear system, independent of the
//! miner's absolute size. One set of gains therefore works identically for a
//! 10 GH/s device and a 500 TH/s farm, and a multiplicative difficulty
//! adjustment `D *= exp(u)` is just an additive control move.
//!
//! # Differences from [`super::classic::VardiffState`]
//!
//! - The measurement window resets on **every** evaluation, not only when an
//!   update fires. Controller memory lives in the integral term, so reaction
//!   time is bounded by the loop period instead of growing with the time
//!   since the last adjustment.
//! - There is no deviation deadband ladder; any persistent error is corrected
//!   at a rate set by the gains. A small output deadband merely suppresses
//!   sub-percent `SetTarget` churn.
//!
//! # Loop shape
//!
//! ```text
//! e  = ln(realized_spm / setpoint_spm)        // positive => diff too low
//! w  = N_eff / (N_eff + K)                     // confidence in the window
//! e *= w                                       // shrink noisy measurements
//! d  = (m - m_prev) / dt, EWMA-filtered        // derivative on measurement
//! u* = KP*e + KI*I + KD*d                      // unsaturated output
//! u  = clamp(u*, ±ln(max_step))
//! I += e*dt + (u - u*)/KI * dt/T_t             // back-calculation anti-windup
//! new_hashrate = hashrate * exp(u)
//! ```
//!
//! The `(u - u*)` tracking term bleeds the integral by the saturation excess
//! whenever the output clamp engages, so the I-term follows the control move
//! actually emitted instead of winding up behind the clamp; `T_t`
//! ([`PidParams::tracking_secs`]) sets how fast it unwinds.
//!
//! The integral is **consumed (reset) whenever an update is emitted**: the
//! difficulty plant is itself an integrator — each emitted move is absorbed
//! multiplicatively into the channel target — so the integral's role here is
//! to accumulate sub-significance evidence *between* emissions, not to hold a
//! standing offset (which would double-count and ring).
//!
//! # Confidence weighting
//!
//! Share arrivals are Poisson, so a window containing `N` shares estimates
//! the log-rate with variance ≈ `1/N`: a window with 4 shares is *loud but
//! unreliable*, one with 100 shares is trustworthy. The error is therefore
//! shrunk by `w = N_eff / (N_eff + K)` before entering the P and I terms,
//! which scales the effective gain with measurement quality — one set of
//! gains then behaves consistently across share rates and loop periods.
//! `N_eff = max(observed, expected_at_setpoint)` so that *zero* shares when
//! many were expected still counts as high-confidence evidence (observing 0
//! against an expectation of E has probability e^-E), rather than being
//! discounted as a small sample.

use crate::target::hash_rate_from_target;
use bitcoin::Target;
use tracing::debug;

use super::{error::VardiffError, Vardiff};

/// Default proportional gain: correct 60% of the observed log-error per step.
pub const DEFAULT_KP: f64 = 0.6;
/// Default integral gain per second (≈0.3 per 60 s window).
pub const DEFAULT_KI: f64 = 0.005;
/// Default derivative gain. Off by default: share arrivals are Poisson and
/// differentiating that noise hurts more than it helps at typical rates.
pub const DEFAULT_KD: f64 = 0.0;
/// Largest multiplicative difficulty change permitted in one update.
pub const DEFAULT_MAX_STEP: f64 = 8.0;
/// Integral-pressure deadband: when a window is not individually
/// significant, an update is only emitted once the accumulated I-term
/// exceeds this fraction.
pub const DEFAULT_DEADBAND: f64 = 0.1;
/// Significance threshold in standard deviations: a single window only
/// triggers an update when its raw log-error exceeds `Z / sqrt(N_eff)`, the
/// approximate sigma of a Poisson window's log-rate estimate.
pub const DEFAULT_SIGNIFICANCE_Z: f64 = 2.0;
/// Back-calculation tracking time constant (seconds): how fast the integral
/// unwinds toward the saturated output when the `max_step` clamp engages.
pub const DEFAULT_TRACKING_SECS: f64 = 60.0;
/// Default minimum hashrate (H/s), matching the classic implementation.
const DEFAULT_MIN_HASHRATE: f32 = 1.0;
/// Degenerate-window floor. Unlike the classic implementation there is no
/// 15-second minimum: the significance gate and confidence weight already
/// hold windows that carry no statistical evidence, and short windows with a
/// genuine burst of shares are real evidence worth acting on. This floor
/// only guards the rate division against near-zero windows.
const MIN_WINDOW_SECS: f64 = 0.1;
/// Log-error clamp; bounds the reaction to pathological windows (e.g. zero
/// shares after a huge hashrate drop).
const MAX_ABS_ERROR: f64 = 3.0;
/// Shrinkage constant for the confidence weight `N_eff / (N_eff + K)`.
/// Larger values discount small windows more aggressively.
const CONFIDENCE_K: f64 = 2.0;
/// Absolute floor below which an update is never worth a SetTarget.
const MIN_EMIT_OUTPUT: f64 = 0.02;
/// EWMA factor for the derivative filter (higher = smoother, slower).
const DERIVATIVE_FILTER: f64 = 0.7;

/// Tuning parameters for [`PidVardiffState`].
#[derive(Debug, Clone, Copy)]
pub struct PidParams {
    /// Proportional gain on the log-space error.
    pub kp: f64,
    /// Integral gain per second of accumulated log-space error.
    pub ki: f64,
    /// Derivative gain on the (filtered) measurement slope.
    pub kd: f64,
    /// Largest multiplicative difficulty change per update (> 1).
    pub max_step: f64,
    /// I-term pressure required to emit when no single window is
    /// individually significant (corrects persistent small errors).
    pub deadband: f64,
    /// Single-window significance threshold in sigmas (the statistical
    /// deadband): a window emits on its own only when
    /// `|error| >= significance_z / sqrt(N_eff)`.
    pub significance_z: f64,
    /// Back-calculation anti-windup tracking time constant in seconds.
    pub tracking_secs: f64,
    /// Lowest hashrate estimate the controller will output.
    pub min_allowed_hashrate: f32,
}

impl Default for PidParams {
    fn default() -> Self {
        Self {
            kp: DEFAULT_KP,
            ki: DEFAULT_KI,
            kd: DEFAULT_KD,
            max_step: DEFAULT_MAX_STEP,
            deadband: DEFAULT_DEADBAND,
            significance_z: DEFAULT_SIGNIFICANCE_Z,
            tracking_secs: DEFAULT_TRACKING_SECS,
            min_allowed_hashrate: DEFAULT_MIN_HASHRATE,
        }
    }
}

/// PID vardiff controller state for one channel.
#[derive(Debug)]
pub struct PidVardiffState {
    params: PidParams,
    /// Shares observed in the current evaluation window.
    shares_since_last_update: u32,
    /// Unix timestamp (fractional seconds) when the current window started.
    /// Fractional so short evaluation windows measure their true length
    /// instead of suffering whole-second truncation.
    timestamp_of_last_update: f64,
    /// Integral of the log-space error over time (seconds).
    integral: f64,
    /// Previous log-measurement, for the derivative term.
    prev_log_measurement: Option<f64>,
    /// EWMA-filtered derivative of the log-measurement.
    filtered_derivative: f64,
}

impl PidVardiffState {
    pub fn new() -> Result<Self, VardiffError> {
        Self::with_params(PidParams::default())
    }

    pub fn with_params(params: PidParams) -> Result<Self, VardiffError> {
        let timestamp_secs = super::sim_clock::now_secs_f64();
        Ok(Self {
            params,
            shares_since_last_update: 0,
            timestamp_of_last_update: timestamp_secs,
            integral: 0.0,
            prev_log_measurement: None,
            filtered_derivative: 0.0,
        })
    }

    pub fn params(&self) -> &PidParams {
        &self.params
    }

    /// Integral clamp so that the I-term alone can never exceed the per-step
    /// output limit; combined with conditional integration this provides
    /// anti-windup.
    fn integral_limit(&self) -> f64 {
        if self.params.ki > 0.0 {
            self.params.max_step.ln() / self.params.ki
        } else {
            0.0
        }
    }
}

impl Vardiff for PidVardiffState {
    fn last_update_timestamp(&self) -> u64 {
        self.timestamp_of_last_update as u64
    }

    fn shares_since_last_update(&self) -> u32 {
        self.shares_since_last_update
    }

    fn min_allowed_hashrate(&self) -> f32 {
        self.params.min_allowed_hashrate
    }

    fn set_timestamp_of_last_update(&mut self, timestamp: u64) {
        self.timestamp_of_last_update = timestamp as f64;
    }

    fn increment_shares_since_last_update(&mut self) {
        self.shares_since_last_update += 1;
    }

    fn reset_counter(&mut self) -> Result<(), VardiffError> {
        self.timestamp_of_last_update = super::sim_clock::now_secs_f64();
        self.shares_since_last_update = 0;
        Ok(())
    }

    fn try_vardiff(
        &mut self,
        hashrate: f32,
        target: &Target,
        shares_per_minute: f32,
    ) -> Result<Option<f32>, VardiffError> {
        let now = super::sim_clock::now_secs_f64();
        let dt = now - self.timestamp_of_last_update;
        if dt < MIN_WINDOW_SECS {
            return Ok(None);
        }
        let shares = self.shares_since_last_update;

        // Fresh window every evaluation: controller memory lives in the
        // integral term, not in an ever-growing measurement window.
        self.reset_counter()?;

        // With zero shares "0.5 shares" bounds the log-error instead of
        // sending it to -infinity; the clamp below caps pathological windows.
        let realized_spm = (shares as f64).max(0.5) * 60.0 / dt;
        let setpoint = shares_per_minute as f64;
        if setpoint <= 0.0 {
            return Ok(None);
        }
        // Positive error => shares arriving too fast => difficulty too low.
        // Shrunk by the window's statistical confidence: the log-rate of a
        // Poisson window with N shares has variance ~1/N, so few-share
        // windows get proportionally less gain instead of being chased as if
        // they were exact. Zero-share windows are judged against how many
        // shares the setpoint *expected*, which is real evidence, not noise.
        let expected_at_setpoint = setpoint * dt / 60.0;
        let n_eff = (shares as f64).max(expected_at_setpoint);
        let confidence = n_eff / (n_eff + CONFIDENCE_K);
        let raw_error = (realized_spm / setpoint)
            .ln()
            .clamp(-MAX_ABS_ERROR, MAX_ABS_ERROR);
        let error = raw_error * confidence;

        // Derivative on the measurement (not the error) so setpoint changes
        // don't kick, EWMA-filtered against Poisson noise.
        let log_measurement = realized_spm.ln();
        let raw_derivative = match self.prev_log_measurement {
            Some(prev) => (log_measurement - prev) / dt,
            None => 0.0,
        };
        self.prev_log_measurement = Some(log_measurement);
        self.filtered_derivative = DERIVATIVE_FILTER * self.filtered_derivative
            + (1.0 - DERIVATIVE_FILTER) * raw_derivative;

        let max_step_ln = self.params.max_step.ln();
        let unsaturated = self.params.kp * error
            + self.params.ki * self.integral
            + self.params.kd * self.filtered_derivative;
        let output = unsaturated.clamp(-max_step_ln, max_step_ln);

        // Back-calculation anti-windup: integrate the error plus a tracking
        // term proportional to the saturation excess `(output - unsaturated)`.
        // While the clamp is engaged this bleeds the integral toward what was
        // actually emitted (time constant `tracking_secs`); when the output
        // is unsaturated the term is zero and integration is plain. The
        // clamp stays as a hard backstop.
        if self.params.ki > 0.0 {
            let tracking = (output - unsaturated) / self.params.ki;
            self.integral +=
                error * dt + tracking * dt / self.params.tracking_secs.max(f64::EPSILON);
            let limit = self.integral_limit();
            self.integral = self.integral.clamp(-limit, limit);
        }

        debug!(
            target: "vardiff",
            "PID update: window={dt:.2}s shares={shares} realized={realized_spm:.3}/min \
             setpoint={setpoint:.3}/min e={error:.4} I={:.4} d={:.5} u={output:.4}",
            self.integral, self.filtered_derivative,
        );

        // Statistical deadband: a single window earns an update only when its
        // raw error clears the ~Z-sigma noise floor of a Poisson window with
        // N_eff shares. Windows that fail the test still feed the integral,
        // so a persistent sub-sigma bias accumulates and emits through the
        // integral-pressure path instead of being lost.
        let significant_window = raw_error.abs() >= self.params.significance_z / n_eff.sqrt();
        let integral_pressure = (self.params.ki * self.integral).abs() >= self.params.deadband;
        if !significant_window && !integral_pressure {
            debug!(
                target: "vardiff",
                "PID hold: |e|={:.4} < {:.4} (Z/sqrt(N)) and |ki*I|={:.4} < {:.4}",
                raw_error.abs(),
                self.params.significance_z / n_eff.sqrt(),
                (self.params.ki * self.integral).abs(),
                self.params.deadband,
            );
            return Ok(None);
        }
        // Never worth a SetTarget regardless of significance.
        if output.abs() < MIN_EMIT_OUTPUT {
            return Ok(None);
        }

        // Estimate the miner's true hashrate from what the current target
        // realized, then scale by the controller output. The estimate (rather
        // than the nominal hashrate) anchors the P-step to the observed rate,
        // like the classic implementation.
        let estimated_hashrate =
            match hash_rate_from_target(target.to_le_bytes().into(), realized_spm) {
                Ok(h) => h as f64,
                Err(e) => {
                    debug!(
                        target: "vardiff",
                        "target->hashrate conversion failed ({e:?}); scaling nominal hashrate"
                    );
                    hashrate as f64 * realized_spm / setpoint
                }
            };
        // The controller output is the total move from the *current* operating
        // point; the estimate already embodies the proportional correction
        // (estimated/nominal = exp(error)), so apply the residual.
        let residual = output - self.params.kp * error;
        let mut new_hashrate = (estimated_hashrate.powf(self.params.kp)
            * (hashrate as f64).powf(1.0 - self.params.kp)
            * residual.exp()) as f32;

        // Bound the resulting per-step change like the raw output.
        let ratio = (new_hashrate / hashrate) as f64;
        if ratio > self.params.max_step {
            new_hashrate = (hashrate as f64 * self.params.max_step) as f32;
        } else if ratio < 1.0 / self.params.max_step {
            new_hashrate = (hashrate as f64 / self.params.max_step) as f32;
        }
        if new_hashrate < self.params.min_allowed_hashrate {
            new_hashrate = self.params.min_allowed_hashrate;
        }
        if !new_hashrate.is_finite() || new_hashrate <= 0.0 {
            return Ok(None);
        }

        // The difficulty plant is itself an integrator: the emitted move is
        // absorbed multiplicatively into the channel target, so the evidence
        // the integral accumulated is now applied. Consume it — a standing
        // I-term after emission double-counts and rings.
        self.integral = 0.0;

        Ok(Some(new_hashrate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::{hash_rate_from_target, hash_rate_to_target};

    fn target_for(hashrate: f32, spm: f32) -> Target {
        hash_rate_to_target(hashrate as f64, spm as f64)
            .expect("valid target")
            .into()
    }

    /// Puts the window start `secs` in the past so try_vardiff sees a full
    /// measurement window without sleeping.
    fn backdate(state: &mut PidVardiffState, secs: u64) {
        state.set_timestamp_of_last_update(state.last_update_timestamp() - secs);
    }

    #[test]
    fn no_update_within_min_window() {
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        assert!(state
            .try_vardiff(100.0e12, &target, 6.0)
            .unwrap()
            .is_none());
    }

    #[test]
    fn converged_miner_is_left_alone() {
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        // Exactly on setpoint: 6 shares in 60s.
        for _ in 0..6 {
            state.increment_shares_since_last_update();
        }
        backdate(&mut state, 60);
        let res = state.try_vardiff(100.0e12, &target, 6.0).unwrap();
        assert!(res.is_none(), "expected no update, got {res:?}");
    }

    #[test]
    fn raises_hashrate_when_shares_too_fast() {
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        // 4x the setpoint rate: 24 shares in 60s.
        for _ in 0..24 {
            state.increment_shares_since_last_update();
        }
        backdate(&mut state, 60);
        let new = state
            .try_vardiff(100.0e12, &target, 6.0)
            .unwrap()
            .expect("update expected");
        assert!(
            new > 100.0e12,
            "hashrate estimate should rise, got {new:.3e}"
        );
        assert!(
            new <= 100.0e12 * DEFAULT_MAX_STEP as f32,
            "per-step clamp violated: {new:.3e}"
        );
    }

    #[test]
    fn lowers_hashrate_on_zero_shares() {
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        backdate(&mut state, 60);
        let new = state
            .try_vardiff(100.0e12, &target, 6.0)
            .unwrap()
            .expect("update expected");
        assert!(
            new < 100.0e12,
            "hashrate estimate should drop, got {new:.3e}"
        );
        assert!(
            new >= 100.0e12 / DEFAULT_MAX_STEP as f32,
            "per-step clamp violated: {new:.3e}"
        );
    }

    #[test]
    fn noisy_converged_windows_are_held() {
        // 5 shares when 6 were expected is well inside one sigma for N=6;
        // a couple of such windows must not trigger an update.
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        for _ in 0..2 {
            for _ in 0..5 {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            let res = state.try_vardiff(100.0e12, &target, 6.0).unwrap();
            assert!(res.is_none(), "sub-sigma window should hold, got {res:?}");
        }
    }

    #[test]
    fn persistent_small_error_corrects_via_integral() {
        // 8 shares/min vs a 6/min setpoint is only ~1.2 sigma per window at
        // N=8, so single windows hold — but the bias is real and the
        // integral must eventually force a correction.
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        let mut corrected = None;
        for i in 0..20 {
            for _ in 0..8 {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if let Some(new) = state.try_vardiff(100.0e12, &target, 6.0).unwrap() {
                corrected = Some((i, new));
                break;
            }
        }
        let (_, new) = corrected.expect("integral pressure should force an update");
        assert!(new > 100.0e12, "correction should raise the estimate");
    }

    #[test]
    fn saturated_windows_do_not_wind_up_the_integral() {
        // Several windows of extreme error saturate the output; with
        // back-calculation the integral must not keep the controller pushing
        // long after the error is gone.
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        for _ in 0..3 {
            for _ in 0..600 {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            let _ = state.try_vardiff(100.0e12, &target, 6.0).unwrap();
        }
        // Error returns to zero (setpoint-rate windows). With the integral
        // consumed on every emission there is no residual pressure at all.
        let mut updates_after_recovery = 0;
        for _ in 0..4 {
            for _ in 0..6 {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if state.try_vardiff(100.0e12, &target, 6.0).unwrap().is_some() {
                updates_after_recovery += 1;
            }
        }
        assert_eq!(
            updates_after_recovery, 0,
            "integral wound up: {updates_after_recovery} updates after error vanished"
        );
    }

    #[test]
    fn window_resets_every_evaluation() {
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        for _ in 0..6 {
            state.increment_shares_since_last_update();
        }
        backdate(&mut state, 60);
        let _ = state.try_vardiff(100.0e12, &target, 6.0).unwrap();
        assert_eq!(state.shares_since_last_update(), 0);
    }

    #[test]
    fn converges_in_closed_loop() {
        // Simulate a miner whose true hashrate is 20x the pool's estimate and
        // check the loop converges within a handful of 60s cycles.
        let true_hashrate: f64 = 2000.0e12;
        let mut nominal: f32 = 100.0e12;
        let spm: f32 = 6.0;
        let mut state = PidVardiffState::new().unwrap();

        for _ in 0..12 {
            let target = target_for(nominal, spm);
            // Expected shares in 60s at the current target for the true rate;
            // deterministic (no Poisson noise) for test stability.
            let hashrate_per_spm = hash_rate_from_target(target.to_le_bytes().into(), 1.0)
                .expect("valid target");
            let shares = (true_hashrate / hashrate_per_spm).round() as u32;
            for _ in 0..shares {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if let Some(new) = state.try_vardiff(nominal, &target, spm).unwrap() {
                nominal = new;
            }
        }

        let ratio = nominal as f64 / true_hashrate;
        assert!(
            (0.7..1.4).contains(&ratio),
            "expected convergence to ~{true_hashrate:.3e}, got {nominal:.3e} (ratio {ratio:.3})"
        );
    }
}
