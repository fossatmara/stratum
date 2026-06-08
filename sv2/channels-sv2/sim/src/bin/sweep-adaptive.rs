//! Parameter sweep for the AdaptivePoissonCusum boundary composition.
//!
//! Sweeps: CUSUM sensitivity × tighten_multiplier × spm_threshold
//! with fixed EwmaEstimator(120s) + AcceleratingPartialRetarget(0.2, 0.6, 0.2).

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use vardiff_sim::baseline::{CellResult, Scenario, DEFAULT_BASELINE_SEED};
use vardiff_sim::grid::{AlgorithmSpec, Grid, VardiffBox};
use vardiff_sim::metrics;
use vardiff_sim::metrics::DerivedMetric;

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptivePoissonCusum, AsymmetricCusumBoundary, Composed,
    EwmaEstimator, PoissonCI,
};

fn main() -> std::io::Result<()> {
    let trial_count: usize = env::var("VARDIFF_SWEEP_TRIALS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
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

    let sensitivities = vec![1.0, 1.5, 2.0, 2.5];
    let tighten_mults = vec![1.5, 2.0, 3.0, 4.0];
    let transitions = vec![10u32, 20, 30, 50];
    let eta_base = 0.2f32;
    let eta_max = 0.6f32;
    let acceleration = 0.2f32;

    let mut algorithms: Vec<AlgorithmSpec> = Vec::new();

    // Reference algorithms
    algorithms.push(AlgorithmSpec::full_remedy());
    algorithms.push(AlgorithmSpec::classic_vardiff_state());

    // Sweep: sensitivity × tighten × transition
    for &sens in &sensitivities {
        for &tighten in &tighten_mults {
            for &trans in &transitions {
                let name = format!(
                    "Adp-s{}-t{}-tr{}",
                    (sens * 10.0) as u32,
                    (tighten * 10.0) as u32,
                    trans
                );
                algorithms.push(AlgorithmSpec::new(name, move |clock| {
                    let boundary = AdaptivePoissonCusum::with_params(
                        PoissonCI::default_parametric(),
                        AsymmetricCusumBoundary::new(sens, 0.05, tighten),
                        trans,
                    );
                    let inner = Composed::new(
                        EwmaEstimator::new(120),
                        boundary,
                        AcceleratingPartialRetarget::new(eta_base, eta_max, acceleration),
                        1.0,
                        clock,
                    );
                    VardiffBox(Box::new(inner))
                }));
            }
        }
    }

    let mut scenarios = vec![Scenario::ColdStart, Scenario::Stable];
    for &d in &[-50i32, -25, -10, 10, 25, 50] {
        scenarios.push(Scenario::Step { delta_pct: d });
    }
    for &d in &[-50i32, -25, 25, 50] {
        scenarios.push(Scenario::SettledStep {
            settle_minutes: 60,
            delta_pct: d,
        });
    }

    let grid = Grid {
        algorithms,
        share_rates: vec![4.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 30.0],
        scenarios,
        trial_count,
        base_seed,
    };

    eprintln!(
        "Adaptive boundary sweep: {} algorithms × {} cells × {} trials = {} total",
        grid.algorithms.len(),
        grid.share_rates.len() * grid.scenarios.len(),
        trial_count,
        grid.total_runs() * trial_count,
    );

    let started = Instant::now();
    let results = grid.run_paired();
    let elapsed = started.elapsed();
    eprintln!("Sweep complete in {:.1}s", elapsed.as_secs_f64());

    fs::create_dir_all(&out_dir)?;

    // Compute comprehensive fitness via the derived metric system
    let derived_registry = metrics::derived_registry();
    let comp_fitness = derived_registry
        .iter()
        .find(|d| d.id() == "comprehensive_fitness")
        .expect("ComprehensiveFitness not in registry");

    // For each algorithm, compute comprehensive fitness per SPM
    let mut algo_fitness: Vec<(String, Vec<(u32, f64)>)> = Vec::new();
    for (name, cells) in &results {
        let computed = comp_fitness.compute(cells);
        let mut per_spm: Vec<(u32, f64)> = Vec::new();
        for (spm, mv) in &computed {
            if let Some(v) = mv.get("comprehensive_fitness") {
                per_spm.push((*spm as u32, v));
            }
        }
        per_spm.sort_by_key(|(spm, _)| *spm);
        algo_fitness.push((name.clone(), per_spm));
    }

    // Sort by mean comprehensive fitness
    algo_fitness.sort_by(|a, b| {
        let avg_a: f64 = if a.1.is_empty() {
            0.0
        } else {
            a.1.iter().map(|(_, v)| v).sum::<f64>() / a.1.len() as f64
        };
        let avg_b: f64 = if b.1.is_empty() {
            0.0
        } else {
            b.1.iter().map(|(_, v)| v).sum::<f64>() / b.1.len() as f64
        };
        avg_b.partial_cmp(&avg_a).unwrap()
    });

    // Also compute operational fitness
    let op_fitness = derived_registry
        .iter()
        .find(|d| d.id() == "operational_fitness")
        .expect("OperationalFitness not in registry");

    // Build report
    let share_rates: Vec<u32> = vec![4, 6, 8, 10, 12, 15, 20, 30];
    let mut report = String::new();
    report.push_str(&format!(
        "# Adaptive Boundary Parameter Sweep\n\n\
         {} trials/cell, {} algorithms, base_seed = {:#x}, completed in {:.1}s\n\n\
         Boundary: AdaptivePoissonCusum (PoissonCI below transition, CUSUM above)\n\
         Update: AcceleratingPartialRetarget(eta={}, max={}, acc={})\n\
         Estimator: EwmaEstimator(120s)\n\n",
        trial_count,
        grid.algorithms.len(),
        base_seed,
        elapsed.as_secs_f64(),
        eta_base,
        eta_max,
        acceleration,
    ));

    // Top 20 table
    report.push_str("## Top 20 by mean comprehensive fitness\n\n");
    report.push_str("| Rank | Algorithm | Mean |");
    for spm in &share_rates {
        report.push_str(&format!(" {} |", spm));
    }
    report.push_str("\n| --- | --- | --- |");
    for _ in &share_rates {
        report.push_str(" --- |");
    }
    report.push('\n');

    for (i, (name, per_spm)) in algo_fitness.iter().take(20).enumerate() {
        let avg: f64 = if per_spm.is_empty() {
            0.0
        } else {
            per_spm.iter().map(|(_, v)| v).sum::<f64>() / per_spm.len() as f64
        };
        report.push_str(&format!("| {} | {} | {:.4} |", i + 1, name, avg));
        for &target_spm in &share_rates {
            match per_spm.iter().find(|(s, _)| *s == target_spm) {
                Some((_, v)) => report.push_str(&format!(" {:.3} |", v)),
                None => report.push_str(" — |"),
            }
        }
        report.push('\n');
    }

    // Per-cell metrics for top 5: reaction rate, accuracy, overshoot, jitter
    let metric_keys = vec![
        ("reaction_rate", "Reaction rate (cold-start -50%)"),
        ("settled_accuracy_p50", "Settled accuracy p50 (stable)"),
        ("jitter_mean_per_min", "Jitter mean (stable)"),
    ];

    let top5_names: Vec<&str> = algo_fitness
        .iter()
        .take(5)
        .map(|(n, _)| n.as_str())
        .collect();
    let ref_names = vec!["FullRemedy", "VardiffState"];

    for (key, label) in &metric_keys {
        report.push_str(&format!("\n## {} (top 5 + references)\n\n", label));
        report.push_str("| Algorithm |");
        for spm in &share_rates {
            report.push_str(&format!(" {} |", spm));
        }
        report.push_str("\n| --- |");
        for _ in &share_rates {
            report.push_str(" --- |");
        }
        report.push('\n');

        let all_names: Vec<&str> = top5_names
            .iter()
            .copied()
            .chain(ref_names.iter().copied())
            .collect();

        for name in &all_names {
            if let Some(cells) = results.get(*name) {
                report.push_str(&format!("| {} |", name));
                for &target_spm in &share_rates {
                    // Find cell matching this SPM + the right scenario
                    let scenario_filter = if key.contains("settled") || key.contains("jitter") {
                        "Stable"
                    } else {
                        "Step-50"
                    };
                    let val: Option<f64> = cells
                        .iter()
                        .filter(|c| c.shares_per_minute as u32 == target_spm)
                        .filter(|c| {
                            let sk = c.scenario_key();
                            if scenario_filter == "Stable" {
                                sk == "stable"
                            } else {
                                sk.contains("-50")
                            }
                        })
                        .filter_map(|c| c.get(key))
                        .next();
                    match val {
                        Some(v) => report.push_str(&format!(" {:.3} |", v)),
                        None => report.push_str(" — |"),
                    }
                }
                report.push('\n');
            }
        }
    }

    // Overshoot from cold-start scenario
    report.push_str("\n## Ramp overshoot p99 (cold-start) (top 5 + references)\n\n");
    report.push_str("| Algorithm |");
    for spm in &share_rates {
        report.push_str(&format!(" {} |", spm));
    }
    report.push_str("\n| --- |");
    for _ in &share_rates {
        report.push_str(" --- |");
    }
    report.push('\n');
    let all_names: Vec<&str> = top5_names
        .iter()
        .copied()
        .chain(ref_names.iter().copied())
        .collect();
    for name in &all_names {
        if let Some(cells) = results.get(*name) {
            report.push_str(&format!("| {} |", name));
            for &target_spm in &share_rates {
                let val: Option<f64> = cells
                    .iter()
                    .filter(|c| c.shares_per_minute as u32 == target_spm)
                    .filter(|c| c.scenario_key() == "cold_start")
                    .filter_map(|c| c.get("ramp_overshoot_p99"))
                    .next();
                match val {
                    Some(v) => report.push_str(&format!(" {:.1}% |", v * 100.0)),
                    None => report.push_str(" — |"),
                }
            }
            report.push('\n');
        }
    }

    let out_path = out_dir.join("adaptive_sweep.md");
    fs::write(&out_path, &report)?;
    eprintln!("Wrote {}", out_path.display());

    Ok(())
}
