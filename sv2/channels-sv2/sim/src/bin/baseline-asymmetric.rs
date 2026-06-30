//! Generate per-metric baseline for the asymmetric PoissonCI production composition.

use std::env;
use std::time::Instant;

use vardiff_sim::baseline::{Scenario, DEFAULT_BASELINE_SEED};
use vardiff_sim::grid::{AlgorithmSpec, Grid, VardiffBox};
use vardiff_sim::metrics;

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AsymmetricCusumBoundary, AsymmetricPoissonCI, Boundary, Composed,
    EstimatorSnapshot, EwmaEstimator,
};

#[derive(Debug, Clone)]
struct AdaptiveAsymPoissonCusum {
    poisson: AsymmetricPoissonCI,
    cusum: AsymmetricCusumBoundary,
    spm_threshold: u32,
}

impl Boundary for AdaptiveAsymPoissonCusum {
    fn threshold(&self, dt_secs: u64, shares_per_minute: f32, snap: &EstimatorSnapshot) -> f64 {
        if (shares_per_minute as u32) < self.spm_threshold {
            self.poisson.threshold(dt_secs, shares_per_minute, snap)
        } else {
            self.cusum.threshold(dt_secs, shares_per_minute, snap)
        }
    }

    fn code(&self) -> String {
        format!(
            "AdaptAsymPC-spm{}[{}|{}]",
            self.spm_threshold,
            self.poisson.code(),
            self.cusum.code()
        )
    }
}

fn main() -> std::io::Result<()> {
    let trial_count: usize = env::var("VARDIFF_BASELINE_TRIALS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let base_seed: u64 = env::var("VARDIFF_BASELINE_SEED")
        .ok()
        .and_then(|s| {
            s.strip_prefix("0x")
                .and_then(|h| u64::from_str_radix(h, 16).ok())
                .or_else(|| s.parse().ok())
        })
        .unwrap_or(DEFAULT_BASELINE_SEED);

    let algorithms = vec![
        AlgorithmSpec::new("AsymProduction", |clock| {
            let boundary = AdaptiveAsymPoissonCusum {
                poisson: AsymmetricPoissonCI::new(2.576, 0.05, 3.0),
                cusum: AsymmetricCusumBoundary::new(1.5, 0.05, 3.0),
                spm_threshold: 10,
            };
            let inner = Composed::new(
                EwmaEstimator::new(120),
                boundary,
                AcceleratingPartialRetarget::new(0.2, 0.4, 0.2),
                1.0,
                clock,
            );
            VardiffBox(Box::new(inner))
        }),
        AlgorithmSpec::classic_vardiff_state(),
    ];

    let share_rates = vec![4.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 25.0, 30.0];

    let mut scenarios = vec![Scenario::ColdStart, Scenario::Stable];
    for &d in &[-50i32, -25, -10, -5, 5, 10, 25, 50] {
        scenarios.push(Scenario::Step { delta_pct: d });
    }

    let grid = Grid {
        algorithms,
        share_rates,
        scenarios,
        trial_count,
        base_seed,
    };

    eprintln!(
        "Baseline comparison: {} algorithms × {} cells × {} trials",
        grid.algorithms.len(),
        grid.share_rates.len() * grid.scenarios.len(),
        trial_count,
    );

    let started = Instant::now();
    let results = grid.run_paired();
    let elapsed = started.elapsed();
    eprintln!("Complete in {:.2}s", elapsed.as_secs_f64());

    for (name, cells) in &results {
        println!("\n# {}\n", name);

        let mut by_spm: std::collections::BTreeMap<u32, Vec<&vardiff_sim::baseline::CellResult>> =
            std::collections::BTreeMap::new();
        for cell in cells {
            by_spm.entry(cell.shares_per_minute as u32).or_default().push(cell);
        }

        println!("## Settled accuracy (stable load)");
        println!("| SPM | p50 | p90 | p99 |");
        println!("| --- | --- | --- | --- |");
        for (&spm, spm_cells) in &by_spm {
            for cell in spm_cells {
                if matches!(cell.scenario, Scenario::Stable) {
                    if let Some(mv) = cell.metrics.get("settled_accuracy") {
                        let p50 = mv.get("settled_accuracy_p50").map(|v| format!("{:.1}%", v * 100.0)).unwrap_or("-".into());
                        let p90 = mv.get("settled_accuracy_p90").map(|v| format!("{:.1}%", v * 100.0)).unwrap_or("-".into());
                        let p99 = mv.get("settled_accuracy_p99").map(|v| format!("{:.1}%", v * 100.0)).unwrap_or("-".into());
                        println!("| {} | {} | {} | {} |", spm, p50, p90, p99);
                    }
                }
            }
        }

        println!("\n## Steady-state jitter (fires/min)");
        println!("| SPM | p50 | p90 | mean |");
        println!("| --- | --- | --- | --- |");
        for (&spm, spm_cells) in &by_spm {
            for cell in spm_cells {
                if matches!(cell.scenario, Scenario::Stable) {
                    if let Some(mv) = cell.metrics.get("jitter") {
                        let p50 = mv.get("jitter_p50_per_min").map(|v| format!("{:.3}", v)).unwrap_or("-".into());
                        let p90 = mv.get("jitter_p90_per_min").map(|v| format!("{:.3}", v)).unwrap_or("-".into());
                        let mean = mv.get("jitter_mean_per_min").map(|v| format!("{:.3}", v)).unwrap_or("-".into());
                        println!("| {} | {} | {} | {} |", spm, p50, p90, mean);
                    }
                }
            }
        }

        println!("\n## Cold-start overshoot");
        println!("| SPM | p50 | p90 | p99 |");
        println!("| --- | --- | --- | --- |");
        for (&spm, spm_cells) in &by_spm {
            for cell in spm_cells {
                if matches!(cell.scenario, Scenario::ColdStart) {
                    if let Some(mv) = cell.metrics.get("ramp_target_overshoot") {
                        let p50 = mv.get("ramp_target_overshoot_p50").map(|v| format!("{:.1}%", v * 100.0)).unwrap_or("-".into());
                        let p90 = mv.get("ramp_target_overshoot_p90").map(|v| format!("{:.1}%", v * 100.0)).unwrap_or("-".into());
                        let p99 = mv.get("ramp_target_overshoot_p99").map(|v| format!("{:.1}%", v * 100.0)).unwrap_or("-".into());
                        println!("| {} | {} | {} | {} |", spm, p50, p90, p99);
                    }
                }
            }
        }

        println!("\n## Reaction sensitivity (P[fire within 5 min])");
        println!("| Δ% | 4 | 6 | 8 | 10 | 12 | 15 | 20 | 30 |");
        println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
        for &delta in &[-50i32, -25, -10, -5, 5, 10, 25, 50] {
            print!("| {:+}% |", delta);
            for &spm in &[4u32, 6, 8, 10, 12, 15, 20, 30] {
                let mut found = false;
                if let Some(spm_cells) = by_spm.get(&spm) {
                    for cell in spm_cells {
                        let is_match = match cell.scenario {
                            Scenario::Step { delta_pct } => delta_pct == delta,
                            _ => false,
                        };
                        if is_match {
                            if let Some(mv) = cell.metrics.get("reaction_time") {
                                if let Some(v) = mv.get("reaction_rate") {
                                    print!(" {:.2} |", v);
                                    found = true;
                                }
                            }
                        }
                    }
                }
                if !found {
                    print!(" - |");
                }
            }
            println!();
        }

        println!("\n## Convergence (cold start)");
        println!("| SPM | rate | p50 | p90 |");
        println!("| --- | --- | --- | --- |");
        for (&spm, spm_cells) in &by_spm {
            for cell in spm_cells {
                if matches!(cell.scenario, Scenario::ColdStart) {
                    if let Some(mv) = cell.metrics.get("convergence_time") {
                        let rate = mv.get("convergence_rate").map(|v| format!("{:.1}%", v * 100.0)).unwrap_or("-".into());
                        let p50 = mv.get("convergence_p50_secs").map(|v| format!("{}m", (v as u32) / 60)).unwrap_or("-".into());
                        let p90 = mv.get("convergence_p90_secs").map(|v| format!("{}m", (v as u32) / 60)).unwrap_or("-".into());
                        println!("| {} | {} | {} | {} |", spm, rate, p50, p90);
                    }
                }
            }
        }
    }

    Ok(())
}
