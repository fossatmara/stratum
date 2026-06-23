//! Full corrected-metric funnel re-sweep (reviewer option 2).
//!
//! The original 9,216-config sweep optimized against a metric with two holes:
//! a detection term saturated by window-vs-cadence, and a quadratic-only
//! effort term that under-charges frequent firing. Correcting both demoted
//! the champion (s0.3) to 7th in a sensitivity slice — but that slice froze
//! tau/tighten/discount/accel at values CO-TUNED against the broken metric.
//! A short 150s window plus twitchy firing scored well partly because firing
//! was cheap and detection free; close both holes and the pressure for a
//! short window is gone, so the corrected optimum may want a LONGER tau with
//! moderate sensitivity — invisible to a one-axis sweep. This frees all axes.
//!
//! Scoring (GRID-WIDE, every cell — not post-hoc on finalists):
//!   cost = 3·reg_over + 1·reg_under + ρ·(quad_eff_dir + λ·lin_eff_dir)
//! over the production regime (spm 4,6). Detection is OUT of the scalar
//! (floor-flat at 4-6 spm, Theorem 2); reported separately as high-r* EXCESS.
//! reg = time-avg |e|; effort flavors are direction-weighted (3:1 up:down).
//! λ swept {0,0.5,1,2}; the winner must be stable across it (not free-tuned).
//! λ is anchored on principle: a tighten of step s loses ≈s of in-flight work
//! (§6(ii)), so Σ|s| is cumulative lost-work-fraction in regret's currency;
//! λ=1 enters it at face value.
//!
//! Family-confined (EWMA/SignPersist/accel): a parameter re-optimization
//! under a corrected objective, NOT an architecture search. The CUSUM floor
//! bounds what's achievable on detection.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptiveSignPersist, Composed, EwmaEstimator,
    SignPersistenceCusumBoundary,
};
use channels_sv2::vardiff::MockClock;
use vardiff_sim::baseline::{Scenario, DEFAULT_BASELINE_SEED};
use vardiff_sim::grid::{AlgorithmSpec, VardiffBox};
use vardiff_sim::trial::{run_trial_observed, TrialConfig};

const FLOOR: f64 = 0.05;
const W_OVER: f64 = 3.0;
const W_UNDER: f64 = 1.0;
const RHO_UP: f64 = 3.0;
const RHO_DOWN: f64 = 1.0;
const RHO: f64 = 0.5;
const LAMBDAS: [f64; 4] = [0.0, 0.5, 1.0, 2.0];

#[derive(Clone, Copy)]
struct P {
    tau: u64,
    sens: f64,
    tighten: f64,
    discount: f64,
    max_discount: f64,
    spm_thresh: u32,
    eta_max: f32,
    accel: f32,
}

fn spec(p: P) -> AlgorithmSpec {
    AlgorithmSpec::new(
        format!(
            "Ewma{}/SP-s{}-t{}-d{}-dm{}-spm{}/Acc-0.2-{}-{}",
            p.tau, p.sens, p.tighten, p.discount, p.max_discount, p.spm_thresh, p.eta_max, p.accel
        ),
        move |clock| {
            VardiffBox(Box::new(Composed::new(
                EwmaEstimator::new(p.tau),
                AdaptiveSignPersist::sign_persist(
                    SignPersistenceCusumBoundary::new(p.sens, FLOOR, p.tighten, p.discount, p.max_discount),
                    p.spm_thresh,
                ),
                AcceleratingPartialRetarget::new(0.2, p.eta_max, p.accel),
                1.0,
                clock,
            )))
        },
    )
}

/// Production-regime components over spm {4,6} × scenario mix, from the
/// trajectory. Returns (reg_over, reg_under, quad_eff, lin_eff), all
/// direction-weighted for effort.
fn components(p: P, trials: usize, seed: u64) -> (f64, f64, f64, f64) {
    let scens = [
        Scenario::Stable,
        Scenario::Step { delta_pct: -50 },
        Scenario::Step { delta_pct: -10 },
        Scenario::Step { delta_pct: 10 },
        Scenario::Step { delta_pct: 50 },
    ];
    let spms = [4.0f32, 6.0];
    let (mut ro, mut ru, mut q, mut l, mut n) = (0.0, 0.0, 0.0, 0.0, 0u32);
    let a = spec(p);
    for &spm in &spms {
        for (si, scen) in scens.iter().enumerate() {
            let (cfg, sched) = scen.build(spm);
            let config = TrialConfig { tick_interval_secs: 60, ..cfg };
            for i in 0..trials {
                let clock = Arc::new(MockClock::new(0));
                let v = (a.factory)(clock.clone());
                let t = run_trial_observed(v, clock, config.clone(), &sched,
                    seed.wrapping_add(((si as u64) << 20) + ((spm as u64) << 32) + i as u64));
                let mut last_t = 0u64;
                let (mut tro, mut tru, mut cov) = (0.0, 0.0, 0.0);
                for tk in &t.ticks {
                    let dt = (tk.t_secs - last_t) as f64; last_t = tk.t_secs;
                    let h_true = sched.at(tk.t_secs.saturating_sub(30)) as f64;
                    let h_est = tk.current_hashrate_before as f64;
                    if dt > 0.0 && h_true > 0.0 && h_est > 0.0 {
                        let e = (h_est / h_true).ln();
                        if e >= 0.0 { tro += dt * e.abs(); } else { tru += dt * e.abs(); }
                        cov += dt;
                    }
                    if tk.fired {
                        if let Some(nh) = tk.new_hashrate {
                            let old = tk.current_hashrate_before as f64;
                            if old > 0.0 && nh as f64 > 0.0 {
                                let dl = (nh as f64 / old).ln();
                                let dir = if dl >= 0.0 { RHO_UP } else { RHO_DOWN };
                                q += dir * dl * dl;
                                l += dir * dl.abs();
                            }
                        }
                    }
                }
                if cov > 0.0 { ro += tro / cov; ru += tru / cov; }
                n += 1;
            }
        }
    }
    let nf = n.max(1) as f64;
    (ro / nf, ru / nf, q / nf, l / nf)
}

/// Slow-decline safety gate (the §9 gate, applied DURING the search instead
/// of after). Runs a sustained 10%/hr decline at the sparsest production
/// rate (6 spm, matured 60-min counter), measures the SETTLED error after a
/// 120-min recovery window. A "barely-fire" corner config lags the decline
/// into over-difficulty and settles positive (the +69% classic failure);
/// a responsive config tracks down and settles safe-side (≤ ~+2%). Returns
/// settled e% (median over trials). Gate passes if settled_e ≤ GATE_PCT.
fn decline_settled_e_pct(p: P, trials: usize, seed: u64) -> f64 {
    let spm = 6.0f32;
    let mature = 60u64; // min
    let rate = 0.10f32 / 60.0; // 10%/hr in fraction/min
    let decline_min = 300u64; // reach ~50% over 5h-ish (capped by target)
    let target_drop = 0.50f32;
    let observe = 120u64;
    let mut phases = vec![vardiff_sim::baseline::Phase::Hold { secs: mature * 60, h: vardiff_sim::baseline::TRUE_HASHRATE }];
    let mut dm = 0u64;
    for m in 0..decline_min {
        let frac = (rate * (m as f32 + 1.0)).min(target_drop);
        phases.push(vardiff_sim::baseline::Phase::Hold { secs: 60, h: vardiff_sim::baseline::TRUE_HASHRATE * (1.0 - frac) });
        dm = m + 1;
        if frac >= target_drop { break; }
    }
    let floor_h = vardiff_sim::baseline::TRUE_HASHRATE * (1.0 - (rate * dm as f32).min(target_drop));
    phases.push(vardiff_sim::baseline::Phase::Hold { secs: observe * 60, h: floor_h });
    let scen = Scenario::Custom { name: "gate_decline".into(), phases, initial_estimate: None };
    let (cfg, sched) = scen.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..cfg };
    let d_end = (mature + dm) * 60;
    let trial_end = d_end + observe * 60;
    let a = spec(p);
    let mut settled: Vec<f64> = Vec::with_capacity(trials);
    for i in 0..trials {
        let clock = Arc::new(MockClock::new(0));
        let v = (a.factory)(clock.clone());
        let t = run_trial_observed(v, clock, config.clone(), &sched, seed.wrapping_add(i as u64));
        let mut se = 0.0f64;
        for tk in &t.ticks {
            if tk.t_secs > d_end && tk.t_secs <= trial_end {
                let h_true = sched.at(tk.t_secs.saturating_sub(30)) as f64;
                se = (tk.current_hashrate_before as f64 / h_true).ln() * 100.0;
            }
        }
        settled.push(se);
    }
    settled.sort_by(|a, b| a.partial_cmp(b).unwrap());
    settled[settled.len() / 2]
}

/// Gate threshold: settled over-difficulty after a 10%/hr decline must not
/// exceed this. +2% is the band the responsive champion family sits in;
/// the barely-fire corner runs to +10..+69% (lagging). This is a stated
/// operational requirement (safe-side after a decline), not a tuned weight.
const GATE_PCT: f64 = 2.0;

fn cost(c: (f64, f64, f64, f64), lambda: f64) -> f64 {
    W_OVER * c.0 + W_UNDER * c.1 + RHO * (c.2 + lambda * c.3)
}

fn main() {
    let trials: usize = std::env::var("VARDIFF_SC_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    let base_seed = DEFAULT_BASELINE_SEED;

    // τ-edge probe (default): narrow to the corrected winner's neighborhood
    // and extend tau PAST the 360 grid edge to locate the interior optimum.
    // VARDIFF_SC_TAUEDGE=0 restores the full multi-axis grid.
    let tau_edge = std::env::var("VARDIFF_SC_TAUEDGE").map(|v| v != "0").unwrap_or(true);
    let (taus, senss, tightens, discounts, max_discounts, spm_threshs, eta_maxes, accels): (
        Vec<u64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<u32>, Vec<f32>, Vec<f32>,
    ) = if tau_edge {
        (vec![240, 360, 480, 600, 720], vec![1.0, 1.5, 2.0], vec![8.0], vec![0.06],
         vec![0.6], vec![6], vec![0.6], vec![0.05])
    } else {
        (vec![120, 150, 240, 360], vec![0.3, 0.6, 1.0, 1.5], vec![4.0, 6.0, 8.0], vec![0.06, 0.12],
         vec![0.6], vec![6], vec![0.6, 0.8], vec![0.05, 0.1])
    };

    let mut grid: Vec<P> = Vec::new();
    for &tau in &taus { for &sens in &senss { for &tighten in &tightens {
        for &discount in &discounts { for &max_discount in &max_discounts {
            for &spm_thresh in &spm_threshs { for &eta_max in &eta_maxes { for &accel in &accels {
                grid.push(P { tau, sens, tighten, discount, max_discount, spm_thresh, eta_max, accel });
            }}}
        }}
    }}}
    eprintln!("sweep-corrected: {} configs × (2 spm × 5 scen) × {} trials, λ-sweep {:?}", grid.len(), trials, LAMBDAS);

    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    // Each entry: (config, components, decline_settled_e_pct). The gate is
    // applied DURING the search (the reviewer's key point): the corner is
    // excluded by the §9 safety gate we already justified, not by a new
    // weighted term. Gate trials are fewer (the median settled-e is robust).
    let gate_trials = (trials / 2).max(100);
    let out: Mutex<Vec<(P, (f64, f64, f64, f64), f64)>> = Mutex::new(Vec::with_capacity(grid.len()));
    let n_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..n_threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= grid.len() { break; }
                let c = components(grid[i], trials, base_seed);
                let gate = decline_settled_e_pct(grid[i], gate_trials, base_seed ^ 0xDEC11E);
                out.lock().unwrap().push((grid[i], c, gate));
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if d % 64 == 0 { eprintln!("  {}/{} ({:.0}s)", d, grid.len(), started.elapsed().as_secs_f64()); }
            });
        }
    });
    let results = out.into_inner().unwrap();
    eprintln!("done {} in {:.0}s", results.len(), started.elapsed().as_secs_f64());

    // THE KEY TEST: does the slow-decline safety gate (applied during the
    // search) close the degenerate 'barely-fire' corner on its own — with NO
    // new weighted term? Partition into gate-pass / gate-fail and check
    // whether the gate-passing frontier has an INTERIOR optimum.
    let pass: Vec<&(P,(f64,f64,f64,f64),f64)> = results.iter().filter(|(_,_,g)| *g <= GATE_PCT).collect();
    let fail = results.len() - pass.len();
    println!("\n## Slow-decline gate (settled e ≤ {:+.0}% after 10%/hr decline): {} pass, {} fail of {}.",
        GATE_PCT, pass.len(), fail, results.len());

    for &lambda in &LAMBDAS {
        // Unconstrained winner (the corner) vs gate-constrained winner.
        let unc = results.iter().min_by(|a,b| cost(a.1,lambda).partial_cmp(&cost(b.1,lambda)).unwrap()).unwrap();
        let con = pass.iter().min_by(|a,b| cost(a.1,lambda).partial_cmp(&cost(b.1,lambda)).unwrap());
        println!("\n### λ={lambda}");
        println!("  unconstrained best: Ewma{} s{} t{} (cost {:.4}, gate {:+.1}%)  {}",
            unc.0.tau, unc.0.sens, unc.0.tighten, cost(unc.1,lambda), unc.2,
            if unc.2 > GATE_PCT { "← FAILS gate (corner)" } else { "(passes gate)" });
        if let Some(c) = con {
            // is the gate-constrained winner INTERIOR in tau? (not at grid edge)
            let taus_sorted = [240u64,360,480,600,720];
            let at_edge = c.0.tau == *taus_sorted.last().unwrap();
            println!("  gate-constrained best: Ewma{} s{} t{} (cost {:.4}, gate {:+.1}%)  {}",
                c.0.tau, c.0.sens, c.0.tighten, cost(c.1,lambda), c.2,
                if at_edge { "← still at tau edge (gate did NOT close corner)" } else { "← INTERIOR (gate closed the corner)" });
        }
        // top 6 gate-passing configs
        let mut p2 = pass.clone();
        p2.sort_by(|a,b| cost(a.1,lambda).partial_cmp(&cost(b.1,lambda)).unwrap());
        println!("  gate-passing top 6:");
        for (i,(p,c,g)) in p2.iter().take(6).enumerate() {
            println!("    {} | Ewma{} s{} t{} d{} eta{} acc{} | cost {:.4} | gate {:+.1}%",
                i+1, p.tau, p.sens, p.tighten, p.discount, p.eta_max, p.accel, cost(*c,lambda), g);
        }
    }
}
