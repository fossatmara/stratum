//! Generates a radar chart (SVG) comparing algorithms on the 6 equal-weight
//! fitness sub-metrics, averaged across share rates.
//!
//! Each algorithm is rendered as a colored polygon on 6 axes:
//!   - react-10%, react-50%, jitter, step-safety, convergence, overshoot
//!
//! This makes it immediately visible which algorithms are "round" (balanced)
//! vs. spiked (dominant on one cluster but weak on another).
//!
//! ## Usage
//!
//! ```text
//! cargo run --release --bin radar-chart
//! ```
//!
//! Output: `radar_chart.svg` (or override with `VARDIFF_RADAR_OUT`).
//!
//! ## Configuration
//!
//! - `VARDIFF_RADAR_ALGOS` — comma-separated algorithm subset to plot
//!   (default: all). Example: `VardiffState,FullRemedy,Adp10-max40`
//! - `VARDIFF_RADAR_SPM` — single share rate to plot instead of averaging
//!   (default: average across all). Example: `10`
//! - `VARDIFF_COMPARE_TRIALS` — trials per cell (default 1000).
//! - `VARDIFF_COMPARE_SEED` — base seed.
//! - `VARDIFF_RADAR_OUT` — output path (default `radar_chart.svg`).

use std::env;
use std::f64::consts::PI;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use vardiff_sim::baseline::{Scenario, DEFAULT_BASELINE_SEED, DEFAULT_TRIAL_COUNT};
use vardiff_sim::grid::{AlgorithmSpec, Grid, VardiffBox};
use vardiff_sim::metrics::{DerivedMetric, EqualWeightFitness};
use vardiff_sim::{
    AcceleratingPartialRetarget, AdaptivePoissonCusum, AsymmetricCusumBoundary, Composed,
    EwmaEstimator, HysteresisGate, PoissonCI,
};

const AXES: &[(&str, &str)] = &[
    ("reaction_10", "React −10%"),
    ("reaction_50", "React −50%"),
    ("jitter", "Jitter control"),
    ("step_safety", "Step safety"),
    ("convergence", "Convergence"),
    ("overshoot", "Overshoot safety"),
];

const COLORS: &[&str] = &[
    "#e41a1c", "#377eb8", "#4daf4a", "#984ea3", "#ff7f00", "#a65628",
    "#f781bf", "#999999", "#66c2a5", "#fc8d62", "#8da0cb", "#e78ac3",
    "#a6d854", "#ffd92f", "#e5c494", "#b3b3b3",
];

fn main() -> std::io::Result<()> {
    let trial_count = env_or("VARDIFF_COMPARE_TRIALS", DEFAULT_TRIAL_COUNT);
    let base_seed = env_or_seed("VARDIFF_COMPARE_SEED", DEFAULT_BASELINE_SEED);
    let out_path = env::var("VARDIFF_RADAR_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("radar_chart.svg"));
    let filter_algos: Option<Vec<String>> = env::var("VARDIFF_RADAR_ALGOS")
        .ok()
        .map(|s| s.split(',').map(|a| a.trim().to_string()).collect());
    let filter_spm: Option<u32> = env::var("VARDIFF_RADAR_SPM")
        .ok()
        .and_then(|s| s.parse().ok());

    let mut scenarios = vec![Scenario::ColdStart, Scenario::Stable];
    for &delta in &[-50i32, -25, -10, -5, 5, 10, 25, 50] {
        scenarios.push(Scenario::Step { delta_pct: delta });
    }

    let grid = Grid {
        algorithms: build_algorithm_specs(),
        share_rates: vec![4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 25.0, 30.0],
        scenarios,
        trial_count,
        base_seed,
    };

    eprintln!(
        "Running {} algorithms × {} cells for radar chart...",
        grid.algorithms.len(),
        grid.share_rates.len() * grid.scenarios.len(),
    );

    let started = Instant::now();
    let results = grid.run();
    eprintln!("Sweep done in {:.1}s", started.elapsed().as_secs_f64());

    let metric = EqualWeightFitness;

    // Compute per-algorithm per-SPM scores
    let mut algo_profiles: Vec<(String, [f64; 6])> = Vec::new();
    let mut algo_names: Vec<&String> = results.keys().collect();
    algo_names.sort();

    for name in &algo_names {
        if let Some(ref filter) = filter_algos {
            if !filter.iter().any(|f| f == *name) {
                continue;
            }
        }

        let cells = &results[*name];
        let computed = metric.compute(cells);

        let mut axis_sums = [0.0f64; 6];
        let mut count = 0u32;

        for (spm, mv) in &computed {
            if let Some(target_spm) = filter_spm {
                if *spm as u32 != target_spm {
                    continue;
                }
            }
            axis_sums[0] += mv.get("reaction_10").unwrap_or(0.0);
            axis_sums[1] += mv.get("reaction_50").unwrap_or(0.0);
            axis_sums[2] += mv.get("jitter").unwrap_or(0.0);
            axis_sums[3] += mv.get("step_safety").unwrap_or(0.0);
            axis_sums[4] += mv.get("convergence").unwrap_or(0.0);
            axis_sums[5] += mv.get("overshoot").unwrap_or(0.0);
            count += 1;
        }

        if count > 0 {
            let avg: [f64; 6] = std::array::from_fn(|i| axis_sums[i] / count as f64);
            algo_profiles.push(((*name).clone(), avg));
        }
    }

    if algo_profiles.is_empty() {
        eprintln!("No algorithms matched filter. Exiting.");
        return Ok(());
    }

    // Best-in-class hull: per-axis maximum across all plotted algorithms.
    // Drawn as a dashed reference so each algorithm's shortfall on any axis
    // is the visible gap between its vertex and the hull.
    let hull: [f64; 6] = std::array::from_fn(|i| {
        algo_profiles
            .iter()
            .map(|(_, axes)| axes[i])
            .fold(0.0_f64, f64::max)
    });
    // Which algorithm leads each axis (for the gap report).
    let axis_leader: [String; 6] = std::array::from_fn(|i| {
        algo_profiles
            .iter()
            .max_by(|a, b| a.1[i].partial_cmp(&b.1[i]).unwrap())
            .map(|(n, _)| n.clone())
            .unwrap_or_default()
    });

    // Sort by maximin (worst axis, descending): the most *balanced*
    // algorithms — those with no big gap on any axis — come first. This is
    // the lens for "balanced best", distinct from ranking by the mean.
    algo_profiles.sort_by(|a, b| {
        let min_a = a.1.iter().cloned().fold(f64::INFINITY, f64::min);
        let min_b = b.1.iter().cloned().fold(f64::INFINITY, f64::min);
        min_b.partial_cmp(&min_a).unwrap()
    });

    // Markdown table: per-axis values plus min (worst axis, the maximin
    // score) and mean. Sorted most-balanced-first. The trailing rows show
    // best-in-class per axis and who achieves it, so gaps are explicit.
    eprintln!("\n## Equal-weight fitness radar (averaged across SPMs):\n");
    eprintln!("Sorted by **min axis** (maximin) descending — most balanced first.");
    eprintln!("All axes higher = better (jitter/step/overshoot are inverted to 'safety').\n");
    eprintln!(
        "| Algorithm | react-10% | react-50% | jitter | step-safe | conv | overshoot | **min** | avg |"
    );
    eprintln!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for (name, axes) in &algo_profiles {
        let avg: f64 = axes.iter().sum::<f64>() / 6.0;
        let min: f64 = axes.iter().cloned().fold(f64::INFINITY, f64::min);
        // Flag the worst axis with a marker so gaps are findable in text.
        let worst_i = axes
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        let cell = |i: usize| {
            if i == worst_i {
                format!("_{:.3}_", axes[i]) // italic = worst axis for this algo
            } else {
                format!("{:.3}", axes[i])
            }
        };
        eprintln!(
            "| {} | {} | {} | {} | {} | {} | {} | **{:.3}** | {:.3} |",
            name,
            cell(0), cell(1), cell(2), cell(3), cell(4), cell(5),
            min, avg,
        );
    }
    eprintln!(
        "| **best-in-class** | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | | |",
        hull[0], hull[1], hull[2], hull[3], hull[4], hull[5],
    );
    eprintln!("\n**Per-axis leader:**");
    for (i, (_key, label)) in AXES.iter().enumerate() {
        eprintln!("- {}: {:.3} ({})", label, hull[i], axis_leader[i]);
    }

    let svg = render_radar_svg(&algo_profiles, &hull, filter_spm);
    fs::write(&out_path, svg)?;
    eprintln!("\nWrote {}", out_path.display());
    Ok(())
}

fn render_radar_svg(
    profiles: &[(String, [f64; 6])],
    hull: &[f64; 6],
    filter_spm: Option<u32>,
) -> String {
    let cx = 400.0f64;
    let cy = 350.0;
    let radius = 250.0;
    let n = AXES.len();
    let width = 800;
    // 2-column legend + a hull key row.
    let legend_rows = profiles.len().div_ceil(2) + 1;
    let height = 712 + 20 * legend_rows;

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" font-family="system-ui, -apple-system, sans-serif" font-size="13">
<rect width="100%" height="100%" fill="#fafafa"/>
"##,
        width, height
    ));

    // Title
    let title = match filter_spm {
        Some(spm) => format!("Vardiff Equal-Weight Fitness Radar \u{2014} {} SPM", spm),
        None => "Vardiff Equal-Weight Fitness Radar \u{2014} Averaged Across Share Rates".to_string(),
    };
    svg.push_str(&format!(
        r##"<text x="{}" y="30" text-anchor="middle" font-size="16" font-weight="bold">{}</text>
"##,
        cx, title
    ));

    // Grid rings at 0.2, 0.4, 0.6, 0.8, 1.0
    for ring in 1..=5 {
        let r = radius * (ring as f64 / 5.0);
        let mut points = String::new();
        for i in 0..n {
            let angle = -PI / 2.0 + 2.0 * PI * (i as f64) / (n as f64);
            let x = cx + r * angle.cos();
            let y = cy + r * angle.sin();
            if i == 0 {
                points.push_str(&format!("{:.1},{:.1}", x, y));
            } else {
                points.push_str(&format!(" {:.1},{:.1}", x, y));
            }
        }
        let opacity = if ring == 5 { "0.3" } else { "0.15" };
        svg.push_str(&format!(
            r##"<polygon points="{}" fill="none" stroke="#666" stroke-opacity="{}" stroke-width="0.5"/>
"##,
            points, opacity
        ));
        let label_y = cy - r - 3.0;
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="9" fill="#999">{:.1}</text>
"##,
            cx, label_y, ring as f64 * 0.2
        ));
    }

    // Axis lines and labels
    for i in 0..n {
        let angle = -PI / 2.0 + 2.0 * PI * (i as f64) / (n as f64);
        let x_end = cx + radius * angle.cos();
        let y_end = cy + radius * angle.sin();
        svg.push_str(&format!(
            r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#aaa" stroke-width="0.5"/>
"##,
            cx, cy, x_end, y_end
        ));
        let label_r = radius + 20.0;
        let lx = cx + label_r * angle.cos();
        let ly = cy + label_r * angle.sin();
        let anchor = if (angle.cos()).abs() < 0.1 {
            "middle"
        } else if angle.cos() > 0.0 {
            "start"
        } else {
            "end"
        };
        // Every axis is higher-is-better; the "↑" makes that explicit so a
        // reader never wonders whether an axis is inverted.
        let label = format!("{} \u{2191}", AXES[i].1);
        svg.push_str(&format!(
            r##"<text x="{:.1}" y="{:.1}" text-anchor="{}" font-size="12" font-weight="500">{}</text>
"##,
            lx, ly + 4.0, anchor, label
        ));
    }

    // Best-in-class hull: dashed gray polygon at the per-axis maximum. Any
    // algorithm's shortfall on an axis is the visible gap to this outline.
    {
        let mut points = String::new();
        for i in 0..n {
            let angle = -PI / 2.0 + 2.0 * PI * (i as f64) / (n as f64);
            let r = radius * hull[i].clamp(0.0, 1.0);
            let x = cx + r * angle.cos();
            let y = cy + r * angle.sin();
            if i == 0 {
                points.push_str(&format!("{:.1},{:.1}", x, y));
            } else {
                points.push_str(&format!(" {:.1},{:.1}", x, y));
            }
        }
        svg.push_str(&format!(
            r##"<polygon points="{}" fill="none" stroke="#333" stroke-width="1.5" stroke-dasharray="6 4" stroke-opacity="0.6"/>
"##,
            points
        ));
    }

    // Algorithm polygons
    for (idx, (_name, axes)) in profiles.iter().enumerate() {
        let color = COLORS[idx % COLORS.len()];
        let mut points = String::new();
        for i in 0..n {
            let angle = -PI / 2.0 + 2.0 * PI * (i as f64) / (n as f64);
            let r = radius * axes[i].clamp(0.0, 1.0);
            let x = cx + r * angle.cos();
            let y = cy + r * angle.sin();
            if i == 0 {
                points.push_str(&format!("{:.1},{:.1}", x, y));
            } else {
                points.push_str(&format!(" {:.1},{:.1}", x, y));
            }
        }
        svg.push_str(&format!(
            r##"<polygon points="{}" fill="{}" fill-opacity="0.08" stroke="{}" stroke-width="2" stroke-opacity="0.8"/>
"##,
            points, color, color
        ));
        for i in 0..n {
            let angle = -PI / 2.0 + 2.0 * PI * (i as f64) / (n as f64);
            let r = radius * axes[i].clamp(0.0, 1.0);
            let x = cx + r * angle.cos();
            let y = cy + r * angle.sin();
            svg.push_str(&format!(
                r##"<circle cx="{:.1}" cy="{:.1}" r="3" fill="{}"/>
"##,
                x, y, color
            ));
        }
    }

    // Legend. Each entry shows "min=… avg=…" so the balance (min) and the
    // overall level (avg) are both visible; entries are in maximin order.
    let legend_y_start = 690.0;
    let col_width = width as f64 / 2.0; // 2 columns — names are long triples
    // Dashed-hull key first.
    svg.push_str(&format!(
        r##"<line x1="20" y1="{:.0}" x2="32" y2="{:.0}" stroke="#333" stroke-width="1.5" stroke-dasharray="6 4"/>
<text x="40" y="{:.0}" font-size="11" font-style="italic">best-in-class hull (per-axis max)</text>
"##,
        legend_y_start - 4.0, legend_y_start - 4.0, legend_y_start
    ));
    for (idx, (name, axes)) in profiles.iter().enumerate() {
        let color = COLORS[idx % COLORS.len()];
        let col = idx % 2;
        let row = idx / 2;
        let lx = 20.0 + col as f64 * col_width;
        let ly = legend_y_start + 22.0 + row as f64 * 20.0;
        let avg: f64 = axes.iter().sum::<f64>() / 6.0;
        let min: f64 = axes.iter().cloned().fold(f64::INFINITY, f64::min);
        svg.push_str(&format!(
            r##"<rect x="{:.0}" y="{:.0}" width="12" height="12" fill="{}" rx="2"/>
<text x="{:.0}" y="{:.0}" font-size="11">{} (min={:.2} avg={:.2})</text>
"##,
            lx, ly - 9.0, color, lx + 16.0, ly, name, min, avg
        ));
    }

    svg.push_str("</svg>\n");
    svg
}

/// Curated set of *interesting contenders* for head-to-head comparison,
/// distilled from the radar + maximin sweeps. Set `VARDIFF_RADAR_FULL=1` to
/// instead plot the broad historical set (kept in `full_algorithm_specs`).
fn build_algorithm_specs() -> Vec<AlgorithmSpec> {
    if env::var("VARDIFF_RADAR_FULL").is_ok() {
        return full_algorithm_specs();
    }

    vec![
        // Production reference (monolith).
        AlgorithmSpec::classic_vardiff_state(),
        // Balanced Pareto optimum (maximin 0.551).
        AlgorithmSpec::balanced(),
        // React-priority Pareto optimum (react−10% 0.696).
        AlgorithmSpec::react_priority(),
    ]
}

/// The broad historical set (every shipped/experimental algorithm), used when
/// `VARDIFF_RADAR_FULL=1` is set.
fn full_algorithm_specs() -> Vec<AlgorithmSpec> {
    vec![
        AlgorithmSpec::classic_vardiff_state(),
        AlgorithmSpec::classic_composed(),
        AlgorithmSpec::parametric(),
        AlgorithmSpec::parametric_strict(),
        AlgorithmSpec::classic_partial_retarget(0.3),
        AlgorithmSpec::ewma_60s(),
        AlgorithmSpec::sliding_window(10),
        AlgorithmSpec::full_remedy(),
        AlgorithmSpec::poisson_accel(),
        AlgorithmSpec::adaptive_boundary(10),
        derived_spec(
            || EwmaEstimator::new(120),
            || AdaptivePoissonCusum::new(10),
            || AcceleratingPartialRetarget::new(0.2, 0.4, 0.2),
        ),
        derived_spec(
            || EwmaEstimator::new(120),
            || {
                AdaptivePoissonCusum::with_params(
                    PoissonCI::default_parametric(),
                    AsymmetricCusumBoundary::new(1.5, 0.05, 1.0),
                    10,
                )
            },
            || AcceleratingPartialRetarget::new(0.2, 0.6, 0.2),
        ),
        derived_spec(
            || EwmaEstimator::new(120),
            || HysteresisGate::new(4, 60, 0.85, 1.15),
            || AcceleratingPartialRetarget::new(0.2, 0.4, 0.2),
        ),
        AlgorithmSpec::ewma_adaptive_cusum(120, 1.5, 0.05, 0.2),
        AlgorithmSpec::ckpool_remedy(),
        AlgorithmSpec::ckpool_remedy_ft(12),
    ]
}

/// Build an `AlgorithmSpec` whose name is DERIVED from its three parts via
/// `naming::triple_name`, so the displayed name can never drift from what
/// the algorithm actually runs. Each closure constructs a fresh component
/// (components aren't all `Clone`): one set is built to compute the name,
/// another set per trial inside the factory.
fn derived_spec<E, B, U, FE, FB, FU>(make_e: FE, make_b: FB, make_u: FU) -> AlgorithmSpec
where
    E: vardiff_sim::composed::Estimator + 'static,
    B: vardiff_sim::composed::Boundary + 'static,
    U: vardiff_sim::composed::UpdateRule + 'static,
    FE: Fn() -> E + Send + Sync + 'static,
    FB: Fn() -> B + Send + Sync + 'static,
    FU: Fn() -> U + Send + Sync + 'static,
{
    let name = vardiff_sim::naming::triple_name(&make_e(), &make_b(), &make_u());
    AlgorithmSpec::new(name, move |clock| {
        VardiffBox(Box::new(Composed::new(
            make_e(),
            make_b(),
            make_u(),
            1.0,
            clock,
        )))
    })
}

fn env_or<T: std::str::FromStr>(var: &str, default: T) -> T {
    env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_or_seed(var: &str, default: u64) -> u64 {
    if let Ok(s) = env::var(var) {
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            return u64::from_str_radix(hex, 16).unwrap_or(default);
        }
        return s.parse().unwrap_or(default);
    }
    default
}
