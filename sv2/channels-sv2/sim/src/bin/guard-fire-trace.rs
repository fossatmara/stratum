//! GUARD-FIRE TRACE — does PoissonCI EVER fire during a guard-regime escape, and
//! does recovery happen regardless? Settles the category question: is the spiral
//! threshold a property of PoissonCI (the gate) or of the estimator+update loop?
//!
//! The guard-threshold-probe found PoissonCI's single-tick fire-threshold is ~212%
//! at spm2 — so it should almost NEVER fire at sparse rate. If true, the recovery
//! seen in lowrate-sigma (+25% descending) is ESTIMATOR-driven (Ewma360 belief
//! tracking the true rate as shares arrive), NOT boundary-driven, and the guard
//! regime is NOT a different spiral-controller — it's the SAME estimator+update loop
//! (identical to dense, composed.rs:262) behind a more-conservative, rarely-firing
//! gate. That would make "PoissonCI's spiral threshold" a category error: the gate
//! doesn't correct, so it has no correction-failure depth.
//!
//! This traces one guard escape (spm2, worst severity) tick by tick over the
//! catch-up window, printing: e, the fire threshold, whether it fired, and the
//! belief — to see (a) how often PoissonCI fires, (b) whether e descends (recovers)
//! between/without fires. Champion config, guard rate.
//!
//! Usage: cargo run --release --bin guard-fire-trace

use std::sync::Arc;
use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptiveSignPersist, Composed, EwmaEstimator,
    SignPersistenceCusumBoundary,
};
use channels_sv2::vardiff::MockClock;
use vardiff_sim::baseline::{Phase, Scenario, DEFAULT_BASELINE_SEED, TRUE_HASHRATE};
use vardiff_sim::grid::{AlgorithmSpec, VardiffBox};
use vardiff_sim::trial::{run_trial_observed, TrialConfig};

fn champion() -> AlgorithmSpec {
    AlgorithmSpec::new(format!("champion"), move |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(360),
            AdaptiveSignPersist::sign_persist(
                SignPersistenceCusumBoundary::new(1.5, 0.05, 8.0, 0.06, 0.6), 6,
            ),
            AcceleratingPartialRetarget::new(0.2, 0.6, 0.05), 1.0, clock,
        )))
    })
}

fn main() {
    let spm = 2.0f32;
    let rate_pph = 40.0f32; // worst severity (steepest decline)
    let seed = DEFAULT_BASELINE_SEED ^ 0xF18E;
    let a = champion();

    let mature = 60u64;
    let rate = rate_pph / 100.0 / 60.0;
    let target = 0.50f32;
    let observe = 120u64;
    let mut phases = vec![Phase::Hold { secs: mature * 60, h: TRUE_HASHRATE }];
    let mut dm = 0u64;
    for m in 0..300u64 {
        let frac = (rate * (m as f32 + 1.0)).min(target);
        phases.push(Phase::Hold { secs: 60, h: TRUE_HASHRATE * (1.0 - frac) });
        dm = m + 1;
        if frac >= target { break; }
    }
    let floor_h = TRUE_HASHRATE * (1.0 - (rate * dm as f32).min(target));
    phases.push(Phase::Hold { secs: observe * 60, h: floor_h });
    let scen = Scenario::Custom { name: "decline".into(), phases, initial_estimate: None };
    let (proto, sched) = scen.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..proto };
    let clock = Arc::new(MockClock::new(0));
    let v = (a.factory)(clock.clone());
    let d_start = mature * 60;
    let d_end = (mature + dm) * 60;
    let trial_end = d_end + observe * 60;
    let t = run_trial_observed(v, clock, config, &sched, seed);

    println!("# GUARD-FIRE TRACE — spm={}, decline {}%/hr, champion. Does PoissonCI fire during the escape?\n", spm as u32, rate_pph as u32);
    println!("Decline d_start={}min d_end={}min; catch-up = d_end..{}min.\n", d_start/60, d_end/60, trial_end/60);
    println!("| t(min) | true H (norm) | e% | fire thresh% | FIRED | n_sh | belief (norm) |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let h0 = TRUE_HASHRATE as f64;
    let (mut fires_decline, mut fires_catchup, mut ticks_catchup) = (0u32, 0u32, 0u32);
    for tk in &t.ticks {
        let h_true = sched.at(tk.t_secs.saturating_sub(30)) as f64;
        let e = (tk.current_hashrate_before as f64 / h_true).ln() * 100.0;
        let tmin = tk.t_secs / 60;
        if tk.t_secs > d_start && tk.t_secs <= trial_end {
            if tk.fired { if tk.t_secs <= d_end { fires_decline += 1; } else { fires_catchup += 1; } }
            if tk.t_secs > d_end { ticks_catchup += 1; }
            // print every 5 min in the decline + early catch-up, all fires
            if tmin % 5 == 0 || tk.fired {
                println!("| {} | {:.2} | {:+.0} | {} | {} | {} | {:.2} |",
                    tmin, h_true / h0, e,
                    tk.threshold.map(|x| format!("{:.0}", x)).unwrap_or_else(|| "—".into()),
                    if tk.fired {"**YES**"} else {"no"},
                    tk.n_shares, tk.current_hashrate_before as f64 / h0);
            }
        }
    }
    println!("\nFIRES during decline: {} | FIRES during catch-up: {} (over {} catch-up ticks).", fires_decline, fires_catchup, ticks_catchup);
    println!("\nREAD: if FIRES≈0 during the escape, PoissonCI is a near-always-HOLD gate at guard rate — the recovery (e descending) is");
    println!("ESTIMATOR-driven (Ewma360 belief tracking true rate), NOT boundary-driven. Then 'PoissonCI's spiral threshold' is a CATEGORY");
    println!("ERROR: the gate doesn't correct, so it has no correction-failure depth. The spiral question is the estimator+update loop's —");
    println!("IDENTICAL in both regimes (composed.rs:262) — already characterized sub-spiral in the dense regime. If FIRES are frequent,");
    println!("the gate IS active and a fire-based threshold is the right object — rethink the guard-threshold-probe's single-tick model.");
}
