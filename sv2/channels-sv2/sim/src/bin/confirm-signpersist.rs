//! High-trial confirmation + weight-robustness for the SignPersist winner.
//!
//! sweep-signpersist-regret (973 algos, 500 trials) put 581 SignPersist
//! configs ahead of the champion (AsymCusum, rank 582). The win is the
//! predicted mechanism: regret_under drops 0.096->0.075 (settle bias
//! shrinks) while regret_over barely moves and detection holds 100%. But
//! the top ~20 are a flat tie (cost 0.1958..0.1984, ~1.3% spread) and the
//! winning discount d=0.06 sits at the LOW EDGE of the swept range -> the
//! optimum may want an even gentler discount.
//!
//! This bin settles both, the way confirm-champions / champion-weights did:
//!   (1) re-run the top cluster at high trials to break the flat-top tie;
//!   (2) add edge probes BELOW d=0.06 (0.02, 0.04) to test the box edge;
//!   (3) re-score the one simulation pass under a w_over:w_under x w_det
//!       weight grid (free) to confirm the winner is weight-robust.
//! Champion (AsymCusum) is the anchor throughout.
//!
//! Usage: `cargo run --release --bin confirm-signpersist`
//! Env: VARDIFF_CS_TRIALS (default 2000), VARDIFF_CS_THREADS,
//!      VARDIFF_SWEEP_SEED, VARDIFF_CS_OUT_DIR (default ".").
//! Writes confirm_signpersist.md. Sweeps weights internally (ignores
//! VARDIFF_W_* overrides).

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

const FLOOR: f64 = 0.05;
const ETA_BASE: f32 = 0.2;
const ETA_MAX: f32 = 0.8;
const ACCEL: f32 = 0.05;

#[derive(Clone)]
struct Components {
    name: String,
    tag: &'static str,
    discount: f64, // for edge classification
    regret_over: f64,
    regret_under: f64,
    effort_up: f64,
    effort_down: f64,
    detection: f64,
}
impl Components {
    fn cost(&self, w_over: f64, w_under: f64, rho: f64, rho_up: f64, rho_down: f64, w_det: f64) -> f64 {
        w_over * self.regret_over
            + w_under * self.regret_under
            + rho * (rho_up * self.effort_up + rho_down * self.effort_down)
            + w_det * (1.0 - self.detection)
    }
}

#[derive(Clone, Copy)]
struct Sp {
    tau: u64,
    sens: f64,
    tighten: f64,
    discount: f64,
    max_discount: f64,
    spm: u32,
}

fn sp_spec(p: Sp) -> AlgorithmSpec {
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

fn sp(spm: u32, discount: f64, max_discount: f64) -> Sp {
    // top-cluster skeleton: tau150, s0.3, t7.
    Sp { tau: 150, sens: 0.3, tighten: 7.0, discount, max_discount, spm }
}

#[allow(clippy::too_many_arguments)]
fn measure(
    algo: &AlgorithmSpec,
    idx: usize,
    tag: &'static str,
    discount: f64,
    share_rates: &[f32],
    scenarios: &[Scenario],
    trials: usize,
    base_seed: u64,
) -> Option<Components> {
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
    Some(Components {
        name: algo.name.clone(),
        tag,
        discount,
        regret_over: ro / n as f64,
        regret_under: ru / n as f64,
        effort_up: eu / n as f64,
        effort_down: ed / n as f64,
        detection: if dn > 0 { ds / dn as f64 } else { 0.0 },
    })
}

fn main() -> std::io::Result<()> {
    let trials: usize = env::var("VARDIFF_CS_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
    let base_seed: u64 = env::var("VARDIFF_SWEEP_SEED")
        .ok()
        .and_then(|s| s.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).or_else(|| s.parse().ok()))
        .unwrap_or(DEFAULT_BASELINE_SEED);
    let n_threads: usize = env::var("VARDIFF_CS_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .max(1);
    let out_dir = env::var("VARDIFF_CS_OUT_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));

    // (spec, tag, discount). Anchor first.
    let mut entries: Vec<(AlgorithmSpec, &'static str, f64)> = Vec::new();
    entries.push((champion_spec(), "anchor", f64::NAN));

    // (1) top cluster: s0.3/t7/d0.06, spm in {4,5,6}, dm in {0.4,0.6,0.8}.
    for &spm in &[4u32, 5, 6] {
        for &dm in &[0.4f64, 0.6, 0.8] {
            entries.push((sp_spec(sp(spm, 0.06, dm)), "cluster", 0.06));
        }
    }
    // (2) edge probes BELOW d=0.06 on the representative skeleton (spm5).
    for &d in &[0.02f64, 0.04] {
        for &dm in &[0.4f64, 0.6, 0.8] {
            entries.push((sp_spec(sp(5, d, dm)), "probe", d));
        }
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

    let n = entries.len();
    eprintln!(
        "confirm-signpersist: {} algos x {} cells x {} trials = {} runs, {} threads",
        n, share_rates.len() * scenarios.len(), trials,
        n * share_rates.len() * scenarios.len() * trials, n_threads
    );

    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Components>> = Mutex::new(Vec::with_capacity(n));
    std::thread::scope(|scope| {
        for _ in 0..n_threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let (spec, tag, d) = &entries[i];
                if let Some(c) = measure(spec, i, tag, *d, &share_rates, &scenarios, trials, base_seed) {
                    out.lock().unwrap().push(c);
                }
            });
        }
    });
    let comps = out.into_inner().unwrap();
    eprintln!("Simulated {} configs in {:.1}s", comps.len(), started.elapsed().as_secs_f64());

    let (rho, rho_up, rho_down) = (0.5, 3.0, 1.0);

    // (1)+(2): default-weight ranking.
    let mut ranked: Vec<&Components> = comps.iter().collect();
    ranked.sort_by(|a, b| a.cost(3.0, 1.0, rho, rho_up, rho_down, 0.5).partial_cmp(&b.cost(3.0, 1.0, rho, rho_up, rho_down, 0.5)).unwrap());

    let mut md = String::new();
    md.push_str("# SignPersist champion confirmation\n\n");
    md.push_str(&format!("{} configs, {} trials/cell, base_seed {:#x}. Skeleton tau150/s0.3/t7; cluster d=0.06, probes d<0.06.\n\n", n, trials, base_seed));
    md.push_str("## Default-weight ranking (3:1, w_det 0.5)\n\n");
    md.push_str("| rank | tag | config | cost | reg_over | reg_under | det% |\n| --- | --- | --- | --- | --- | --- | --- |\n");
    for (i, c) in ranked.iter().enumerate() {
        md.push_str(&format!(
            "| {} | {} | {} | {:.4} | {:.4} | {:.4} | {:.0}% |\n",
            i + 1, c.tag, c.name, c.cost(3.0, 1.0, rho, rho_up, rho_down, 0.5), c.regret_over, c.regret_under, c.detection * 100.0,
        ));
    }
    md.push('\n');

    // (3) weight sensitivity: winner kind per (w_over, w_det).
    let w_over_axis = [1.0f64, 2.0, 3.0, 4.0, 5.0];
    let w_det_axis = [0.5f64, 1.0, 2.0];
    md.push_str("## Weight sensitivity — winning config per (w_over:w_under, w_det)\n\n");
    md.push_str("Tagged anchor / cluster / probe(d). If probes (d<0.06) keep winning, the discount edge is real and we widen further.\n\n");
    md.push_str("| w_over:w_under \\ w_det |");
    for wd in &w_det_axis {
        md.push_str(&format!(" w_det={} |", wd));
    }
    md.push('\n');
    md.push_str("| --- |");
    for _ in &w_det_axis {
        md.push_str(" --- |");
    }
    md.push('\n');
    for &wo in &w_over_axis {
        md.push_str(&format!("| {:.1}:1 |", wo));
        for &wd in &w_det_axis {
            let win = comps.iter().min_by(|a, b| a.cost(wo, 1.0, rho, rho_up, rho_down, wd).partial_cmp(&b.cost(wo, 1.0, rho, rho_up, rho_down, wd)).unwrap()).unwrap();
            let tag = if win.tag == "probe" { format!("probe d{}", win.discount) } else { win.tag.to_string() };
            md.push_str(&format!(" {} |", tag));
        }
        md.push('\n');
    }
    let out_path = out_dir.join("confirm_signpersist.md");
    fs::write(&out_path, &md)?;
    eprintln!("Wrote {}", out_path.display());

    // Console summary.
    println!("\n## SignPersist confirmation - default-weight top 12 ({} trials)\n", trials);
    println!("| rank | tag | config | cost | reg_over | reg_under | det% |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for (i, c) in ranked.iter().take(12).enumerate() {
        println!(
            "| {} | {} | {} | {:.4} | {:.4} | {:.4} | {:.0}% |",
            i + 1, c.tag, c.name, c.cost(3.0, 1.0, rho, rho_up, rho_down, 0.5), c.regret_over, c.regret_under, c.detection * 100.0,
        );
    }
    let champ = comps.iter().find(|c| c.tag == "anchor").unwrap();
    let champ_rank = ranked.iter().position(|c| c.tag == "anchor").unwrap() + 1;
    let best_probe = ranked.iter().position(|c| c.tag == "probe").map(|i| i + 1);
    let best_cluster = ranked.iter().position(|c| c.tag == "cluster").map(|i| i + 1);
    println!("\nChampion (AsymCusum) at rank {}/{}, cost {:.4}.", champ_rank, n, champ.cost(3.0, 1.0, rho, rho_up, rho_down, 0.5));
    println!(
        "Best cluster rank {:?}, best probe rank {:?} -> {}",
        best_cluster, best_probe,
        match (best_cluster, best_probe) {
            (Some(c), Some(p)) if p < c => "PROBE WINS (d<0.06) -> widen discount lower",
            (Some(_), _) => "cluster holds -> d=0.06 region is the optimum",
            _ => "inconclusive",
        }
    );
    Ok(())
}
