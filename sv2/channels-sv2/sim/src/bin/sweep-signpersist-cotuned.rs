//! Co-tuned maximin sweep around the SignPersistenceCusumBoundary.
//!
//! The first SignPersist sweep broke the react−10% wall (0.54 → 0.67+) but
//! borrowed the estimator/update settings that were tuned for the DIFFERENT
//! AdaptPC boundary, and convergence emerged as the new weak axis (~0.42).
//!
//! Hypothesis for the convergence sag: cold-start is a *tightening* scenario
//! (miner faster than the too-low initial target → would_tighten), so the
//! inherited tighten_multiplier=3 makes the ramp 3× harder to fire, and the
//! sign discount (cap 0.4–0.8) can't fully offset it. So tighten_multiplier,
//! the update aggressiveness (eta_max/accel), and τ are the untested levers.
//!
//! This sweep co-tunes all three axes around SignPersist to find the Pareto
//! best — does fixing convergence cost the react−10% gain, or is there a
//! setting that holds both? Scored by maximin; full per-axis table emitted.
//!
//! ## Usage
//! ```text
//! VARDIFF_SWEEP_TRIALS=300 cargo run --release --bin sweep-signpersist-cotuned
//! ```
//! Writes `signpersist_cotuned_sweep.md`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use vardiff_sim::baseline::{Scenario, DEFAULT_BASELINE_SEED};
use vardiff_sim::grid::{AlgorithmSpec, Grid, VardiffBox};
use vardiff_sim::metrics::{DerivedMetric, EqualWeightFitness};

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptivePoissonCusum, AsymmetricCusumBoundary, Composed,
    EwmaEstimator, PoissonCI, SignPersistenceCusumBoundary,
};

const AXES: &[(&str, &str)] = &[
    ("reaction_10", "react-10%"),
    ("reaction_50", "react-50%"),
    ("jitter", "jitter"),
    ("step_safety", "step-safe"),
    ("convergence", "conv"),
    ("overshoot", "overshoot"),
];

struct Profile {
    name: String,
    axes: [f64; 6],
    maximin: f64,
    mean: f64,
}

fn main() -> std::io::Result<()> {
    let trial_count: usize = env::var("VARDIFF_SWEEP_TRIALS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let base_seed: u64 = env::var("VARDIFF_SWEEP_SEED")
        .ok()
        .and_then(|s| {
            s.strip_prefix("0x")
                .and_then(|h| u64::from_str_radix(h, 16).ok())
                .or_else(|| s.parse().ok())
        })
        .unwrap_or(DEFAULT_BASELINE_SEED);
    let out_dir = env::var("VARDIFF_SWEEP_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    // Estimator axis: τ trades reaction/convergence speed vs jitter.
    let taus = [75u64, 90];
    // Boundary axis: sign-persistence with the convergence-critical lever.
    let sensitivities = [1.0f64, 1.5];
    // tighten_multiplier: THE convergence lever. 1.0 = symmetric (best ramp),
    // up to 3.0 = original (best jitter/safety). Sweep the whole range.
    let tightens = [1.0f64, 1.5, 2.0, 3.0];
    let discounts = [0.10f64, 0.15]; // the react-winning settings
    let max_disc = 0.4f64; // higher caps gave no react benefit last time
    // Update axis: aggressiveness drives convergence and overshoot.
    // (eta_base, eta_max, acceleration)
    let updates = [
        (0.3f32, 0.6f32, 0.2f32),
        (0.3f32, 0.8f32, 0.2f32),
        (0.3f32, 0.8f32, 0.3f32),
    ];

    let mut algorithms: Vec<AlgorithmSpec> = Vec::new();

    // References: production + the balanced champion + the prior SignPersist
    // react leader (so the Pareto movement is anchored).
    algorithms.push(AlgorithmSpec::classic_vardiff_state());
    {
        let name = vardiff_sim::naming::triple_name(
            &EwmaEstimator::new(90),
            &AdaptivePoissonCusum::with_params(
                PoissonCI::default_parametric(),
                AsymmetricCusumBoundary::new(1.0, 0.05, 3.0),
                8,
            ),
            &AcceleratingPartialRetarget::new(0.3, 0.6, 0.2),
        );
        algorithms.push(AlgorithmSpec::new(name, move |clock| {
            VardiffBox(Box::new(Composed::new(
                EwmaEstimator::new(90),
                AdaptivePoissonCusum::with_params(
                    PoissonCI::default_parametric(),
                    AsymmetricCusumBoundary::new(1.0, 0.05, 3.0),
                    8,
                ),
                AcceleratingPartialRetarget::new(0.3, 0.6, 0.2),
                1.0,
                clock,
            )))
        }));
    }

    for &tau in &taus {
        for &sens in &sensitivities {
            for &tighten in &tightens {
                for &disc in &discounts {
                    for &(eb, em, acc) in &updates {
                        let name = vardiff_sim::naming::triple_name(
                            &EwmaEstimator::new(tau),
                            &SignPersistenceCusumBoundary::new(sens, 0.05, tighten, disc, max_disc),
                            &AcceleratingPartialRetarget::new(eb, em, acc),
                        );
                        algorithms.push(AlgorithmSpec::new(name, move |clock| {
                            VardiffBox(Box::new(Composed::new(
                                EwmaEstimator::new(tau),
                                SignPersistenceCusumBoundary::new(sens, 0.05, tighten, disc, max_disc),
                                AcceleratingPartialRetarget::new(eb, em, acc),
                                1.0,
                                clock,
                            )))
                        }));
                    }
                }
            }
        }
    }

    let scenarios = vec![
        Scenario::ColdStart,
        Scenario::Stable,
        Scenario::Step { delta_pct: -10 },
        Scenario::Step { delta_pct: -50 },
    ];

    let grid = Grid {
        algorithms,
        share_rates: vec![4.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 30.0],
        scenarios,
        trial_count,
        base_seed,
    };

    eprintln!(
        "Sweeping {} algorithms × {} cells × {} trials",
        grid.algorithms.len(),
        grid.share_rates.len() * grid.scenarios.len(),
        trial_count,
    );
    let started = Instant::now();
    let results = grid.run();
    eprintln!("Sweep complete in {:.1}s", started.elapsed().as_secs_f64());

    let metric = EqualWeightFitness;
    let mut profiles: Vec<Profile> = Vec::new();
    for (name, cells) in &results {
        let computed = metric.compute(cells);
        let mut sums = [0.0f64; 6];
        let mut count = 0u32;
        for (_spm, mv) in &computed {
            for (i, (key, _)) in AXES.iter().enumerate() {
                sums[i] += mv.get(key).unwrap_or(0.0);
            }
            count += 1;
        }
        if count == 0 {
            continue;
        }
        let axes: [f64; 6] = std::array::from_fn(|i| sums[i] / count as f64);
        let maximin = axes.iter().cloned().fold(f64::INFINITY, f64::min);
        let mean = axes.iter().sum::<f64>() / 6.0;
        profiles.push(Profile {
            name: name.clone(),
            axes,
            maximin,
            mean,
        });
    }

    profiles.sort_by(|a, b| {
        b.maximin
            .partial_cmp(&a.maximin)
            .unwrap()
            .then(b.mean.partial_cmp(&a.mean).unwrap())
    });

    let hull: [f64; 6] = std::array::from_fn(|i| {
        profiles.iter().map(|p| p.axes[i]).fold(0.0_f64, f64::max)
    });

    let mut md = String::new();
    md.push_str("# SignPersist co-tuned (maximin) sweep\n\n");
    md.push_str(&format!(
        "{} configs, {} trials/cell, seed `{:#x}`. Estimator τ × boundary \
         (sensitivity, tighten, discount) × update co-tuned around \
         SignPersist. Ranked by **min axis** (maximin). All axes higher = \
         better.\n\n",
        profiles.len(),
        trial_count,
        base_seed,
    ));
    md.push_str("| # | Algorithm | react-10% | react-50% | jitter | step-safe | conv | overshoot | **min** | mean |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for (rank, p) in profiles.iter().enumerate().take(40) {
        let worst_i = p
            .axes
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        let cell = |i: usize| {
            if i == worst_i {
                format!("_{:.3}_", p.axes[i])
            } else {
                format!("{:.3}", p.axes[i])
            }
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | **{:.3}** | {:.3} |\n",
            rank + 1,
            p.name,
            cell(0),
            cell(1),
            cell(2),
            cell(3),
            cell(4),
            cell(5),
            p.maximin,
            p.mean,
        ));
    }
    md.push_str(&format!(
        "| | **best-in-class** | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | | |\n",
        hull[0], hull[1], hull[2], hull[3], hull[4], hull[5],
    ));

    let path = out_dir.join("signpersist_cotuned_sweep.md");
    fs::create_dir_all(&out_dir)?;
    fs::write(&path, &md)?;

    eprintln!("\nTop 12 by maximin:\n");
    eprintln!("| rank | min | mean | react10 | conv | algorithm |");
    eprintln!("| --- | --- | --- | --- | --- | --- |");
    for (rank, p) in profiles.iter().enumerate().take(12) {
        eprintln!(
            "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {} |",
            rank + 1,
            p.maximin,
            p.mean,
            p.axes[0],
            p.axes[4],
            p.name
        );
    }
    eprintln!(
        "\nhull: react10={:.3} conv={:.3} (prior maximin ceiling 0.551)",
        hull[0], hull[4]
    );
    eprintln!("Wrote {}", path.display());
    Ok(())
}
