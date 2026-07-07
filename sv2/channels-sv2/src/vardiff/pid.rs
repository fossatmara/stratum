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
/// approximate sigma of a Poisson window's log-rate estimate. 2 sigma
/// favors responsiveness: moderate hashrate changes clear the gate a window
/// or two sooner, at the cost of an occasional (~5% per judged window)
/// noise-triggered adjustment on converged miners. Raise toward 3 when
/// SetTarget churn matters more than reaction latency (live-tunable via
/// `super::tuning`).
pub const DEFAULT_SIGNIFICANCE_Z: f64 = 2.0;
/// Significance threshold for DOWNWARD difficulty corrections (miner slower
/// than setpoint). Asymmetric by design: overshooting downward yields more
/// shares — more information and fast self-correction — while staying too
/// high starves the pool of evidence. A lower bar on the down side buys
/// materially faster reaction to hashrate drops for a small rate of benign
/// downward nudges.
pub const DEFAULT_SIGNIFICANCE_Z_DOWN: f64 = 1.5;
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
/// Minimum evidence to judge a window: at least this many observed shares,
/// or a span in which the setpoint expected [`MIN_EVAL_EXPECTED`]. Share-
/// driven callers may evaluate on every share; single inter-arrival times
/// are exponentially distributed and far too noisy to judge alone (a lone
/// share looks "2-sigma fast" ~13% of the time by chance), so the window
/// keeps accumulating until it carries real evidence. Reaction latency then
/// scales with information rate: a big hashrate step supplies these shares
/// almost instantly, while a converged miner accumulates them quietly.
const MIN_EVAL_SHARES: f64 = 8.0;
/// Expected-count qualification for judging a share-bearing window that
/// lacks MIN_EVAL_SHARES observed shares. Zero-share windows instead derive
/// their qualification from the down-side significance bar (see
/// `silence_eval_expected`): silence of expected count E has probability
/// e^-E, so the sigma-equivalent evidence scales with E and a lower Z_down
/// honestly permits judging shorter silences.
const MIN_EVAL_EXPECTED: f64 = 6.0;

/// Expected-count needed before judging a pure-silence window, matched to
/// the down-side significance bar: approximates -ln(p) for the one-sided
/// normal tail at `z_down` sigmas (1.5 -> ~2.7, 2 -> ~4, 3 -> ~7.7).
fn silence_eval_expected(z_down: f64) -> f64 {
    z_down * z_down * 0.75 + 1.0
}
/// Log-error clamp; bounds the reaction to pathological windows (e.g. zero
/// shares after a huge hashrate drop).
const MAX_ABS_ERROR: f64 = 3.0;
// Confidence shrinkage constant: live-tunable, see `super::tuning`.
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
    /// Significance threshold for downward difficulty corrections; lower
    /// than `significance_z` by design (see [`DEFAULT_SIGNIFICANCE_Z_DOWN`]).
    /// Effectively capped at `significance_z`.
    pub significance_z_down: f64,
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
            significance_z_down: DEFAULT_SIGNIFICANCE_Z_DOWN,
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
    /// Accumulated variance of the integral under pure Poisson noise; the
    /// integral-pressure emission gate tests |I| against Z * sqrt(this), so
    /// noise random-walks don't masquerade as accumulated evidence.
    integral_noise_var: f64,
    /// Telemetry identity: when set, gain values are published to
    /// [`super::telemetry`] so visualizations can track them (instance id
    /// guards against a reconnect race clearing the successor's entry).
    telemetry: Option<(String, u64)>,
    /// Virtual timestamp of the most recent share (or channel creation).
    /// Zero-share windows are judged over the silence since this point, so
    /// sustained starvation is cumulative evidence and successive corrections
    /// deepen instead of repeating from scratch.
    last_share_time: f64,
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
            integral_noise_var: 0.0,
            telemetry: None,
            last_share_time: timestamp_secs,
            prev_log_measurement: None,
            filtered_derivative: 0.0,
        })
    }

    pub fn params(&self) -> &PidParams {
        &self.params
    }

    /// Replaces the P/I/D gains (used by adaptive controllers layered on
    /// top, e.g. Q-learning gain scheduling).
    pub fn set_gains(&mut self, kp: f64, ki: f64, kd: f64) {
        let changed =
            kp != self.params.kp || ki != self.params.ki || kd != self.params.kd;
        self.params.kp = kp;
        self.params.ki = ki;
        self.params.kd = kd;
        if changed {
            if let Some((key, id)) = &self.telemetry {
                super::telemetry::publish_gains(key, *id, kp, ki, kd);
            }
        }
    }

    /// Enables gain telemetry under the given identity (usually the
    /// channel's user identity) and publishes the current gains.
    pub fn set_telemetry_key(&mut self, key: String) {
        let id = super::telemetry::next_instance_id();
        super::telemetry::publish_gains(&key, id, self.params.kp, self.params.ki, self.params.kd);
        self.telemetry = Some((key, id));
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

impl Drop for PidVardiffState {
    fn drop(&mut self) {
        if let Some((key, id)) = &self.telemetry {
            super::telemetry::clear_gains(key, *id);
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
        self.last_share_time = super::sim_clock::now_secs_f64();
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
        self.try_vardiff_observed(hashrate, target, shares_per_minute)
            .map(|(update, _)| update)
    }
}

/// What one evaluated measurement window looked like; consumed by adaptive
/// gain schedulers.
#[derive(Debug, Clone, Copy)]
pub struct WindowObservation {
    /// Confidence-unweighted log-space error.
    pub raw_error: f64,
    /// Effective sample count backing the window.
    pub n_eff: f64,
    /// EWMA-filtered log-measurement slope.
    pub derivative: f64,
    /// Window length in (virtual) seconds.
    pub dt: f64,
    /// Whether this evaluation emitted a difficulty update.
    pub emitted: bool,
}

impl PidVardiffState {
    /// Same as [`Vardiff::try_vardiff`], but also returns the window
    /// observation when a window was actually measured (`None` while the
    /// window is still too short).
    pub fn try_vardiff_observed(
        &mut self,
        hashrate: f32,
        _target: &Target,
        shares_per_minute: f32,
    ) -> Result<(Option<f32>, Option<WindowObservation>), VardiffError> {
        let now = super::sim_clock::now_secs_f64();
        let dt = now - self.timestamp_of_last_update;
        if dt < MIN_WINDOW_SECS {
            return Ok((None, None));
        }
        let shares = self.shares_since_last_update;
        let setpoint = shares_per_minute as f64;
        if setpoint <= 0.0 {
            return Ok((None, None));
        }
        // Zero-share windows are measured over the FULL silence since the
        // last share: absence is cumulative evidence, and re-anchoring it to
        // each post-emission window would make every correction round repeat
        // the same timid first step (drops would take many minutes to track).
        let dt_eff = if shares == 0 {
            (now - self.last_share_time).max(dt)
        } else {
            dt
        };

        // Keep accumulating until the window carries enough evidence to be
        // judged; the counter is NOT reset on this path, so the evidence is
        // retained for the next call. Pure-silence windows qualify at the
        // (lower) evidence level matched to the down-side significance bar.
        let z_up = super::tuning::significance_z_override().unwrap_or(self.params.significance_z);
        let z_down = super::tuning::significance_z_down_override()
            .unwrap_or(self.params.significance_z_down)
            .min(z_up);
        let eval_expected = if shares == 0 {
            silence_eval_expected(z_down)
        } else {
            MIN_EVAL_EXPECTED
        };
        if (shares as f64) < MIN_EVAL_SHARES && setpoint * dt_eff / 60.0 < eval_expected {
            return Ok((None, None));
        }

        // Fresh window every evaluation: controller memory lives in the
        // integral term, not in an ever-growing measurement window.
        self.reset_counter()?;

        // With zero shares "0.5 shares" bounds the log-error instead of
        // sending it to -infinity; the clamp below caps pathological windows.
        let realized_spm = (shares as f64).max(0.5) * 60.0 / dt_eff;
        // Positive error => shares arriving too fast => difficulty too low.
        // Shrunk by the window's statistical confidence: the log-rate of a
        // Poisson window with N shares has variance ~1/N, so few-share
        // windows get proportionally less gain instead of being chased as if
        // they were exact. Zero-share windows are judged against how many
        // shares the setpoint *expected*, which is real evidence, not noise.
        let expected_at_setpoint = setpoint * dt_eff / 60.0;
        let n_eff = (shares as f64).max(expected_at_setpoint);
        let confidence = n_eff / (n_eff + super::tuning::confidence_k());
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

        let mut observation = WindowObservation {
            raw_error,
            n_eff,
            derivative: self.filtered_derivative,
            dt,
            emitted: false,
        };

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
            // One window of pure noise contributes conf*dt*sigma_e to the
            // integral, sigma_e ~ 1/sqrt(N); accumulate its variance so the
            // pressure gate can test significance of the sum.
            self.integral_noise_var += (confidence * dt / n_eff.sqrt()).powi(2);
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
        // Asymmetric significance: downward corrections (negative error —
        // the miner is slower than setpoint) use the lower bar.
        let z_for = |direction: f64| if direction < 0.0 { z_down } else { z_up };
        let significant_window = raw_error.abs() >= z_for(raw_error) / n_eff.sqrt();
        // Integral pressure must be *statistically significant* accumulated
        // evidence, not a noise random-walk: require |I| to clear Z sigmas
        // of its own accumulated noise in addition to the deadband floor.
        let integral_sigma = self.integral_noise_var.sqrt().max(f64::EPSILON);
        let integral_pressure = (self.params.ki * self.integral).abs() >= self.params.deadband
            && self.integral.abs() >= z_for(self.integral) * integral_sigma;
        if !significant_window && !integral_pressure {
            debug!(
                target: "vardiff",
                "PID hold: |e|={:.4} < {:.4} (Z/sqrt(N)) and |ki*I|={:.4} < {:.4}",
                raw_error.abs(),
                z_for(raw_error) / n_eff.sqrt(),
                (self.params.ki * self.integral).abs(),
                self.params.deadband,
            );
            return Ok((None, Some(observation)));
        }
        // Never worth a SetTarget regardless of significance.
        if output.abs() < MIN_EMIT_OUTPUT {
            return Ok((None, Some(observation)));
        }

        // Emit exactly the controller's clamped output as a multiplicative
        // move from the current operating point. The output already carries
        // the confidence weighting, integral evidence, and per-step clamp;
        // anchoring to a raw observed-rate hashrate estimate here (as the
        // classic algorithm does) would bypass the confidence weighting and
        // let one thin lucky window emit its full unweighted correction,
        // which oscillates in closed loop.
        let mut new_hashrate = (hashrate as f64 * output.exp()) as f32;
        if new_hashrate < self.params.min_allowed_hashrate {
            new_hashrate = self.params.min_allowed_hashrate;
        }
        if !new_hashrate.is_finite() || new_hashrate <= 0.0 {
            return Ok((None, Some(observation)));
        }

        // The difficulty plant is itself an integrator: the emitted move is
        // absorbed multiplicatively into the channel target, so the evidence
        // the integral accumulated is now applied. Consume it — a standing
        // I-term after emission double-counts and rings. Its noise budget
        // restarts with it.
        self.integral = 0.0;
        self.integral_noise_var = 0.0;

        observation.emitted = true;
        Ok((Some(new_hashrate), Some(observation)))
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
        // Long enough that the setpoint expected >= MIN_EVAL_SHARES shares.
        backdate(&mut state, 120);
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
        for _ in 0..8 {
            state.increment_shares_since_last_update();
        }
        backdate(&mut state, 80);
        let _ = state.try_vardiff(100.0e12, &target, 6.0).unwrap();
        assert_eq!(state.shares_since_last_update(), 0);
    }

    #[test]
    fn thin_windows_accumulate_instead_of_resetting() {
        // Below the evidence threshold the window must hold AND keep its
        // shares, so share-driven callers can evaluate on every share
        // without judging meaningless single-interval windows.
        let mut state = PidVardiffState::new().unwrap();
        let target = target_for(100.0e12, 6.0);
        for _ in 0..3 {
            state.increment_shares_since_last_update();
        }
        backdate(&mut state, 20);
        let res = state.try_vardiff(100.0e12, &target, 6.0).unwrap();
        assert!(res.is_none());
        assert_eq!(state.shares_since_last_update(), 3, "evidence must be retained");
    }

    /// Deterministic xorshift for reproducible noise in tests.
    struct TestRng(u64);
    impl TestRng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn uniform(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
        /// Poisson sample (Knuth), matching real share-arrival statistics.
        fn poisson(&mut self, lambda: f64) -> u32 {
            let l = (-lambda).exp();
            let mut k = 0u32;
            let mut p = 1.0;
            loop {
                p *= self.uniform();
                if p <= l {
                    return k;
                }
                k += 1;
            }
        }
    }

    /// Closed-loop helper: expected shares for `true_hashrate` in a 60s
    /// window at the current target.
    fn shares_at(true_hashrate: f64, nominal: f32, spm: f32) -> u32 {
        let target = target_for(nominal, spm);
        let per_spm = hash_rate_from_target(target.to_le_bytes().into(), 1.0)
            .expect("valid target");
        (true_hashrate / per_spm).round() as u32
    }

    #[test]
    fn noisy_on_target_miner_gets_few_updates() {
        // Closed loop: a steady miner whose shares arrive with true Poisson
        // noise must rarely be disturbed, and the difficulty must not walk
        // away from the correct operating point.
        let spm: f32 = 6.0;
        let true_hashrate: f64 = 100.0e12;
        let mut nominal: f32 = 100.0e12;
        // Pin both bars symmetric at 3: this test guards the confidence
        // weighting and integral noise gate, independent of the shipped
        // significance defaults (including the asymmetric down-bar).
        let mut state = PidVardiffState::with_params(PidParams {
            significance_z: 3.0,
            significance_z_down: 3.0,
            ..PidParams::default()
        })
        .unwrap();
        let mut rng = TestRng(0x5eed_cafe);
        let mut emissions = 0;
        for _ in 0..60 {
            let expected = shares_at(true_hashrate, nominal, spm) as f64;
            for _ in 0..rng.poisson(expected) {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if let Some(new) = state
                .try_vardiff(nominal, &target_for(nominal, spm), spm)
                .unwrap()
            {
                nominal = new;
                emissions += 1;
            }
        }
        // Window lengths carry sub-second wall-clock jitter (timestamps are
        // whole seconds, "now" is fractional), so borderline windows flip
        // between runs; the pathology this guards against was 18-22/60.
        assert!(
            emissions <= 9,
            "noisy but on-target miner disturbed {emissions}/60 windows"
        );
        let ratio = nominal as f64 / true_hashrate;
        assert!(
            (0.5..2.0).contains(&ratio),
            "difficulty walked away under noise: ratio {ratio:.3}"
        );
    }

    #[test]
    fn aggressive_ramp_up_then_down_is_tracked() {
        // True hashrate doubles every window to 16x, holds, then halves
        // back down; the controller must follow both directions and end
        // near the truth without winding up.
        let spm: f32 = 6.0;
        let mut nominal: f32 = 100.0e12;
        let mut true_hashrate: f64 = 100.0e12;
        let mut state = PidVardiffState::new().unwrap();
        let step = |state: &mut PidVardiffState, nominal: &mut f32, th: f64| {
            for _ in 0..shares_at(th, *nominal, spm) {
                state.increment_shares_since_last_update();
            }
            backdate(state, 60);
            if let Some(new) = state.try_vardiff(*nominal, &target_for(*nominal, spm), spm).unwrap() {
                *nominal = new;
            }
        };
        for _ in 0..4 {
            true_hashrate *= 2.0;
            step(&mut state, &mut nominal, true_hashrate);
        }
        for _ in 0..6 {
            step(&mut state, &mut nominal, true_hashrate);
        }
        let up_ratio = nominal as f64 / true_hashrate;
        assert!(
            (0.5..2.0).contains(&up_ratio),
            "ramp-up not tracked: nominal {nominal:.3e} vs true {true_hashrate:.3e}"
        );
        for _ in 0..4 {
            true_hashrate /= 2.0;
            step(&mut state, &mut nominal, true_hashrate);
        }
        for _ in 0..8 {
            step(&mut state, &mut nominal, true_hashrate);
        }
        let down_ratio = nominal as f64 / true_hashrate;
        assert!(
            (0.5..2.0).contains(&down_ratio),
            "ramp-down not tracked: nominal {nominal:.3e} vs true {true_hashrate:.3e}"
        );
    }

    #[test]
    fn downward_bar_is_lower_than_upward() {
        // A moderate slowdown clears the down-bar; the mirror-image window
        // with the same |error| against a symmetric Z (z_down == z_up) holds.
        // 7 shares in 120s at setpoint 6: e = ln(3.5/6) = -0.539,
        // n_eff = max(7, 12) = 12 -> down-bar 1.5/sqrt(12) = 0.43 (emit),
        // up-bar 2/sqrt(12) = 0.58 (hold).
        let spm: f32 = 6.0;
        let target = target_for(100.0e12, spm);

        let mut asym = PidVardiffState::new().unwrap();
        for _ in 0..7 {
            asym.increment_shares_since_last_update();
        }
        backdate(&mut asym, 120);
        assert!(
            asym.try_vardiff(100.0e12, &target, spm).unwrap().is_some(),
            "slowdown should clear the down-bar"
        );

        let mut sym = PidVardiffState::with_params(PidParams {
            significance_z_down: DEFAULT_SIGNIFICANCE_Z,
            ..PidParams::default()
        })
        .unwrap();
        for _ in 0..7 {
            sym.increment_shares_since_last_update();
        }
        backdate(&mut sym, 120);
        assert!(
            sym.try_vardiff(100.0e12, &target, spm).unwrap().is_none(),
            "same window must hold under a symmetric bar"
        );
    }

    #[test]
    fn large_drop_tracks_within_minutes() {
        // A converged miner loses 100x of its hashrate. Silence is
        // cumulative evidence: each starved window measures the full quiet
        // span, so corrections deepen exponentially instead of repeating the
        // same first step. Must be within 3x of truth after 6 windows (~6
        // minutes at 60s windows) — the old per-window anchoring took far
        // longer.
        let spm: f32 = 6.0;
        let mut true_hashrate: f64 = 100.0e12;
        let mut nominal: f32 = 100.0e12;
        let mut state = PidVardiffState::new().unwrap();
        // Converged phase.
        for _ in 0..3 {
            for _ in 0..shares_at(true_hashrate, nominal, spm) {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if let Some(new) = state
                .try_vardiff(nominal, &target_for(nominal, spm), spm)
                .unwrap()
            {
                nominal = new;
            }
        }
        // The drop.
        true_hashrate /= 100.0;
        for _ in 0..6 {
            let shares = shares_at(true_hashrate, nominal, spm);
            for _ in 0..shares {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if shares == 0 {
                // Tests simulate time by backdating; extend the silence
                // anchor the same way the wall clock would.
                state.last_share_time -= 60.0;
            }
            if let Some(new) = state
                .try_vardiff(nominal, &target_for(nominal, spm), spm)
                .unwrap()
            {
                nominal = new;
            }
        }
        let ratio = nominal as f64 / true_hashrate;
        assert!(
            ratio < 3.0,
            "drop not tracked: nominal {nominal:.3e} vs true {true_hashrate:.3e} (ratio {ratio:.1})"
        );
        assert!(
            ratio > 0.2,
            "overshot below truth: ratio {ratio:.3}"
        );
    }

    #[test]
    fn silence_then_resume_reconverges() {
        // Flaky miner: converged, goes silent (rig off / disconnect without
        // channel teardown), difficulty ratchets down via zero-share
        // windows, then the miner resumes at its true rate and the loop
        // must re-converge without oscillating from leftover state.
        let spm: f32 = 6.0;
        let true_hashrate: f64 = 100.0e12;
        let mut nominal: f32 = 100.0e12;
        let mut state = PidVardiffState::new().unwrap();
        // Converged phase.
        for _ in 0..3 {
            for _ in 0..shares_at(true_hashrate, nominal, spm) {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if let Some(new) = state
                .try_vardiff(nominal, &target_for(nominal, spm), spm)
                .unwrap()
            {
                nominal = new;
            }
        }
        // Silence: several long zero-share backstop windows.
        for _ in 0..5 {
            backdate(&mut state, 120);
            if let Some(new) = state
                .try_vardiff(nominal, &target_for(nominal, spm), spm)
                .unwrap()
            {
                nominal = new;
            }
        }
        assert!(
            (nominal as f64) < true_hashrate * 0.6,
            "silence should ratchet the estimate down, got {nominal:.3e}"
        );
        // Resume at the true rate.
        for _ in 0..10 {
            for _ in 0..shares_at(true_hashrate, nominal, spm) {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            if let Some(new) = state
                .try_vardiff(nominal, &target_for(nominal, spm), spm)
                .unwrap()
            {
                nominal = new;
            }
        }
        let ratio = nominal as f64 / true_hashrate;
        assert!(
            (0.5..2.0).contains(&ratio),
            "did not reconverge after resume: {nominal:.3e} (ratio {ratio:.3})"
        );
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
