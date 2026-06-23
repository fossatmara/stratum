//! Convention-INDEPENDENT binding test. The flaw in the last round: a single
//! pinned ARL0 sets one floor for everyone, but the corner fires far less
//! than the champion, so they operate at DIFFERENT false-alarm rates and
//! can't share a floor convention. The right test:
//!   1. measure each config's stable-load fire rate → its OWN ARL0
//!      (mean minutes between spurious fires on steady H = the false-alarm
//!      budget that config actually produces — grounded in retarget cost,
//!      not the operator's monitoring cadence)
//!   2. score each config's −g reaction against the CUSUM floor AT ITS OWN
//!      ARL0 (ratio = reaction / floor(g, spm, that config's ARL0))
//!   3. the gate binds iff corner-ratio > champion-ratio when each is at its
//!      own operating point — convention-independent.
//!
//! Reports per-config ARL0 (the headline number: what budget do good configs
//! actually run?) and the re-scored −25%/−33% gap. CUSUM floor delays are
//! interpolated in ln(ARL0) from the simulated floor table (cusum-floor.rs).

use std::sync::Arc;
use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptiveSignPersist, Composed, EwmaEstimator,
    SignPersistenceCusumBoundary,
};
use channels_sv2::vardiff::MockClock;
use vardiff_sim::baseline::{Scenario, DEFAULT_BASELINE_SEED, TRUE_HASHRATE};
use vardiff_sim::grid::{AlgorithmSpec, VardiffBox};
use vardiff_sim::reaction_time_distribution;
use vardiff_sim::trial::{run_trial_observed, TrialConfig};

#[derive(Clone, Copy)]
struct Cfg { tau: u64, sens: f64, tighten: f64, eta_max: f32 }
fn spec(c: Cfg) -> AlgorithmSpec {
    AlgorithmSpec::new(format!("Ewma{}/s{}/t{}", c.tau, c.sens, c.tighten), move |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(c.tau),
            AdaptiveSignPersist::sign_persist(
                SignPersistenceCusumBoundary::new(c.sens, 0.05, c.tighten, 0.06, 0.6), 6),
            AcceleratingPartialRetarget::new(0.2, c.eta_max, 0.05), 1.0, clock,
        )))
    })
}

/// Stable-load false-alarm rate → ARL0 (mean MINUTES between spurious fires
/// on steady H, after a 30-min warmup to exclude cold-start convergence).
fn stable_arl0_min(c: Cfg, spm: f32, trials: usize, seed: u64) -> f64 {
    let dur = 6 * 60 * 60u64; // 6h stable
    let warmup = 30 * 60u64;
    let (cfg, sched) = Scenario::Custom {
        name: "stable6h".into(),
        phases: vec![vardiff_sim::baseline::Phase::Hold { secs: dur, h: TRUE_HASHRATE }],
        initial_estimate: None,
    }.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..cfg };
    let a = spec(c);
    let (mut fires, mut secs) = (0u64, 0u64);
    for i in 0..trials {
        let clock = Arc::new(MockClock::new(0));
        let v = (a.factory)(clock.clone());
        let t = run_trial_observed(v, clock, config.clone(), &sched, seed.wrapping_add(i as u64));
        fires += t.ticks.iter().filter(|tk| tk.fired && tk.t_secs > warmup).count() as u64;
        secs += dur - warmup;
    }
    if fires == 0 { return 1e9; } // never false-alarms → effectively infinite ARL0
    (secs as f64 / 60.0) / fires as f64
}

fn react_p50_min(c: Cfg, spm: f32, delta: i32, trials: usize, seed: u64) -> f64 {
    let (cfg, sched) = Scenario::SettledStep { settle_minutes: 60, delta_pct: delta }.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..cfg };
    let event = 60 * 60u64;
    let a = spec(c);
    let mut tr = Vec::with_capacity(trials);
    for i in 0..trials {
        let clock = Arc::new(MockClock::new(0));
        let v = (a.factory)(clock.clone());
        tr.push(run_trial_observed(v, clock, config.clone(), &sched, seed.wrapping_add(i as u64)));
    }
    reaction_time_distribution(&tr, event, 180 * 60).1.p50().unwrap_or(f64::NAN) / 60.0
}

/// CUSUM floor delay (min) for drop g at spm, interpolated in ln(ARL0) from
/// the simulated cusum-floor.rs table (ARL0 = 60,240,1440 min).
fn floor_min(g: u32, spm: u32, arl0: f64) -> f64 {
    let pts: &[(f64,f64)] = match (g, spm) {
        (10,4)=>&[(60.,21.),(240.,49.),(1440.,105.)],
        (10,6)=>&[(60.,18.),(240.,40.),(1440.,80.)],
        (25,4)=>&[(60.,9.),(240.,17.),(1440.,28.)],
        (25,6)=>&[(60.,7.),(240.,13.),(1440.,20.)],
        (33,4)=>&[(60.,6.),(240.,11.),(1440.,18.)],  // ~interp between 25 and 50
        (33,6)=>&[(60.,5.),(240.,9.),(1440.,14.)],
        _=>&[(60.,5.),(240.,9.),(1440.,14.)],
    };
    let la = arl0.max(1.0).ln();
    // linear in ln(ARL0), clamped to table ends
    if la <= pts[0].0.ln() { return pts[0].1; }
    if la >= pts[pts.len()-1].0.ln() { return pts[pts.len()-1].1; }
    for w in pts.windows(2) {
        let (a0,d0)=w[0]; let (a1,d1)=w[1];
        if la >= a0.ln() && la <= a1.ln() {
            let f = (la - a0.ln())/(a1.ln()-a0.ln());
            return d0 + f*(d1-d0);
        }
    }
    pts[pts.len()-1].1
}

fn main() {
    let trials = 1500usize;
    let configs = [
        ("champion(s0.3)", Cfg{tau:150,sens:0.3,tighten:6.0,eta_max:0.8}),
        ("mid(s0.6)",      Cfg{tau:150,sens:0.6,tighten:6.0,eta_max:0.8}),
        ("mid(s1.0,t360)", Cfg{tau:360,sens:1.0,tighten:8.0,eta_max:0.6}),
        ("corner(s2,t720)",Cfg{tau:720,sens:2.0,tighten:8.0,eta_max:0.6}),
    ];
    for spm in [4u32, 6] {
        println!("\n## spm={spm}: per-config ARL0 (own false-alarm rate) + reaction ratio AT THAT ARL0\n");
        println!("| config | stable ARL0 (min) | −25% react | ratio@ownARL0 | −33% react | ratio@ownARL0 |");
        println!("| --- | --- | --- | --- | --- | --- |");
        for (name, c) in &configs {
            let arl0 = stable_arl0_min(*c, spm as f32, trials, DEFAULT_BASELINE_SEED);
            let r25 = react_p50_min(*c, spm as f32, -25, trials, DEFAULT_BASELINE_SEED ^ 0x25);
            let r33 = react_p50_min(*c, spm as f32, -33, trials, DEFAULT_BASELINE_SEED ^ 0x33);
            let f25 = floor_min(25, spm, arl0);
            let f33 = floor_min(33, spm, arl0);
            let arl_disp = if arl0 >= 1e8 { "∞".to_string() } else { format!("{:.0}", arl0) };
            println!("| {} | {} | {:.0} min | {:.2} | {:.0} min | {:.2} |",
                name, arl_disp, r25, r25/f25, r33, r33/f33);
        }
        println!("  Gate binds iff corner ratio > champion ratio when EACH is scored at its OWN ARL0.");
        println!("  (ratio = config's reaction / CUSUM floor evaluated at that config's own false-alarm budget)");
    }
    println!("\nHeadline number: the champion's stable ARL0 = the false-alarm budget a good config");
    println!("actually runs. If ~60min, ARL0=60 was realistic (gate convention grounded). If loose,");
    println!("the gate's binding was an artifact of a budget nothing operates at.");
}
