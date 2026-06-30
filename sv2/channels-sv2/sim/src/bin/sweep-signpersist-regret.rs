//! Regret/effort sweep over the SignPersistence boundary family — the
//! follow-up to the trajectory spike where `c+signpersist` beat the
//! champion on ramp-up (33->11 min), settle gap (-9.8%->-5.8%), AND
//! detection (11->7 min), all from one mechanism. (The §10 regret/effort
//! successor to the older maximin `sweep-signpersist` bin.)
//!
//! Sign-persistence discounts the fire threshold on consecutive same-sign
//! residuals (applied AFTER the tighten multiplier), so a PERSISTENT
//! under-difficulty (cold start, settle bias) progressively relaxes the
//! tighten brake and fires frequent small corrections, while a one-off
//! spike keeps full death-spiral reluctance. This sweep asks: tuned
//! properly, how much of the remaining settle gap (-5.8% vs the -0.7%
//! oracle floor) can it close WITHOUT losing detection or gentleness?
//!
//! Cost is IDENTICAL to sweep-regret-big so rankings compare directly; the
//! current champion (AsymCusum boundary) is the anchor.
//!
//! Usage: `cargo run --release --bin sweep-signpersist-regret`
//! Env: VARDIFF_SP_TRIALS (default 300), VARDIFF_SP_THREADS,
//!      VARDIFF_SWEEP_SEED, VARDIFF_W_*/RHO_* weights,
//!      VARDIFF_SP_OUT_DIR (default "."), writes `signpersist_regret_sweep.md`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptivePoissonCusum, AdaptiveSignPersist, AsymmetricCusumBoundary,
    Composed, EwmaEstimator, PoissonCI, SignPersistenceCusumBoundary,
};
use vardiff_sim::baseline::{Cell, Scenario, DEFAULT_BASELINE_SEED};
use vardiff_sim::grid::{run_cell_with_algorithm, AlgorithmSpec, VardiffBox};

struct Weights {
    w_over: f64,
    w_under: f64,
    rho_up: f64,
    rho_down: f64,
    rho: f64,
    w_det: f64,
}
impl Weights {
    fn from_env() -> Self {
        let g = |k: &str, d: f64| env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
        Weights {
            w_over: g("VARDIFF_W_OVER", 3.0),
            w_under: g("VARDIFF_W_UNDER", 1.0),
            rho_up: g("VARDIFF_RHO_UP", 3.0),
            rho_down: g("VARDIFF_RHO_DOWN", 1.0),
            rho: g("VARDIFF_RHO", 0.5),
            w_det: g("VARDIFF_W_DET", 0.5),
        }
    }
}

struct Profile {
    name: String,
    regret_over: f64,
    regret_under: f64,
    effort_up: f64,
    effort_down: f64,
    detection: f64,
    cost: f64,
    is_anchor: bool,
}

#[derive(Clone, Copy)]
struct Params {
    tau: u64,
    sens: f64,
    tighten: f64,
    discount: f64,
    max_discount: f64,
    spm: u32,
}

const FLOOR: f64 = 0.05;
const ETA_BASE: f32 = 0.2;
const ETA_MAX: f32 = 0.8;
const ACCEL: f32 = 0.05;

fn signpersist_spec(p: Params) -> AlgorithmSpec {
    let name = vardiff_sim::naming::triple_name(
        &EwmaEstimator::new(p.tau),
        &AdaptiveSignPersist::sign_persist(
            SignPersistenceCusumBoundary::new(p.sens, FLOOR, p.tighten, p.discount, p.max_discount),
            p.spm,
        ),
        &AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
    );
    AlgorithmSpec::new(name, move |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(p.tau),
            AdaptiveSignPersist::sign_persist(
                SignPersistenceCusumBoundary::new(p.sens, FLOOR, p.tighten, p.discount, p.max_discount),
                p.spm,
            ),
            AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
            1.0,
            clock,
        )))
    })
}

fn champion_spec() -> AlgorithmSpec {
    let name = vardiff_sim::naming::triple_name(
        &EwmaEstimator::new(150),
        &AdaptivePoissonCusum::with_params(
            PoissonCI::default_parametric(),
            AsymmetricCusumBoundary::new(0.2, FLOOR, 6.0),
            5,
        ),
        &AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
    );
    AlgorithmSpec::new(name, |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(150),
            AdaptivePoissonCusum::with_params(
                PoissonCI::default_parametric(),
                AsymmetricCusumBoundary::new(0.2, FLOOR, 6.0),
                5,
            ),
            AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
            1.0,
            clock,
        )))
    })
}

#[allow(clippy::too_many_arguments)]
fn profile_for(
    algo: &AlgorithmSpec,
    algo_idx: usize,
    share_rates: &[f32],
    scenarios: &[Scenario],
    trial_count: usize,
    base_seed: u64,
    w: &Weights,
    is_anchor: bool,
) -> Option<Profile> {
    let (n_spm, n_scen) = (share_rates.len(), scenarios.len());
    let (mut ro, mut ru, mut eu, mut ed) = (0.0, 0.0, 0.0, 0.0);
    let mut n = 0u32;
    let (mut ds, mut dn) = (0.0, 0u32);
    for (spm_idx, &spm) in share_rates.iter().enumerate() {
        for (scen_idx, scen) in scenarios.iter().enumerate() {
            let cell = Cell { shares_per_minute: spm, scenario: scen.clone() };
            let cell_index = (algo_idx * n_spm * n_scen + spm_idx * n_scen + scen_idx) as u64;
            let r = run_cell_with_algorithm(algo, &cell, trial_count, base_seed, cell_index);
            if matches!(scen, Scenario::Stable | Scenario::Step { delta_pct: -50 | -10 | 10 | 50 }) {
                ro += r.get("regret_over").unwrap_or(0.0);
                ru += r.get("regret_under").unwrap_or(0.0);
                eu += r.get("effort_up").unwrap_or(0.0);
                ed += r.get("effort_down").unwrap_or(0.0);
                n += 1;
            }
            if matches!(scen, Scenario::SettledStep { settle_minutes: 60, delta_pct: -10 }) {
                if let Some(rate) = r.get("settled_reaction_rate") {
                    ds += rate;
                    dn += 1;
                }
            }
        }
    }
    if n == 0 {
        return None;
    }
    let (regret_over, regret_under, effort_up, effort_down) =
        (ro / n as f64, ru / n as f64, eu / n as f64, ed / n as f64);
    let detection = if dn > 0 { ds / dn as f64 } else { 0.0 };
    let cost = w.w_over * regret_over
        + w.w_under * regret_under
        + w.rho * (w.rho_up * effort_up + w.rho_down * effort_down)
        + w.w_det * (1.0 - detection);
    Some(Profile {
        name: algo.name.clone(),
        regret_over,
        regret_under,
        effort_up,
        effort_down,
        detection,
        cost,
        is_anchor,
    })
}

fn main() -> std::io::Result<()> {
    let trial_count: usize = env::var("VARDIFF_SP_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let base_seed: u64 = env::var("VARDIFF_SWEEP_SEED")
        .ok()
        .and_then(|s| s.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).or_else(|| s.parse().ok()))
        .unwrap_or(DEFAULT_BASELINE_SEED);
    let n_threads: usize = env::var("VARDIFF_SP_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .max(1);
    let out_dir = env::var("VARDIFF_SP_OUT_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    let w = Weights::from_env();

    // Centered on the spike config (s0.2,t6,d0.12,dm0.6,spm5,tau150);
    // widened on the discount axes (the new levers).
    let taus = [120u64, 150, 180];
    let sensitivities = [0.15f64, 0.2, 0.3];
    let tightens = [5.0f64, 6.0, 7.0];
    let discounts = [0.06f64, 0.12, 0.2, 0.3];
    let max_discounts = [0.4f64, 0.6, 0.8];
    let spms = [4u32, 5, 6];

    let mut params: Vec<Params> = Vec::new();
    for &tau in &taus {
        for &sens in &sensitivities {
            for &tighten in &tightens {
                for &discount in &discounts {
                    for &max_discount in &max_discounts {
                        for &spm in &spms {
                            params.push(Params { tau, sens, tighten, discount, max_discount, spm });
                        }
                    }
                }
            }
        }
    }

    let mut specs: Vec<AlgorithmSpec> = vec![champion_spec()];
    let n_anchors = specs.len();
    for p in &params {
        specs.push(signpersist_spec(*p));
    }

    let scenarios = vec![
        Scenario::Stable,
        Scenario::Step { delta_pct: -50 },
        Scenario::Step { delta_pct: -10 },
        Scenario::Step { delta_pct: 10 },
        Scenario::Step { delta_pct: 50 },
        Scenario::SettledStep { settle_minutes: 60, delta_pct: -10 },
    ];
    let share_rates = vec![6.0f32, 8.0, 12.0, 20.0, 30.0];

    let n_algos = specs.len();
    let n_cells = share_rates.len() * scenarios.len();
    eprintln!(
        "SignPersist sweep: {} algos ({} grid + {} anchor) x {} cells x {} trials = {} runs, {} threads",
        n_algos, n_algos - n_anchors, n_anchors, n_cells, trial_count, n_algos * n_cells * trial_count, n_threads
    );

    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let out: Mutex<Vec<Profile>> = Mutex::new(Vec::with_capacity(n_algos));
    std::thread::scope(|scope| {
        for _ in 0..n_threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n_algos {
                    break;
                }
                let is_anchor = i < n_anchors;
                if let Some(p) = profile_for(&specs[i], i, &share_rates, &scenarios, trial_count, base_seed, &w, is_anchor) {
                    out.lock().unwrap().push(p);
                }
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if d % 128 == 0 || d == n_algos {
                    let el = started.elapsed().as_secs_f64();
                    let eta = (n_algos - d) as f64 / (d as f64 / el).max(1e-9);
                    eprintln!("  {}/{} ({:.0}%) | {:.0}s elapsed | ~{:.0}s left", d, n_algos, 100.0 * d as f64 / n_algos as f64, el, eta);
                }
            });
        }
    });

    let mut profiles = out.into_inner().unwrap();
    eprintln!("Done: {} profiles in {:.1}s", profiles.len(), started.elapsed().as_secs_f64());
    profiles.sort_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap());
    let champ_rank = profiles.iter().position(|p| p.is_anchor);

    let row = |i: usize, p: &Profile| {
        format!(
            "| {} | {}{} | **{:.4}** | {:.4} | {:.4} | {:.4} | {:.4} | {:.0}% |\n",
            i + 1, p.name, if p.is_anchor { " <-champion" } else { "" },
            p.cost, p.regret_over, p.regret_under, p.effort_up, p.effort_down, p.detection * 100.0,
        )
    };

    let mut md = String::new();
    md.push_str("# SignPersistence boundary sweep (regret/effort)\n\n");
    md.push_str(&format!(
        "Cost = {:.1}.regret_over + {:.1}.regret_under + {:.2}.({:.1}.effort_up + {:.1}.effort_down) + {:.2}.(1-detection). Lower is better.\n\n",
        w.w_over, w.w_under, w.rho, w.rho_up, w.rho_down, w.w_det
    ));
    md.push_str(&format!(
        "{} algos, {} trials/cell, base_seed {:#x}. Champion (AsymCusum) anchor at rank {}/{}.\n\n",
        n_algos, trial_count, base_seed, champ_rank.map(|r| r + 1).unwrap_or(0), n_algos
    ));
    md.push_str("| rank | algorithm | **cost** | reg_over | reg_under | eff_up | eff_down | det% |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for (i, p) in profiles.iter().enumerate() {
        md.push_str(&row(i, p));
    }
    let out_path = out_dir.join("signpersist_regret_sweep.md");
    fs::write(&out_path, &md)?;
    eprintln!("Wrote {}", out_path.display());

    println!("\n## SignPersist sweep - top 20 + champion anchor\n");
    println!("| rank | algorithm | cost | reg_over | reg_under | eff_up | eff_down | det% |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for (i, p) in profiles.iter().take(20).enumerate() {
        print!("{}", row(i, p));
    }
    if let Some(r) = champ_rank {
        if r >= 20 {
            print!("{}", row(r, &profiles[r]));
        }
        println!("\nChampion (AsymCusum) anchor at rank {}/{}, cost {:.4}.", r + 1, n_algos, profiles[r].cost);
    }
    Ok(())
}
