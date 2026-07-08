//! Q-learning gain scheduling on top of the PID vardiff controller.
//!
//! The PID's fixed gains are a compromise: values that react fast to a real
//! hashrate step also chase noise on a converged miner, and vice versa. This
//! module learns, per operating regime, which gains work — a tabular
//! Q-learning agent observes each measurement window, classifies it into a
//! discrete state, and nudges `kp`/`ki` up or down, reinforced by whether the
//! *next* window's error shrank.
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
/// Initial exploration rate; decays with table experience.
pub const DEFAULT_EPSILON: f64 = 0.1;
/// Experience half-life of the exploration rate: after this many recorded
/// decisions the effective epsilon is half the configured value.
const EPSILON_HALF_LIFE: f64 = 2000.0;
/// Exploration never drops below this (the environment is non-stationary:
/// miners come, go, and change behavior).
const EPSILON_FLOOR: f64 = 0.02;
/// Churn penalty per emitted update in the reward.
const CHURN_PENALTY: f64 = 0.05;
/// Multiplicative gain nudge per action step.
const GAIN_STEP: f64 = 1.25;
/// kp bounds.
const KP_BOUNDS: (f64, f64) = (0.1, 1.2);
/// ki bounds (per second).
const KI_BOUNDS: (f64, f64) = (0.0005, 0.05);
/// kd bounds. Zero disables derivative action entirely.
const KD_BOUNDS: (f64, f64) = (0.0, 0.5);
/// Smallest non-zero kd; the increase action bootstraps a zero kd here.
const KD_FLOOR: f64 = 0.01;

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
        if self.next_random() < epsilon {
            (self.next_random() * ACTIONS as f64) as usize % ACTIONS
        } else {
            self.best_action(state)
        }
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
        Self::with_params(table, QPidParams::default(), PidParams::default())
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

        let epsilon = table.effective_epsilon(self.params.epsilon);
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
        table.update(3, 7, 5.0, 3, 0.5, 0.0);
        for _ in 0..20 {
            assert_eq!(table.choose(3, 0.0), 7);
        }
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
