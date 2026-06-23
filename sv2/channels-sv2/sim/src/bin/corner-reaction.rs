//! Is the −50% reaction gate even BINDING on the corner config? The reviewer's
//! key point: a −50% drop is enormous (CUSUM floor ~4–6 min at production
//! rates), so even the sleepy τ=720/s2.0 corner config may catch it fast —
//! making a −50% gate non-binding and back to the corner. This measures the
//! corner config's reaction time across drop magnitudes {−50,−25,−10} at
//! production spm, vs the responsive old champion, to decide the gate stimulus.
use std::sync::Arc;
use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptiveSignPersist, AdaptivePoissonCusum, AsymmetricCusumBoundary,
    Composed, EwmaEstimator, PoissonCI, SignPersistenceCusumBoundary,
};
use channels_sv2::vardiff::MockClock;
use vardiff_sim::baseline::{Scenario, DEFAULT_BASELINE_SEED};
use vardiff_sim::grid::{AlgorithmSpec, VardiffBox};
use vardiff_sim::reaction_time_distribution;
use vardiff_sim::trial::{run_trial_observed, TrialConfig};

// The degenerate corner config: long window, barely fires.
fn corner() -> AlgorithmSpec {
    AlgorithmSpec::new("corner(Ewma720/s2/t8)", |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(720),
            AdaptiveSignPersist::sign_persist(
                SignPersistenceCusumBoundary::new(2.0, 0.05, 8.0, 0.06, 0.6), 6),
            AcceleratingPartialRetarget::new(0.2, 0.6, 0.05), 1.0, clock,
        )))
    })
}
// The responsive old champion, for contrast.
fn champ() -> AlgorithmSpec { AlgorithmSpec::champion() }

fn react_p50_min(mk: &AlgorithmSpec, spm: f32, delta: i32, n: usize, seed: u64) -> (f64, f64) {
    // SettledStep with matured 60-min counter (the realistic case), measure
    // reaction within a 120-min window. Returns (rate_reacted, p50_min).
    let (cfg, sched) = Scenario::SettledStep { settle_minutes: 60, delta_pct: delta }.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..cfg };
    let event = 60 * 60u64;
    let mut trials = Vec::with_capacity(n);
    for i in 0..n {
        let clock = Arc::new(MockClock::new(0));
        let v = (mk.factory)(clock.clone());
        trials.push(run_trial_observed(v, clock, config.clone(), &sched, seed.wrapping_add(i as u64)));
    }
    let (rate, dist) = reaction_time_distribution(&trials, event, 120 * 60);
    (rate, dist.p50().unwrap_or(f64::NAN) / 60.0)
}

fn main() {
    let n = 2000usize;
    println!("## Is the −50% gate binding on the corner? Reaction (matured counter), 2000 trials.\n");
    println!("| spm | drop | corner: reacted%, p50 | champion: reacted%, p50 |");
    println!("| --- | --- | --- | --- |");
    let c = corner(); let h = champ();
    for spm in [4.0f32, 6.0, 30.0] {
        for delta in [-50i32, -25, -10] {
            let (cr, cp) = react_p50_min(&c, spm, delta, n, DEFAULT_BASELINE_SEED);
            let (hr, hp) = react_p50_min(&h, spm, delta, n, DEFAULT_BASELINE_SEED ^ 0x55);
            println!("| {} | {}% | {:.0}%, {:.1} min | {:.0}%, {:.1} min |",
                spm as u32, delta, cr*100.0, cp, hr*100.0, hp);
        }
    }
    println!("\nIf corner's −50% reaction is already fast (~min), the −50% gate is NON-BINDING");
    println!("→ need a moderate-drop (−10/−25) floor-relative gate. If corner is slow on −50%,");
    println!("the −50% gate binds and works as a hard reaction bound.");
}
