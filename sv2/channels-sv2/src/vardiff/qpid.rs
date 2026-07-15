//! Q-learning gain scheduling on top of the PID vardiff controller.
//!
//! The PID's fixed gains are a compromise: values that react fast to a real
//! hashrate step also chase noise on a converged miner, and vice versa. This
//! module learns, per operating regime, which gains work — a tabular
//! Q-learning agent observes each measurement window, classifies it into a
//! discrete state, and nudges `kp`/`ki` up or down, reinforced by whether the
//! *next* window's error shrank.
//!
//! # Do no harm
//!
//! The learner sits on top of a deliberately strong base tuning
//! ([`QPidVardiffState::tuned_base`]) and is gated so it can only *improve* on
//! it, never degrade it — the failure mode of naive gain scheduling, whose
//! exploratory swings on a sample-starved shared table underperformed a
//! well-tuned static PID. Two gates enforce this:
//!
//! - **Confidence gate** ([`QTable::confident_action`]): a learned action
//!   overrides the base gains only where it has enough visits *and* a clear
//!   Q-advantage over "keep"; otherwise the state defers to the base tuning.
//!   Untrained and thin states therefore behave exactly like the static PID.
//! - **Converged freeze**: within [`CONVERGED_ERROR`] of setpoint the learner
//!   exploits greedily (no exploration), so a settled miner is never perturbed
//!   by an exploratory gain step it didn't need. Exploration happens only in
//!   the transient regimes where a better gain actually pays off.
//!
//! # State space (36 states)
//!
//! All features derive from the log-space [`WindowObservation`], so states
//! are miner-size-invariant (the same reason one set of PID gains works for
//! any hashrate):
//!
//! - `|raw_error|` magnitude: `<0.1`, `<0.3`, `<1.0`, `>=1.0` (4 buckets)
//! - error trend (filtered log-measurement slope): falling / flat / rising (3)
//! - confidence (`N_eff`): `<8`, `<32`, `>=32` shares (3)
//!
//! # Actions (27)
//!
//! `kp x {0.8, 1.0, 1.25}` crossed with `ki x {0.8, 1.0, 1.25}` crossed with
//! `kd x {0.8, 1.0, 1.25}`, clamped to bounds. Multiplicative nudges keep the
//! walk scale-free. `kd` usually starts at zero (differentiating Poisson
//! noise rarely helps), so the increase action bootstraps it to a small
//! floor first and the decrease action can switch it back off — the learner
//! decides per regime whether derivative action earns its keep.
//!
//! Exploration is *guided*: an epsilon-greedy step perturbs the greedy action
//! along a single axis by a single step (see [`QTable::explore_from`]) rather
//! than jumping to a uniformly random one of the 27, so the worst gain move
//! ever applied to a live miner is one `GAIN_STEP` on one gain.
//!
//! # Reward
//!
//! Observed at the next evaluation: `-(e'^2) - churn * emitted`. Squared
//! log-error punishes both slow reaction (error persists) and overshoot
//! (error flips sign and grows); the churn term discourages gains that emit
//! `SetTarget` updates a converged miner didn't need.
//!
//! # Sharing
//!
//! The Q-table is shared (`Arc<Mutex<..>>`) across every channel of a pool:
//! all miners teach one policy, so cold-starting channels inherit fleet
//! experience while keeping their own gain state.

use std::sync::{Arc, Mutex};

use bitcoin::Target;
use tracing::debug;

use super::{
    error::VardiffError,
    pid::{PidParams, PidVardiffState, WindowObservation},
    Vardiff,
};

/// Learning rate.
pub const DEFAULT_ALPHA: f64 = 0.15;
/// Discount factor. Short horizon: the plant reacts within a window or two.
pub const DEFAULT_GAMMA: f64 = 0.7;
/// Initial exploration rate; decays with table experience. Kept low: the
/// confidence gate ([`QTable::confident_action`]) means unexplored actions
/// cost nothing (the state defers to the tuned base gains), so exploration
/// buys evidence rather than being the only thing standing between a state and
/// a good action — and every exploratory step is a real gain perturbation on a
/// live miner, so a little goes a long way.
pub const DEFAULT_EPSILON: f64 = 0.05;
/// Experience half-life of the exploration rate: after this many recorded
/// decisions the effective epsilon is half the configured value.
const EPSILON_HALF_LIFE: f64 = 2000.0;
/// Exploration never drops below this (the environment is non-stationary:
/// miners come, go, and change behavior). Kept low because a nonzero floor is
/// a *permanent* perturbation applied to every converged miner; guided
/// single-axis exploration ([`QTable::explore_from`]) makes each such step
/// gentle enough that 1% suffices to keep tracking a changing fleet.
const EPSILON_FLOOR: f64 = 0.01;
/// Churn penalty per emitted update in the reward. Sized so that near
/// convergence (small squared log-error) it meaningfully discourages gains
/// that emit a `SetTarget` a settled miner didn't need, while a genuine
/// correction (error term up to `MAX_ABS_ERROR^2 = 9`) still dominates.
const CHURN_PENALTY: f64 = 0.1;
/// Multiplicative gain nudge per action step.
const GAIN_STEP: f64 = 1.25;
/// kp bounds. Upper bound sits above the tuned base kp (0.9) so the learner can
/// nudge the gain up or down; live sweeps showed kp up to ~1.2 stays stable at
/// the shipped significance bars, so 1.5 gives headroom without inviting a
/// runaway gain.
const KP_BOUNDS: (f64, f64) = (0.1, 1.5);
/// ki bounds (per second).
const KI_BOUNDS: (f64, f64) = (0.0005, 0.05);
/// kd bounds. Zero disables derivative action entirely.
const KD_BOUNDS: (f64, f64) = (0.0, 0.5);
/// Smallest non-zero kd; the increase action bootstraps a zero kd here.
const KD_FLOOR: f64 = 0.01;
/// Log-error magnitude under which a miner counts as converged. Below this the
/// learner exploits greedily (no exploration): a settled channel is held by
/// the base gains, and perturbing them buys nothing but churn. |e|<0.2 is a
/// ~22% rate deviation, comfortably inside the ±20% convergence band's noise.
const CONVERGED_ERROR: f64 = 0.2;
/// Minimum visits on a learned action before it may override the base gains.
/// Below this the state defers to KEEP (the tuned base). Sized so a single
/// miner's handful of windows can't flip the fleet policy on noise.
const MIN_CONFIDENT_VISITS: u32 = 30;
/// Q-advantage over KEEP a learned action must show before it's exploited.
/// Rewards are `-(e^2) - churn`, so this is roughly "the learned action must
/// look at least a modest error-reduction better than holding the base gains."
const CONFIDENT_MARGIN: f64 = 0.05;

pub const STATES: usize = 36;
pub const ACTIONS: usize = 27;
/// Index of the keep/keep/keep action (no gain change).
const KEEP_ACTION: usize = 13;

/// Tabular Q-values shared across all channels of a pool.
#[derive(Debug)]
pub struct QTable {
    q: Vec<f64>,
    visits: Vec<u32>,
    total: u64,
    /// xorshift64 state for epsilon-greedy exploration.
    rng: u64,
}

impl QTable {
    pub fn new() -> Self {
        Self::with_seed(super::sim_clock::now_secs_f64().to_bits())
    }

    /// Deterministic construction for reproducible tests and experiments.
    pub fn with_seed(seed: u64) -> Self {
        // A tiny optimistic seed on the neutral action makes untrained
        // states default to "keep the gains" instead of an arbitrary
        // tie-break drifting them; exploration still visits everything.
        let mut q = vec![0.0; STATES * ACTIONS];
        for state in 0..STATES {
            q[state * ACTIONS + KEEP_ACTION] = 1e-3;
        }
        Self {
            q,
            visits: vec![0; STATES * ACTIONS],
            total: 0,
            rng: seed | 1,
        }
    }

    fn next_random(&mut self) -> f64 {
        // xorshift64: no external RNG dependency needed for epsilon-greedy.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    fn best_action(&self, state: usize) -> usize {
        let row = &self.q[state * ACTIONS..(state + 1) * ACTIONS];
        row.iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn choose(&mut self, state: usize, epsilon: f64) -> usize {
        let greedy = self.confident_action(state);
        if self.next_random() < epsilon {
            self.explore_from(greedy)
        } else {
            greedy
        }
    }

    /// The action to exploit: the learned best action *only* where the table
    /// has earned the right to override the base gains — enough visits on that
    /// action AND a Q-advantage over KEEP that clears a margin. Otherwise
    /// [`KEEP_ACTION`], so a thin or ambiguous state defers to the (well-tuned)
    /// base PID instead of committing to a gain move backed by a handful of
    /// noisy windows. This is what keeps qpid pinned to its strong static
    /// ceiling until learning genuinely knows better; the shared 36×27 table is
    /// sample-starved per state, and acting on unconverged Q-values was costing
    /// more (in transient churn and missed convergence) than the policy earned.
    fn confident_action(&self, state: usize) -> usize {
        let base = state * ACTIONS;
        let best = self.best_action(state);
        if best == KEEP_ACTION {
            return KEEP_ACTION;
        }
        let enough_visits = self.visits[base + best] >= MIN_CONFIDENT_VISITS;
        let advantage = self.q[base + best] - self.q[base + KEEP_ACTION];
        if enough_visits && advantage >= CONFIDENT_MARGIN {
            best
        } else {
            KEEP_ACTION
        }
    }

    /// Guided exploration: perturb the greedy action along exactly ONE gain
    /// axis by ONE step, instead of jumping to a uniformly random action.
    ///
    /// The action encodes `(kp_step, ki_step, kd_step)` in base 3, each step in
    /// `{0=down, 1=keep, 2=up}`. Uniform 27-way exploration can apply a
    /// simultaneous 3-axis gain swing to a *live* miner's difficulty (e.g.
    /// kp x0.8, ki x1.25, kd x1.25 at once) — a large, incoherent move on the
    /// hot path. Restricting exploration to a single-axis, single-step neighbor
    /// keeps learning going while bounding the worst applied move to one
    /// `GAIN_STEP` on one gain, and guarantees the explored action actually
    /// differs from greedy (so an exploration step is never wasted).
    fn explore_from(&mut self, greedy: usize) -> usize {
        let mut steps = [greedy / 9, (greedy / 3) % 3, greedy % 3];
        let axis = (self.next_random() * 3.0) as usize % 3;
        steps[axis] = match steps[axis] {
            // At a boundary the only in-bounds one-step move is toward keep.
            0 => 1,
            2 => 1,
            // From keep, explore up or down with equal probability.
            _ => {
                if self.next_random() < 0.5 {
                    0
                } else {
                    2
                }
            }
        };
        steps[0] * 9 + steps[1] * 3 + steps[2]
    }

    fn update(&mut self, state: usize, action: usize, reward: f64, next_state: usize, alpha: f64, gamma: f64) {
        let best_next = self.q
            [next_state * ACTIONS..(next_state + 1) * ACTIONS]
            .iter()
            .cloned()
            .fold(f64::MIN, f64::max);
        let idx = state * ACTIONS + action;
        self.q[idx] += alpha * (reward + gamma * best_next - self.q[idx]);
        self.visits[idx] = self.visits[idx].saturating_add(1);
        self.total += 1;
    }

    /// Total table visits (how much experience the policy has absorbed).
    pub fn total_visits(&self) -> u64 {
        self.total
    }

    /// Exploration rate after experience decay: `e0 * H / (H + total)`,
    /// floored so a trained policy still tracks a changing fleet.
    pub fn effective_epsilon(&self, initial: f64) -> f64 {
        (initial * EPSILON_HALF_LIFE / (EPSILON_HALF_LIFE + self.total as f64)).max(EPSILON_FLOOR)
    }

    /// Greedy (kp, ki, kd) multipliers for a state, for introspection.
    pub fn greedy_action(&self, state: usize) -> (f64, f64, f64) {
        action_multipliers(self.best_action(state))
    }

    /// Serializes the learned policy to a self-describing little-endian blob so
    /// a pool can persist experience across restarts (see [`QTable::decode`]).
    ///
    /// Layout: `MAGIC(4) | VERSION(2) | STATES(u32) | ACTIONS(u32) | rng(u64) |
    /// total(u64) | q[STATES*ACTIONS] as f64 | visits[STATES*ACTIONS] as u32`.
    /// `STATES`/`ACTIONS` are embedded so a blob written by a build with a
    /// different state/action layout is rejected on load rather than silently
    /// misread. The live `rng` is persisted too, so a reloaded table does not
    /// replay the same exploration sequence every boot.
    pub fn encode(&self) -> Vec<u8> {
        let n = STATES * ACTIONS;
        let mut buf = Vec::with_capacity(Self::HEADER_LEN + n * (8 + 4));
        buf.extend_from_slice(&Self::MAGIC);
        buf.extend_from_slice(&Self::VERSION.to_le_bytes());
        buf.extend_from_slice(&(STATES as u32).to_le_bytes());
        buf.extend_from_slice(&(ACTIONS as u32).to_le_bytes());
        buf.extend_from_slice(&self.rng.to_le_bytes());
        buf.extend_from_slice(&self.total.to_le_bytes());
        for &v in &self.q {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &self.visits {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf
    }

    /// Reconstructs a table from [`QTable::encode`] output. Returns
    /// [`VardiffError::InvalidQTableBlob`] if the magic, version, or embedded
    /// `STATES`/`ACTIONS` don't match this build, or the length is short — a
    /// stale or corrupt file is rejected so the caller can fall back to a fresh
    /// table instead of learning from garbage.
    pub fn decode(bytes: &[u8]) -> Result<Self, VardiffError> {
        let n = STATES * ACTIONS;
        let expected = Self::HEADER_LEN + n * (8 + 4);
        if bytes.len() != expected {
            return Err(VardiffError::InvalidQTableBlob);
        }
        if bytes[0..4] != Self::MAGIC {
            return Err(VardiffError::InvalidQTableBlob);
        }
        let mut off = 4;
        let take = |off: &mut usize, len: usize| {
            let s = &bytes[*off..*off + len];
            *off += len;
            s
        };
        let version = u16::from_le_bytes(take(&mut off, 2).try_into().unwrap());
        let states = u32::from_le_bytes(take(&mut off, 4).try_into().unwrap()) as usize;
        let actions = u32::from_le_bytes(take(&mut off, 4).try_into().unwrap()) as usize;
        if version != Self::VERSION || states != STATES || actions != ACTIONS {
            return Err(VardiffError::InvalidQTableBlob);
        }
        let rng = u64::from_le_bytes(take(&mut off, 8).try_into().unwrap());
        let total = u64::from_le_bytes(take(&mut off, 8).try_into().unwrap());
        let mut q = Vec::with_capacity(n);
        for _ in 0..n {
            q.push(f64::from_le_bytes(take(&mut off, 8).try_into().unwrap()));
        }
        let mut visits = Vec::with_capacity(n);
        for _ in 0..n {
            visits.push(u32::from_le_bytes(take(&mut off, 4).try_into().unwrap()));
        }
        Ok(Self {
            q,
            visits,
            total,
            // A zero rng would freeze xorshift64 (a corrupt/zeroed blob); any
            // nonzero state round-trips exactly, since xorshift64 never reaches
            // zero from a nonzero seed.
            rng: if rng == 0 { 1 } else { rng },
        })
    }

    const MAGIC: [u8; 4] = *b"QPID";
    const VERSION: u16 = 1;
    const HEADER_LEN: usize = 4 + 2 + 4 + 4 + 8 + 8;
}

impl Default for QTable {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedQTable = Arc<Mutex<QTable>>;

/// Tuning for the learning layer (the PID underneath keeps [`PidParams`]).
#[derive(Debug, Clone, Copy)]
pub struct QPidParams {
    pub alpha: f64,
    pub gamma: f64,
    pub epsilon: f64,
}

impl Default for QPidParams {
    fn default() -> Self {
        Self {
            alpha: DEFAULT_ALPHA,
            gamma: DEFAULT_GAMMA,
            epsilon: DEFAULT_EPSILON,
        }
    }
}

fn state_index(obs: &WindowObservation) -> usize {
    let mag = match obs.raw_error.abs() {
        e if e < 0.1 => 0,
        e if e < 0.3 => 1,
        e if e < 1.0 => 2,
        _ => 3,
    };
    let trend = match obs.derivative {
        d if d < -0.005 => 0,
        d if d > 0.005 => 2,
        _ => 1,
    };
    let confidence = match obs.n_eff {
        n if n < 8.0 => 0,
        n if n < 32.0 => 1,
        _ => 2,
    };
    (mag * 3 + trend) * 3 + confidence
}

/// (kp, ki, kd) multipliers for an action index.
fn action_multipliers(action: usize) -> (f64, f64, f64) {
    const STEPS: [f64; 3] = [1.0 / GAIN_STEP, 1.0, GAIN_STEP];
    (STEPS[action / 9], STEPS[(action / 3) % 3], STEPS[action % 3])
}

/// Applies a kd multiplier with the zero-escape rule.
fn nudge_kd(kd: f64, mul: f64) -> f64 {
    if kd < KD_FLOOR {
        // Off (or effectively off): only the increase action turns it on.
        if mul > 1.0 {
            KD_FLOOR
        } else {
            0.0
        }
    } else {
        let next = (kd * mul).clamp(KD_BOUNDS.0, KD_BOUNDS.1);
        // Decreasing below the floor switches derivative action off.
        if next < KD_FLOOR {
            0.0
        } else {
            next
        }
    }
}

/// PID vardiff controller whose gains are adapted online by Q-learning.
#[derive(Debug)]
pub struct QPidVardiffState {
    pid: PidVardiffState,
    table: SharedQTable,
    params: QPidParams,
    kp: f64,
    ki: f64,
    kd: f64,
    /// (state, action) taken at the previous window, rewarded at the next.
    pending: Option<(usize, usize)>,
}

impl QPidVardiffState {
    pub fn new(table: SharedQTable) -> Result<Self, VardiffError> {
        Self::with_params(table, QPidParams::default(), Self::tuned_base())
    }

    /// The base PID tuning qpid schedules gains *around*, tuned for convergence
    /// **and** steady-state bandwidth jointly and validated on the live pool
    /// benchmark (not just the offline fitness harness — see the note on
    /// `significance_z_down`). Differs from [`PidParams::default`] in three ways:
    ///
    /// - **`kp` 0.9** (vs 0.6): closes the gap to the implied estimate in fewer,
    ///   larger moves, so a step re-enters the ±20% band faster. Safe to raise
    ///   only because the significance bars below are *not* loosened — a fast
    ///   gain behind a low bar overshoots and rings.
    /// - **`significance_z_down` 2.5** (vs 1.5): raised toward the up-bar. This
    ///   is the key bandwidth lever, and it is the *opposite* of what the
    ///   offline harness suggested — the harness's clean Poisson model rewards a
    ///   low down-bar (fast drop reaction), but on the live pool (share-delivery
    ///   latency, reconnects, idle backstop) a low down-bar makes the controller
    ///   eager to raise difficulty on any dip yet slow to lower it when fast, so
    ///   the operating point *rectifies high* — miners settle at 4.5–6× setpoint
    ///   and burn proportional extra share bandwidth. A near-symmetric bar keeps
    ///   the equilibrium centered on setpoint (share submissions dominate wire
    ///   traffic, so a centered operating point is the bandwidth win).
    /// - **`confidence_k` 4.0** (vs 2.0): discounts thin, few-share windows a
    ///   little harder so the controller acts on evidence rather than Poisson
    ///   noise, without over-damping response (K=8 was worse live).
    ///
    /// `significance_z` stays at the default 3σ up-bar: at the low
    /// shares-per-minute setpoints this pool runs, that keeps noise-triggered
    /// upward churn on converged miners negligible.
    pub fn tuned_base() -> PidParams {
        PidParams {
            kp: 0.9,
            ki: 0.008,
            significance_z_down: 2.5,
            confidence_k: 4.0,
            ..PidParams::default()
        }
    }

    pub fn with_params(
        table: SharedQTable,
        params: QPidParams,
        pid_params: PidParams,
    ) -> Result<Self, VardiffError> {
        let kp = pid_params.kp;
        let ki = pid_params.ki;
        let kd = pid_params.kd;
        let mut pid = PidVardiffState::with_params(pid_params)?;
        // The learner owns the gains; manual live overrides must not fight
        // its per-decision scheduling.
        pid.set_adaptive_gains();
        Ok(Self {
            pid,
            table,
            params,
            kp,
            ki,
            kd,
            pending: None,
        })
    }

    /// Current adapted gains, for introspection.
    pub fn gains(&self) -> (f64, f64, f64) {
        (self.kp, self.ki, self.kd)
    }

    /// Enables gain telemetry (see [`PidVardiffState::set_telemetry_key`]);
    /// the learner's per-decision gain updates publish automatically through
    /// the inner controller.
    pub fn set_telemetry_key(&mut self, key: String) {
        self.pid.set_telemetry_key(key);
    }

    fn learn_and_schedule(&mut self, obs: &WindowObservation) {
        let state = state_index(obs);
        let mut table = match self.table.lock() {
            Ok(t) => t,
            Err(_) => return, // poisoned table: keep current gains
        };

        if let Some((prev_state, prev_action)) = self.pending.take() {
            let reward =
                -(obs.raw_error * obs.raw_error) - CHURN_PENALTY * (obs.emitted as u8 as f64);
            table.update(
                prev_state,
                prev_action,
                reward,
                state,
                self.params.alpha,
                self.params.gamma,
            );
        }

        // Do no harm when converged: near setpoint the base gains already hold
        // the miner, and any exploratory gain swing is a *permanent*
        // perturbation applied to a settled channel — which is exactly what
        // made naive gain scheduling underperform a well-tuned static PID.
        // Exploit greedily here (epsilon=0) so learning never disturbs a
        // converged miner; the learner still explores in transient regimes.
        let converged = obs.raw_error.abs() < CONVERGED_ERROR;
        let epsilon = if converged {
            0.0
        } else {
            table.effective_epsilon(self.params.epsilon)
        };
        let action = table.choose(state, epsilon);
        drop(table);

        let (kp_mul, ki_mul, kd_mul) = action_multipliers(action);
        self.kp = (self.kp * kp_mul).clamp(KP_BOUNDS.0, KP_BOUNDS.1);
        self.ki = (self.ki * ki_mul).clamp(KI_BOUNDS.0, KI_BOUNDS.1);
        self.kd = nudge_kd(self.kd, kd_mul);
        self.pid.set_gains(self.kp, self.ki, self.kd);
        self.pending = Some((state, action));

        debug!(
            target: "vardiff",
            "QPID: state={state} action={action} kp={:.4} ki={:.5} kd={:.4} eps={epsilon:.4} |e|={:.4} emitted={}",
            self.kp, self.ki, self.kd, obs.raw_error.abs(), obs.emitted,
        );
    }
}

impl Vardiff for QPidVardiffState {
    fn kind(&self) -> super::VardiffKind {
        super::VardiffKind::QPid
    }

    fn last_update_timestamp(&self) -> u64 {
        self.pid.last_update_timestamp()
    }

    fn shares_since_last_update(&self) -> u32 {
        self.pid.shares_since_last_update()
    }

    fn min_allowed_hashrate(&self) -> f32 {
        self.pid.min_allowed_hashrate()
    }

    fn set_timestamp_of_last_update(&mut self, timestamp: u64) {
        self.pid.set_timestamp_of_last_update(timestamp);
    }

    fn increment_shares_since_last_update(&mut self) {
        self.pid.increment_shares_since_last_update();
    }

    fn reset_counter(&mut self) -> Result<(), VardiffError> {
        self.pid.reset_counter()
    }

    fn try_vardiff(
        &mut self,
        hashrate: f32,
        target: &Target,
        shares_per_minute: f32,
    ) -> Result<Option<f32>, VardiffError> {
        let (update, observation) =
            self.pid
                .try_vardiff_observed(hashrate, target, shares_per_minute)?;
        if let Some(obs) = observation {
            self.learn_and_schedule(&obs);
        }
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::hash_rate_to_target;

    fn target_for(hashrate: f32, spm: f32) -> Target {
        hash_rate_to_target(hashrate as f64, spm as f64)
            .expect("valid target")
            .into()
    }

    fn backdate(state: &mut QPidVardiffState, secs: u64) {
        state.set_timestamp_of_last_update(state.last_update_timestamp() - secs);
    }

    fn obs(raw_error: f64, derivative: f64, n_eff: f64) -> WindowObservation {
        WindowObservation {
            raw_error,
            n_eff,
            derivative,
            dt: 60.0,
            emitted: false,
        }
    }

    /// Decodes an action index back to its `(kp, ki, kd)` steps in `{0,1,2}`.
    fn action_steps(a: usize) -> [usize; 3] {
        [a / 9, (a / 3) % 3, a % 3]
    }

    #[test]
    fn exploration_perturbs_exactly_one_axis_by_one_step() {
        let mut table = QTable::with_seed(12345);
        // Greedy action = keep/keep/keep so any single-axis nudge is reachable
        // up or down; every explored action must differ from greedy on exactly
        // one axis, and never touch a second.
        let greedy = KEEP_ACTION;
        let g = action_steps(greedy);
        let mut saw_non_greedy = false;
        for _ in 0..2000 {
            let a = table.explore_from(greedy);
            let s = action_steps(a);
            let diffs: Vec<usize> = (0..3).filter(|&i| s[i] != g[i]).collect();
            assert!(
                diffs.len() <= 1,
                "explored action {a:?} differs from greedy {g:?} on {} axes",
                diffs.len()
            );
            for &i in &diffs {
                assert!(
                    s[i].abs_diff(g[i]) == 1,
                    "axis {i} moved more than one step: {g:?} -> {s:?}"
                );
            }
            if !diffs.is_empty() {
                saw_non_greedy = true;
            }
        }
        assert!(saw_non_greedy, "exploration never left the greedy action");
    }

    #[test]
    fn exploration_from_boundary_stays_in_bounds() {
        let mut table = QTable::with_seed(999);
        // Greedy at down/up/down = steps (0,2,0) => index 6: the only in-bounds
        // single-step moves are toward keep on whichever axis is chosen.
        let greedy = 6;
        assert_eq!(action_steps(greedy), [0, 2, 0]);
        for _ in 0..1000 {
            let a = table.explore_from(greedy);
            for &step in &action_steps(a) {
                assert!(step <= 2, "step {step} out of range for action {a}");
            }
        }
    }

    #[test]
    fn qtable_encode_decode_round_trips() {
        let mut table = QTable::with_seed(42);
        // Train into a non-trivial state so q/visits/total/rng all differ from
        // a fresh table.
        for i in 0..500 {
            let state = i % STATES;
            let action = (i * 7) % ACTIONS;
            table.update(state, action, (i as f64 % 5.0) - 2.0, (i + 1) % STATES, 0.15, 0.7);
        }
        let _ = table.choose(3, 0.5); // advance rng
        let blob = table.encode();
        let restored = QTable::decode(&blob).expect("round-trip decodes");
        assert_eq!(restored.total_visits(), table.total_visits());
        assert_eq!(restored.q, table.q);
        assert_eq!(restored.visits, table.visits);
        assert_eq!(restored.rng, table.rng);
        // Greedy policy is preserved for every state.
        for s in 0..STATES {
            assert_eq!(restored.best_action(s), table.best_action(s));
        }
    }

    #[test]
    fn qtable_decode_rejects_bad_blobs() {
        let good = QTable::with_seed(1).encode();
        // Truncated.
        assert!(matches!(
            QTable::decode(&good[..good.len() - 1]),
            Err(VardiffError::InvalidQTableBlob)
        ));
        // Wrong magic.
        let mut bad_magic = good.clone();
        bad_magic[0] ^= 0xFF;
        assert!(matches!(
            QTable::decode(&bad_magic),
            Err(VardiffError::InvalidQTableBlob)
        ));
        // Wrong version (bytes 4..6).
        let mut bad_ver = good.clone();
        bad_ver[4] = bad_ver[4].wrapping_add(1);
        assert!(matches!(
            QTable::decode(&bad_ver),
            Err(VardiffError::InvalidQTableBlob)
        ));
        // Empty.
        assert!(matches!(
            QTable::decode(&[]),
            Err(VardiffError::InvalidQTableBlob)
        ));
    }

    #[test]
    fn state_index_covers_all_buckets_in_range() {
        for &e in &[0.0, 0.15, 0.5, 2.0] {
            for &d in &[-0.01, 0.0, 0.01] {
                for &n in &[2.0, 16.0, 100.0] {
                    let s = state_index(&obs(e, d, n));
                    assert!(s < STATES, "state {s} out of range");
                }
            }
        }
    }

    #[test]
    fn q_update_reinforces_good_actions() {
        let mut table = QTable::new();
        // Action 13 (keep/keep/keep) repeatedly rewarded in state 0.
        for _ in 0..50 {
            table.update(0, 13, 1.0, 0, 0.2, 0.5);
            table.update(0, 0, -1.0, 0, 0.2, 0.5);
        }
        assert_eq!(table.best_action(0), 13);
    }

    #[test]
    fn greedy_when_epsilon_zero() {
        let mut table = QTable::new();
        // A learned action is only exploited once it clears the confidence
        // gate (visits + advantage), so train action 7 well past both bars.
        for _ in 0..MIN_CONFIDENT_VISITS + 5 {
            table.update(3, 7, 5.0, 3, 0.5, 0.0);
        }
        for _ in 0..20 {
            assert_eq!(table.choose(3, 0.0), 7);
        }
    }

    #[test]
    fn thin_state_defers_to_keep() {
        // An action that looks best but has too few visits must NOT override
        // the base gains: the state falls back to KEEP so a sample-starved
        // cell can't move a live miner off the tuned base on noise.
        let mut table = QTable::new();
        // One lucky high-reward visit on action 7 — best_action prefers it,
        // but it is below MIN_CONFIDENT_VISITS.
        table.update(5, 7, 5.0, 5, 0.5, 0.0);
        assert_eq!(table.best_action(5), 7, "raw greedy should pick the lucky action");
        assert_eq!(
            table.choose(5, 0.0),
            KEEP_ACTION,
            "thin state must defer to KEEP until the action earns confidence"
        );
        // After enough confirming visits with a clear advantage, it takes over.
        for _ in 0..MIN_CONFIDENT_VISITS {
            table.update(5, 7, 5.0, 5, 0.5, 0.0);
        }
        assert_eq!(table.choose(5, 0.0), 7, "confident action should now be exploited");
    }

    #[test]
    fn epsilon_decays_with_experience_and_floors() {
        let mut table = QTable::new();
        assert!((table.effective_epsilon(0.1) - 0.1).abs() < 1e-9);
        for _ in 0..2000 {
            table.update(0, 0, 0.0, 0, 0.1, 0.5);
        }
        let half = table.effective_epsilon(0.1);
        assert!((half - 0.05).abs() < 1e-3, "expected ~half, got {half}");
        for _ in 0..100_000 {
            table.update(0, 0, 0.0, 0, 0.1, 0.5);
        }
        assert!((table.effective_epsilon(0.1) - EPSILON_FLOOR).abs() < 1e-9);
    }

    #[test]
    fn gains_stay_bounded_under_learning() {
        let table: SharedQTable = Arc::new(Mutex::new(QTable::with_seed(7)));
        let mut state = QPidVardiffState::new(table).unwrap();
        let target = target_for(100.0e12, 6.0);
        for i in 0..200 {
            // Alternate noisy windows to bounce through states.
            let shares = if i % 3 == 0 { 2 } else { 12 };
            for _ in 0..shares {
                state.increment_shares_since_last_update();
            }
            backdate(&mut state, 60);
            let _ = state.try_vardiff(100.0e12, &target, 6.0).unwrap();
            let (kp, ki, kd) = state.gains();
            assert!((KP_BOUNDS.0..=KP_BOUNDS.1).contains(&kp), "kp={kp}");
            assert!((KI_BOUNDS.0..=KI_BOUNDS.1).contains(&ki), "ki={ki}");
            assert!(
                (KD_BOUNDS.0..=KD_BOUNDS.1).contains(&kd) && (kd == 0.0 || kd >= KD_FLOOR),
                "kd={kd}"
            );
        }
        assert!(
            state.table.lock().unwrap().total_visits() > 100,
            "learning should have recorded visits"
        );
    }

    #[test]
    fn converges_in_closed_loop_like_pid() {
        // With learning active the controller must still converge a miner
        // whose true hashrate is 20x the estimate.
        let true_hashrate: f64 = 2000.0e12;
        let mut nominal: f32 = 100.0e12;
        let spm: f32 = 6.0;
        let table: SharedQTable = Arc::new(Mutex::new(QTable::with_seed(42)));
        let mut state = QPidVardiffState::new(table).unwrap();

        for _ in 0..24 {
            let target = target_for(nominal, spm);
            let hashrate_per_spm =
                crate::target::hash_rate_from_target(target.to_le_bytes().into(), 1.0)
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
        // Wide bound: epsilon-greedy exploration plus sub-second window
        // jitter make the exact trajectory run-dependent; the test guards
        // "learning does not prevent convergence", not tracking precision.
        assert!(
            (0.35..3.0).contains(&ratio),
            "expected rough convergence, got {nominal:.3e} (ratio {ratio:.3})"
        );
    }
}
