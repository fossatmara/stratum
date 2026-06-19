//! Confirm the cold-start warm-up wrapper: does WarmupBoundary cut ramp-up
//! WITHOUT regressing the steady-state regret/effort cost, and what
//! converge_band is best?
//!
//! Trajectory spike showed c+warmup halves ramp-up (15->7 min) with settle
//! gap and detection essentially unchanged. But (a) the spike warmed the
//! bare SignPersist boundary (no low-SPM guard) and (b) converge_band=0.15
//! was hand-picked. Here we wrap the boundary CORRECTLY as
//! WarmupBoundary<AdaptiveSignPersist> (so sub-6-SPM miners keep the
//! PoissonCI guard after warm-up), sweep converge_band, and score on the
//! full §10 grid against the current SignPersist champion anchor.
//!
//! Cold start is EXCLUDED from the §10 cost (one-time, washes out), so the
//! steady-state cost SHOULD be ~identical to the champion's — this run's
//! job is to PROVE warm-up doesn't quietly regress it, and to pick the
//! band. Ramp-up itself is read from the trajectory bin, not here.
//!
//! Usage: `cargo run --release --bin confirm-warmup`
//! Env: VARDIFF_CW2_TRIALS (default 1000), VARDIFF_CW2_THREADS,
//!      VARDIFF_SWEEP_SEED, VARDIFF_CW2_OUT_DIR (default ".").

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptiveSignPersist, Composed, EwmaEstimator,
    SignPersistenceCusumBoundary, WarmupBoundary,
};
use vardiff_sim::baseline::{Cell, Scenario, DEFAULT_BASELINE_SEED};
use vardiff_sim::grid::{run_cell_with_algorithm, AlgorithmSpec, VardiffBox};

const FLOOR: f64 = 0.05;
const ETA_BASE: f32 = 0.2;
const ETA_MAX: f32 = 0.8;
const ACCEL: f32 = 0.05;
// Champion SignPersist boundary params.
const SENS: f64 = 0.3;
const TIGHTEN: f64 = 6.0;
const DISCOUNT: f64 = 0.06;
const MAX_DISCOUNT: f64 = 0.6;
const SPM_SWITCH: u32 = 6;
const TAU: u64 = 150;

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

fn champion_boundary() -> AdaptiveSignPersist {
    AdaptiveSignPersist::sign_persist(
        SignPersistenceCusumBoundary::new(SENS, FLOOR, TIGHTEN, DISCOUNT, MAX_DISCOUNT),
        SPM_SWITCH,
    )
}

/// The current champion (no warm-up) — anchor.
fn champion_spec() -> AlgorithmSpec {
    let name = vardiff_sim::naming::triple_name(
        &EwmaEstimator::new(TAU),
        &champion_boundary(),
        &AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
    );
    AlgorithmSpec::new(name, |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(TAU),
            champion_boundary(),
            AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
            1.0,
            clock,
        )))
    })
}

/// Champion + warm-up at a given converge_band. WarmupBoundary wraps the
/// FULL AdaptiveSignPersist, so post-warmup the low-SPM PoissonCI guard is
/// preserved.
fn warmup_spec(band: f64) -> AlgorithmSpec {
    let name = vardiff_sim::naming::triple_name(
        &EwmaEstimator::new(TAU),
        &WarmupBoundary::new(champion_boundary(), band),
        &AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
    );
    AlgorithmSpec::new(name, move |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(TAU),
            WarmupBoundary::new(champion_boundary(), band),
            AcceleratingPartialRetarget::new(ETA_BASE, ETA_MAX, ACCEL),
            1.0,
            clock,
        )))
    })
}

#[allow(clippy::too_many_arguments)]
fn profile_for(
    algo: &AlgorithmSpec,
    idx: usize,
    share_rates: &[f32],
    scenarios: &[Scenario],
    trials: usize,
    base_seed: u64,
    w: &Weights,
    is_anchor: bool,
) -> Option<Profile> {
    let (n_spm, n_scen) = (share_rates.len(), scenarios.len());
    let (mut ro, mut ru, mut eu, mut ed) = (0.0, 0.0, 0.0, 0.0);
    let mut n = 0u32;
    let (mut ds, mut dn) = (0.0, 0u32);
    for (si, &spm) in share_rates.iter().enumerate() {
        for (ci, scen) in scenarios.iter().enumerate() {
            let cell = Cell { shares_per_minute: spm, scenario: scen.clone() };
            let cell_index = (idx * n_spm * n_scen + si * n_scen + ci) as u64;
            let r = run_cell_with_algorithm(algo, &cell, trials, base_seed, cell_index);
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
    let trials: usize = env::var("VARDIFF_CW2_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let base_seed: u64 = env::var("VARDIFF_SWEEP_SEED")
        .ok()
        .and_then(|s| s.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).or_else(|| s.parse().ok()))
        .unwrap_or(DEFAULT_BASELINE_SEED);
    let n_threads: usize = env::var("VARDIFF_CW2_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .max(1);
    let out_dir = env::var("VARDIFF_CW2_OUT_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    let w = Weights::from_env();

    let bands = [0.08f64, 0.12, 0.15, 0.2, 0.3];
    let mut specs: Vec<AlgorithmSpec> = vec![champion_spec()];
    let n_anchors = specs.len();
    for &b in &bands {
        specs.push(warmup_spec(b));
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

    let n = specs.len();
    eprintln!(
        "confirm-warmup: {} algos x {} cells x {} trials = {} runs, {} threads",
        n, share_rates.len() * scenarios.len(), trials,
        n * share_rates.len() * scenarios.len() * trials, n_threads
    );

    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Profile>> = Mutex::new(Vec::with_capacity(n));
    std::thread::scope(|scope| {
        for _ in 0..n_threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                if let Some(p) = profile_for(&specs[i], i, &share_rates, &scenarios, trials, base_seed, &w, i < n_anchors) {
                    out.lock().unwrap().push(p);
                }
            });
        }
    });
    let mut profiles = out.into_inner().unwrap();
    eprintln!("Done in {:.1}s", started.elapsed().as_secs_f64());
    profiles.sort_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap());

    let row = |i: usize, p: &Profile| {
        format!(
            "| {} | {}{} | {:.4} | {:.4} | {:.4} | {:.4} | {:.4} | {:.0}% |\n",
            i + 1, p.name, if p.is_anchor { " <-champion(no warmup)" } else { "" },
            p.cost, p.regret_over, p.regret_under, p.effort_up, p.effort_down, p.detection * 100.0,
        )
    };

    let mut md = String::new();
    md.push_str("# Warm-up confirmation (steady-state regret/effort)\n\n");
    md.push_str(&format!(
        "{} algos, {} trials/cell, base_seed {:#x}. WarmupBoundary<AdaptiveSignPersist>, converge_band swept. Cold start is excluded from the cost, so warm-up should NOT change steady-state cost; this proves it.\n\n",
        n, trials, base_seed
    ));
    md.push_str("| rank | algorithm | cost | reg_over | reg_under | eff_up | eff_down | det% |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for (i, p) in profiles.iter().enumerate() {
        md.push_str(&row(i, p));
    }
    let out_path = out_dir.join("confirm_warmup.md");
    fs::write(&out_path, &md)?;
    eprintln!("Wrote {}", out_path.display());

    println!("\n## Warm-up confirmation ({} trials) - steady-state cost vs champion\n", trials);
    println!("| rank | algorithm | cost | reg_over | reg_under | eff_up | eff_down | det% |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for (i, p) in profiles.iter().enumerate() {
        print!("{}", row(i, p));
    }
    let champ = profiles.iter().find(|p| p.is_anchor).unwrap();
    let spread = profiles.iter().map(|p| (p.cost - champ.cost).abs()).fold(0.0f64, f64::max);
    println!(
        "\nMax |cost - champion| across all warm-up bands: {:.4} ({:.2}%). If tiny, warm-up is steady-state-neutral as intended.",
        spread, 100.0 * spread / champ.cost
    );
    Ok(())
}
