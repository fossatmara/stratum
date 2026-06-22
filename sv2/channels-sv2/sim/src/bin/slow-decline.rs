//! Slow-decline safety test (see docs/SLOW_DECLINE_TEST.md).
//!
//! A sustained hashrate decline drives e=ln(Ĥ/H) POSITIVE (over-difficulty),
//! the costly §6 side. Shares arrive slow; the correct response is to EASE.
//! The death-spiral risk is self-reinforcing starvation: over-difficulty →
//! fewer shares → less evidence → slower ease → more over-difficulty. The
//! champion's AdaptiveSignPersist switches to the conservative low-SPM
//! PoissonCI guard below spm_threshold=6, which could freeze the ease
//! exactly when the decline drags effective rate down — the hypothesis.
//!
//! Per (rate × spm × algo) cell we report, over the decline phase:
//!   - tighten_fires : count of s>0 fires  (HARD GATE: must be 0)
//!   - max_e         : worst over-difficulty reached (runaway if unbounded)
//!   - end_e         : e at end of decline (did it turn over or keep climbing)
//!   - mean_e        : time-avg over-difficulty = regret_over (graded lag)
//!   - eased         : count of s<0 fires (the corrective action)
//!
//! Usage: cargo run --release --bin slow-decline
//! Env: VARDIFF_SD_TRIALS (default 300), VARDIFF_SD_THREADS, VARDIFF_SWEEP_SEED.

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptivePoissonCusum, AsymmetricCusumBoundary, Composed,
    EwmaEstimator, PoissonCI,
};
use channels_sv2::vardiff::MockClock;
use vardiff_sim::baseline::{Phase, Scenario, DEFAULT_BASELINE_SEED, TRUE_HASHRATE};
use vardiff_sim::grid::{AlgorithmSpec, VardiffBox};
use vardiff_sim::trial::{run_trial_observed, TrialConfig};

const TICK: u64 = 60;
const MATURE_MIN: u64 = 60; // counter matured on-target before the decline
const OBSERVE_MIN: u64 = 30; // settle window after the decline floor
const MAX_DECLINE_MIN: u64 = 240; // cap a slow decline at 4h
const TARGET_DROP: f32 = 0.50; // decline until 50% lost (or the 4h cap)

/// The interim champion (AsymCusum boundary, no sign-persistence) — the
/// control that isolates whether the sign-persistence discount specifically
/// helps or hurts on a decline.
fn interim() -> AlgorithmSpec {
    AlgorithmSpec::new("interim(AsymCusum)", |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(150),
            AdaptivePoissonCusum::with_params(
                PoissonCI::default_parametric(),
                AsymmetricCusumBoundary::new(0.2, 0.05, 6.0),
                5,
            ),
            AcceleratingPartialRetarget::new(0.2, 0.8, 0.05),
            1.0,
            clock,
        )))
    })
}

/// A sustained decline scenario as a Custom phase list: mature on-target,
/// then drop at `rate_pct_per_hr` in fine 1-min Hold steps until TARGET_DROP
/// (capped at MAX_DECLINE_MIN), then hold at the floor. Returns the scenario
/// and the decline window [start_secs, end_secs] for the readout.
fn decline_scenario(rate_pct_per_hr: f32) -> (Scenario, u64, u64) {
    let rate = rate_pct_per_hr / 100.0; // fraction/hr
    // minutes to reach TARGET_DROP at this rate, capped.
    let decline_min = ((TARGET_DROP / rate) * 60.0).min(MAX_DECLINE_MIN as f32) as u64;
    let mut phases = vec![Phase::Hold {
        secs: MATURE_MIN * 60,
        h: TRUE_HASHRATE,
    }];
    // Fine 1-min declining segments.
    for m in 0..decline_min {
        let frac = (rate / 60.0) * (m as f32 + 1.0); // fraction lost by minute m+1
        let h = TRUE_HASHRATE * (1.0 - frac).max(1.0 - TARGET_DROP);
        phases.push(Phase::Hold { secs: 60, h });
    }
    let floor = TRUE_HASHRATE * (1.0 - (rate / 60.0 * decline_min as f32).min(TARGET_DROP));
    phases.push(Phase::Hold {
        secs: OBSERVE_MIN * 60,
        h: floor,
    });
    let start = MATURE_MIN * 60;
    let end = start + decline_min * 60;
    (
        Scenario::Custom {
            name: format!("decline_{}pph", rate_pct_per_hr as u32),
            phases,
            initial_estimate: None, // start aligned with truth
        },
        start,
        end,
    )
}

struct Row {
    rate: f32,
    spm: f32,
    algo: String,
    tighten_fires: f64, // mean per trial
    eased: f64,
    max_e_pct: f64,  // worst over-difficulty %, median over trials
    end_e_pct: f64,  // e at decline end, median
    mean_e_pct: f64, // time-avg over-difficulty during decline
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let trials: usize = env::var("VARDIFF_SD_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let base_seed: u64 = env::var("VARDIFF_SWEEP_SEED")
        .ok()
        .and_then(|s| s.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).or_else(|| s.parse().ok()))
        .unwrap_or(DEFAULT_BASELINE_SEED);
    let n_threads: usize = env::var("VARDIFF_SD_THREADS")
        .ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)).max(1);

    let rates = [2.0f32, 5.0, 10.0, 20.0, 40.0];
    let spms = [6.0f32, 8.0, 12.0, 20.0, 30.0];
    let algos: Vec<(String, Box<dyn Fn() -> AlgorithmSpec + Send + Sync>)> = vec![
        ("champion".into(), Box::new(AlgorithmSpec::champion)),
        ("interim".into(), Box::new(interim)),
        ("classic".into(), Box::new(AlgorithmSpec::classic_composed)),
    ];

    // Flatten the work into (rate, spm, algo_idx) jobs.
    let jobs: Vec<(f32, f32, usize)> = rates
        .iter()
        .flat_map(|&r| spms.iter().flat_map(move |&s| (0..3).map(move |a| (r, s, a))))
        .collect();
    eprintln!("slow-decline: {} cells × {} trials, {} threads", jobs.len(), trials, n_threads);

    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Row>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..n_threads {
            scope.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                if j >= jobs.len() {
                    break;
                }
                let (rate, spm, ai) = jobs[j];
                let (scen, d_start, d_end) = decline_scenario(rate);
                let (config_proto, schedule) = scen.build(spm);
                let config = TrialConfig { tick_interval_secs: TICK, ..config_proto };
                // wrong_dir = tighten (s>0) WHILE still over-difficulty (e>0):
                //   the literal death-spiral step. A tighten while e<0 is a
                //   correct staircase-overshoot correction and is NOT counted.
                let (mut wrong_dir, mut eased) = (0.0f64, 0.0f64);
                let (mut maxe, mut ende, mut meane) = (vec![], vec![], vec![]);
                for i in 0..trials {
                    let clock = Arc::new(MockClock::new(0));
                    let v = (algos[ai].1)().factory.clone()(clock.clone());
                    let t = run_trial_observed(v, clock, config.clone(), &schedule, base_seed.wrapping_add(i as u64));
                    let (mut mx, mut last, mut sum, mut n) = (f64::MIN, 0.0f64, 0.0f64, 0u32);
                    for tk in &t.ticks {
                        if tk.t_secs <= d_start || tk.t_secs > d_end {
                            continue;
                        }
                        let h_true = schedule.at(tk.t_secs.saturating_sub(TICK / 2)) as f64;
                        let e = (tk.current_hashrate_before as f64 / h_true).ln();
                        mx = mx.max(e);
                        last = e;
                        sum += e.max(0.0); // over-difficulty contribution
                        n += 1;
                        if tk.fired {
                            if let Some(nh) = tk.new_hashrate {
                                let s = (nh as f64 / tk.current_hashrate_before as f64).ln();
                                if s < 0.0 {
                                    eased += 1.0;
                                } else if e > 0.02 {
                                    // tighten while genuinely over-difficulty (>2%,
                                    // i.e. not the noisy e≈0 staircase crossing) =
                                    // the death-spiral step.
                                    wrong_dir += 1.0;
                                }
                            }
                        }
                    }
                    if n > 0 {
                        maxe.push(mx * 100.0);
                        ende.push(last * 100.0);
                        meane.push(sum / n as f64 * 100.0);
                    }
                }
                let tn = trials as f64;
                out.lock().unwrap().push(Row {
                    rate, spm, algo: algos[ai].0.clone(),
                    tighten_fires: wrong_dir / tn, eased: eased / tn,
                    max_e_pct: median(maxe), end_e_pct: median(ende), mean_e_pct: median(meane),
                });
            });
        }
    });

    let mut rows = out.into_inner().unwrap();
    rows.sort_by(|a, b| (a.rate, a.spm as u32, a.algo.clone()).partial_cmp(&(b.rate, b.spm as u32, b.algo.clone())).unwrap());

    println!("\n## Slow-decline safety ({} trials/cell). e>0 = over-difficulty (costly).", trials);
    println!("Hard gate = tighten fired WHILE over-difficulty (the death-spiral step). max_e/end_e track runaway.\n");
    println!("| rate %/hr | spm | algo | wrong-dir fires | eases | max e% | end e% | mean e% (regret_over) |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    let mut fails = 0;
    for r in &rows {
        let flag = if r.tighten_fires > 0.0 { " ⚠TIGHTEN" } else { "" };
        if r.tighten_fires > 0.0 { fails += 1; }
        println!(
            "| {} | {} | {} | {:.2}{} | {:.1} | {:+.1} | {:+.1} | {:.2} |",
            r.rate as u32, r.spm as u32, r.algo, r.tighten_fires, flag, r.eased,
            r.max_e_pct, r.end_e_pct, r.mean_e_pct
        );
    }
    println!("\nHard-gate (tighten fires during decline): {} cell(s) flagged.", fails);
    println!("Champion summary (worst over rates/spm):");
    for algo in ["champion", "interim", "classic"] {
        let sub: Vec<&Row> = rows.iter().filter(|r| r.algo == algo).collect();
        let worst_max = sub.iter().map(|r| r.max_e_pct).fold(f64::MIN, f64::max);
        let worst_mean = sub.iter().map(|r| r.mean_e_pct).fold(f64::MIN, f64::max);
        let tot_tighten: f64 = sub.iter().map(|r| r.tighten_fires).sum();
        println!("  {}: worst max_e {:+.1}%, worst mean_e {:.2}%, total tighten {:.2}", algo, worst_max, worst_mean, tot_tighten);
    }
}
