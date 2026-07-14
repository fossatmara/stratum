//! Deterministic closed-loop fitness harness for vardiff controllers.
//!
//! The live-pool benchmark is too noisy to optimize against (identical runs
//! swing several composite points, because convergence depends on a handful of
//! step events vs. Poisson share timing in a 1–3 miner fleet). This harness
//! removes that noise two ways: a **seeded** Poisson share model (reproducible
//! bit-for-bit) and **averaging over many seeds × disturbance profiles**, so a
//! small real improvement in a controller shows up as a stable score change.
//!
//! It drives the actual `Vardiff` implementations (classic / pid / qpid /
//! champion) in a closed loop — build target from nominal hashrate, draw a
//! Poisson share count for the true hashrate against that target over a 60s
//! window, feed the controller, apply any emitted retarget — with no pool,
//! network, or bitcoind. Runs in milliseconds.
//!
//! Run: `cargo run --release --example vardiff_fitness -p channels_sv2`

use channels_sv2::target::{hash_rate_from_target, hash_rate_to_target};
use channels_sv2::vardiff::{
    champion::ChampionVardiffState, classic::VardiffState, pid::PidVardiffState,
    qpid::QPidVardiffState, sim_clock, Vardiff,
};
use channels_sv2::{QTable, SharedQTable};
use std::sync::{Arc, Mutex};

const SPM: f32 = 4.0; // setpoint (matches the wall benchmark)
const TICK: u64 = 60; // controller evaluation cadence (virtual seconds)
const BAND: f64 = 0.20; // ±20% convergence band
const SETTLE_GUARD: f64 = 90.0; // seconds after an event excluded from "settled"

/// Deterministic xorshift64 → uniform [0,1), so every scenario/seed is
/// reproducible without an RNG dependency.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn u01(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Poisson draw (Knuth) — number of shares actually found in a window whose
    /// expected count is `lambda`.
    fn poisson(&mut self, lambda: f64) -> u32 {
        if lambda <= 0.0 {
            return 0;
        }
        if lambda > 30.0 {
            // Normal approximation for large lambda (Knuth underflows).
            let u1 = self.u01().max(1e-12);
            let u2 = self.u01();
            let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
            return (lambda + z * lambda.sqrt()).round().max(0.0) as u32;
        }
        let l = (-lambda).exp();
        let mut k = 0u32;
        let mut p = 1.0;
        loop {
            p *= self.u01();
            if p <= l {
                return k;
            }
            k += 1;
        }
    }
}

/// A disturbance profile: (name, true-hashrate as a function of elapsed secs,
/// event times for settled-window exclusion, total duration).
struct Scenario {
    name: &'static str,
    hr: fn(f64) -> f64,
    events: &'static [f64],
    dur: f64,
}

// Representative disturbance shapes spanning the metric dimensions.
fn scenarios() -> Vec<Scenario> {
    fn step_up(t: f64) -> f64 {
        if t < 120.0 { 100e12 } else { 1000e12 }
    }
    fn step_down(t: f64) -> f64 {
        if t < 120.0 { 1000e12 } else { 100e12 }
    }
    fn ramp_up(t: f64) -> f64 {
        // 4x every 60s from 10e12, capping at 640e12 by t=240.
        let steps = (t / 60.0).min(4.0) as i32;
        10e12 * 4f64.powi(steps.min(3))
    }
    fn crash(t: f64) -> f64 {
        if t < 180.0 { 300e12 } else { 3e12 } // 100x drop (curtailment)
    }
    fn spike_return(t: f64) -> f64 {
        if t < 120.0 {
            100e12
        } else if t < 360.0 {
            800e12
        } else {
            100e12
        }
    }
    fn steady(_t: f64) -> f64 {
        100e12
    }
    fn sawtooth(t: f64) -> f64 {
        // 200e12 <-> 50e12 every 60s.
        if ((t / 60.0) as i64) % 2 == 0 { 200e12 } else { 50e12 }
    }
    fn sandbag(_t: f64) -> f64 {
        // Miner truly at 300e12 but the pool opens it soft (handled via
        // nominal-start below); steady true rate, pool must tighten hard.
        300e12
    }
    vec![
        Scenario { name: "step-up", hr: step_up, events: &[120.0], dur: 600.0 },
        Scenario { name: "step-down", hr: step_down, events: &[120.0], dur: 600.0 },
        Scenario { name: "ramp-up", hr: ramp_up, events: &[60.0, 120.0, 180.0, 240.0], dur: 600.0 },
        Scenario { name: "crash", hr: crash, events: &[180.0], dur: 720.0 },
        Scenario { name: "spike-return", hr: spike_return, events: &[120.0, 360.0], dur: 720.0 },
        Scenario { name: "steady", hr: steady, events: &[], dur: 600.0 },
        Scenario { name: "sawtooth", hr: sawtooth, events: &[60.0, 120.0, 180.0, 240.0, 300.0, 360.0], dur: 600.0 },
        Scenario { name: "sandbag", hr: sandbag, events: &[], dur: 600.0 },
    ]
}

/// Per-run metrics accumulated across the closed loop.
#[derive(Default)]
struct Metrics {
    settled_err: Vec<f64>,
    updates: u64,
    // convergence: for each event, secs to re-enter band (or None).
    conv_up: Vec<f64>,
    conv_down: Vec<f64>,
    never: u32,
    peak_spm: f64,
}

fn build(algo: &str, table: &SharedQTable) -> Box<dyn Vardiff> {
    match algo {
        "classic" => Box::new(VardiffState::new().unwrap()),
        "pid" => Box::new(PidVardiffState::new().unwrap()),
        "qpid" => Box::new(QPidVardiffState::new(table.clone()).unwrap()),
        "champion" => Box::new(ChampionVardiffState::new().unwrap()),
        _ => unreachable!(),
    }
}

/// Run one closed-loop pass; returns metrics. `nominal0` is the pool's initial
/// hashrate estimate (differs from true rate for the sandbag case).
fn run(algo: &str, sc: &Scenario, seed: u64, table: &SharedQTable, nominal0: f64) -> Metrics {
    // Freeze the sim clock at a fixed virtual instant and advance it manually
    // per tick, so every run is bit-for-bit deterministic (no wall-clock drift
    // in the controllers' dt computation).
    sim_clock::freeze_at(1_000_000.0);
    let mut ctrl = build(algo, table);
    let mut rng = Rng::new(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(0xD1B54A32D192ED03));
    let mut nominal = nominal0 as f32;
    let mut m = Metrics::default();

    // Track expected_spm (the pool's belief-rate for the true hashrate at the
    // current target) — this is what the wall benchmark's metrics key off.
    let steps = (sc.dur / TICK as f64) as u64;
    // Per-event convergence bookkeeping.
    let mut event_dirs: Vec<(f64, i8)> = Vec::new();
    for &et in sc.events {
        let before = (sc.hr)(et - 1.0);
        let after = (sc.hr)(et + 1.0);
        if (after - before).abs() / before > 0.05 {
            event_dirs.push((et, if after > before { 1 } else { -1 }));
        }
    }
    let mut pending_events = event_dirs.clone();

    for i in 0..steps {
        let t = i as f64 * TICK as f64;
        // Advance virtual time by one window before evaluating, so the
        // controller measures a full TICK-second window deterministically.
        sim_clock::advance(TICK as f64);
        let true_hr = (sc.hr)(t);
        let target = hash_rate_to_target(nominal as f64, SPM as f64).expect("target");
        // Expected shares this window for the TRUE rate at the current target.
        let per_spm = hash_rate_from_target(target.to_le_bytes().into(), 1.0).expect("hr");
        let lambda = true_hr / per_spm; // expected shares in 60s
        let shares = rng.poisson(lambda);
        for _ in 0..shares {
            ctrl.increment_shares_since_last_update();
        }
        if let Some(new) = ctrl.try_vardiff(nominal, &target, SPM).unwrap_or(None) {
            nominal = new;
            m.updates += 1;
        }
        // expected_spm at the (possibly new) target for the true rate:
        let target2 = hash_rate_to_target(nominal as f64, SPM as f64).expect("t2");
        let per_spm2 = hash_rate_from_target(target2.to_le_bytes().into(), 1.0).expect("hr2");
        let expected_spm = true_hr / per_spm2;
        m.peak_spm = m.peak_spm.max(expected_spm);
        let rel = (expected_spm - SPM as f64).abs() / SPM as f64;

        let settled = t > 60.0 && sc.events.iter().all(|&e| t < e || t - e > SETTLE_GUARD);
        if settled {
            m.settled_err.push(rel);
        }
        // convergence: first tick after each event back in band
        for (et, dir) in pending_events.clone() {
            if t >= et && rel <= BAND {
                if dir == 1 {
                    m.conv_up.push(t - et);
                } else {
                    m.conv_down.push(t - et);
                }
                pending_events.retain(|(e, _)| *e != et);
            }
        }
    }
    // events that never re-entered the band
    m.never += pending_events.len() as u32;
    m
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((p / 100.0) * (s.len() - 1) as f64).round() as usize]
}
fn med(v: &[f64]) -> f64 {
    pct(v, 50.0)
}

fn main() {
    let algos = ["classic", "pid", "qpid", "champion"];
    let scs = scenarios();

    // DEBUG=<algo> prints the per-scenario convergence breakdown for one algo
    // (aggregated over all seeds) so you can see WHICH disturbances an
    // algorithm handles well or fails to converge.
    if let Ok(algo) = std::env::var("DEBUG") {
        let table: SharedQTable = Arc::new(Mutex::new(QTable::with_seed(0xC0FFEE)));
        for sc in &scs {
            let nominal0 = if sc.name == "sandbag" { 3e12 } else { 100e12 };
            let (mut up, mut down, mut never, mut steps) = (0usize, 0usize, 0u32, 0u32);
            for seed in 0..40u64 {
                let m = run(&algo, sc, seed, &table, nominal0);
                up += m.conv_up.len();
                down += m.conv_down.len();
                never += m.never;
                steps += m.conv_up.len() as u32 + m.conv_down.len() as u32 + m.never;
            }
            println!("{:<14} up={up:>3} down={down:>3} never={never:>3}/{steps:<3}", sc.name);
        }
        return;
    }
    let seeds: Vec<u64> = (0..40).collect(); // 40 seeds × 8 scenarios = 320 runs/algo

    println!(
        "{:<10}{:>11}{:>8}{:>9}{:>7}{:>8}{:>8}{:>8}{:>8}",
        "algo", "COMPOSITE", "conv", "settled", "churn", "s50", "s99", "peak", "never"
    );
    let mut scores: Vec<(String, f64)> = Vec::new();
    for algo in algos {
        // One shared Q-table per algo across all its runs (mirrors a pool that
        // accumulates experience); fresh per algo for a clean comparison.
        let table: SharedQTable = Arc::new(Mutex::new(QTable::with_seed(0xC0FFEE)));
        let (mut settled, mut churn) = (Vec::new(), Vec::new());
        let (mut up, mut down) = (Vec::new(), Vec::new());
        let (mut never, mut steps_ct, mut peak) = (0u32, 0u32, 0.0f64);
        for sc in &scs {
            let nominal0 = if sc.name == "sandbag" { 3e12 } else { 100e12 };
            for &seed in &seeds {
                let m = run(algo, sc, seed, &table, nominal0);
                settled.extend(&m.settled_err);
                let dur_hr = sc.dur / 3600.0;
                churn.push(m.updates as f64 / dur_hr);
                up.extend(&m.conv_up);
                down.extend(&m.conv_down);
                never += m.never;
                steps_ct += m.conv_up.len() as u32 + m.conv_down.len() as u32 + m.never;
                peak = peak.max(m.peak_spm);
            }
        }
        let s50 = med(&settled);
        let s99 = pct(&settled, 99.0);
        let churn_med = med(&churn);
        let up_med = med(&up);
        let down_med = med(&down);
        let never_rate = if steps_ct > 0 { never as f64 / steps_ct as f64 } else { 0.0 };
        // Composite (higher=better), same shape as the live scorer.
        let mut conv = 0.0;
        for v in [up_med, down_med] {
            if v.is_finite() {
                conv += 1.0 - (v.min(600.0) / 600.0);
            }
        }
        conv = conv / 2.0 - 1.5 * never_rate;
        let settled_s = -(if s50.is_finite() { s50 } else { 1.0 })
            - 0.5 * (if s99.is_finite() { s99.min(5.0) } else { 5.0 });
        let churn_s = -(churn_med.min(300.0) / 300.0);
        let composite = 2.0 * conv + 1.5 * settled_s + 0.5 * churn_s;
        scores.push((algo.to_string(), composite));
        println!(
            "{:<10}{:>11.3}{:>8.2}{:>9.2}{:>7.2}{:>8.3}{:>8.3}{:>8.0}{:>5}/{:<3}",
            algo, composite, conv, settled_s, churn_s, s50, s99, peak, never, steps_ct
        );
    }
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let rank = scores.iter().position(|(a, _)| a == "qpid").unwrap() + 1;
    println!("\nranking: {}", scores.iter().map(|(a, s)| format!("{a} {s:.3}")).collect::<Vec<_>>().join(" > "));
    println!("qpid rank: {rank}/4");
}
