//! LOW-RATE STUDY, STAGE 0b — the EMPIRICAL resolution σ (replacing the analytic
//! 100/√N guess in lowrate-resolution.rs, which was SINGLE-SNAPSHOT scatter, not
//! the SUSTAINED-mean scatter the resolution question actually turns on).
//!
//! ===========================================================================
//! WHY THIS SUPERSEDES THE ANALYTIC σ. lowrate-resolution.rs put σ_e ≈ 100/√N_window
//! ≈ 32% at spm2 and concluded outcome C (sub-resolution). But 100/√N is the scatter
//! on a SINGLE snapshot of e; the resolution question is about the SUSTAINED depth =
//! the MEAN of e over the breached catch-up ticks. A mean over ~M_eff independent
//! estimator windows has scatter σ_single/√M_eff, NOT σ_single — and the escape
//! spans several τ-windows, so the real σ could be ~√M_eff smaller, moving the
//! verdict from C (overlap) toward B (resolvable). The empirical tick jitter in
//! deploy-coupling-transient was 0.5–0.9% — 40–60× below the analytic 32% — which is
//! the red flag that the formula is the wrong quantity. So MEASURE the real thing:
//! the ACROSS-TRIAL scatter of the per-trial sustained-depth statistic, at guard and
//! dense rates, for the champion estimator (EwmaEstimator(360), confirmed NOT
//! switched by the guard — composed.rs:262; only the BOUNDARY switches at spm6).
//!
//! THE RESOLUTION QUESTION, EMPIRICALLY. Each trial yields one sustained depth
//! (mean e over its breached catch-up ticks). Across trials that statistic has a
//! distribution; its STD is the operationally-relevant σ (a single deployment is ONE
//! realization). Can a population centered at the SAFE depth be told from one at the
//! SPIRAL depth, separation ~17%? Resolves iff σ ≪ 17%. We DON'T inherit the +13/+30
//! pair as truth — we report the measured guard-regime sustained depth AND its σ, and
//! ask whether σ alone forbids a 17%-separated distinction (σ ≳ 17 ⇒ C regardless of
//! where the centers sit; σ ≪ 17 ⇒ resolvable, the full sweep is worth building).
//! Dense rate (spm30) is the control: σ there must be small (it's the regime the
//! depth-read demonstrably worked in) — same method, validates by resolving where
//! the data supports it.
//!
//! Usage: cargo run --release --bin lowrate-sigma
//! Env: VARDIFF_LRS_TRIALS (default 400), VARDIFF_LRS_THREADS.
//! ===========================================================================

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptiveSignPersist, Composed, EwmaEstimator,
    SignPersistenceCusumBoundary,
};
use channels_sv2::vardiff::MockClock;
use vardiff_sim::baseline::{Phase, Scenario, DEFAULT_BASELINE_SEED, TRUE_HASHRATE};
use vardiff_sim::grid::{AlgorithmSpec, VardiffBox};
use vardiff_sim::trial::{run_trial_observed, TrialConfig};

const SENS: f64 = 1.5;
const BREACH_PCT: f64 = 5.0;
const SEPARATION: f64 = 17.0; // safe(+13) vs spiral(+30) — the gap to resolve
const SPMS: &[f32] = &[2.0, 4.0, 6.0, 30.0]; // guard (2,4), boundary edge (6), dense control (30)
const RATES_PPH: &[f32] = &[1.0, 2.0, 5.0, 10.0, 20.0, 40.0];

fn champion() -> AlgorithmSpec {
    AlgorithmSpec::new(format!("champion"), move |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(360),
            AdaptiveSignPersist::sign_persist(
                SignPersistenceCusumBoundary::new(SENS, 0.05, 8.0, 0.06, 0.6), 6,
            ),
            AcceleratingPartialRetarget::new(0.2, 0.6, 0.05), 1.0, clock,
        )))
    })
}

/// One trial → sustained depth (mean e over breached catch-up ticks), or NaN if the
/// escape never breached. Same decline profile + catch-up window as
/// deploy-coupling-transient.rs.
fn sustained_depth(a: &AlgorithmSpec, rate_pph: f32, spm: f32, seed: u64) -> f64 {
    let mature = 60u64;
    let rate = rate_pph / 100.0 / 60.0;
    let target = 0.50f32;
    let observe = 120u64;
    let mut phases = vec![Phase::Hold { secs: mature * 60, h: TRUE_HASHRATE }];
    let mut dm = 0u64;
    for m in 0..300u64 {
        let frac = (rate * (m as f32 + 1.0)).min(target);
        phases.push(Phase::Hold { secs: 60, h: TRUE_HASHRATE * (1.0 - frac) });
        dm = m + 1;
        if frac >= target { break; }
    }
    let floor_h = TRUE_HASHRATE * (1.0 - (rate * dm as f32).min(target));
    phases.push(Phase::Hold { secs: observe * 60, h: floor_h });
    let scen = Scenario::Custom { name: "decline".into(), phases, initial_estimate: None };
    let (proto, sched) = scen.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..proto };
    let clock = Arc::new(MockClock::new(0));
    let v = (a.factory)(clock.clone());
    let d_end = (mature + dm) * 60;
    let trial_end = d_end + observe * 60;
    let t = run_trial_observed(v, clock, config, &sched, seed);
    let mut breached: Vec<f64> = Vec::new();
    for tk in &t.ticks {
        if tk.t_secs > d_end && tk.t_secs <= trial_end {
            let h_true = sched.at(tk.t_secs.saturating_sub(30)) as f64;
            let e = (tk.current_hashrate_before as f64 / h_true).ln() * 100.0;
            if e > BREACH_PCT { breached.push(e); }
        }
    }
    if breached.is_empty() { f64::NAN } else { breached.iter().sum::<f64>() / breached.len() as f64 }
}

fn mean_std(v: &[f64]) -> (f64, f64) {
    let n = v.len();
    if n == 0 { return (f64::NAN, f64::NAN); }
    let m = v.iter().sum::<f64>() / n as f64;
    if n < 2 { return (m, f64::NAN); }
    let var = v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    (m, var.sqrt())
}

fn main() {
    let trials: usize = env::var("VARDIFF_LRS_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(400);
    let seed = DEFAULT_BASELINE_SEED ^ 0x10E_516;
    let nth: usize = env::var("VARDIFF_LRS_THREADS").ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)).max(1);

    // worst-severity per spm (the deepest escape — where spiral risk and the
    // resolution question both live); measure across-trial σ of sustained depth there.
    let jobs: Vec<(usize, usize)> =
        (0..SPMS.len()).flat_map(|si| (0..RATES_PPH.len()).map(move |ri| (si, ri))).collect();
    let next = AtomicUsize::new(0);
    // out[(si,ri)] = (mean, std, n_breached, frac_breached)
    let out: Mutex<Vec<(usize, usize, f64, f64, usize, f64)>> = Mutex::new(Vec::new());
    let a = champion();
    eprintln!("lowrate-sigma: {} spm × {} sev × {} trials, {} threads.", SPMS.len(), RATES_PPH.len(), trials, nth);
    std::thread::scope(|sc| {
        for _ in 0..nth {
            sc.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                if j >= jobs.len() { break; }
                let (si, ri) = jobs[j];
                let (spm, r) = (SPMS[si], RATES_PPH[ri]);
                let mut sds = Vec::with_capacity(trials);
                for i in 0..trials {
                    let d = sustained_depth(&a, r, spm, seed.wrapping_add((j as u64) << 24).wrapping_add(i as u64));
                    if d.is_finite() { sds.push(d); }
                }
                let (m, s) = mean_std(&sds);
                let frac = sds.len() as f64 / trials as f64;
                out.lock().unwrap().push((si, ri, m, s, sds.len(), frac));
                eprintln!("  spm{} sev{}%/hr done (n_breached={})", spm, r as u32, sds.len());
            });
        }
    });
    let raw = out.into_inner().unwrap();

    // worst severity per spm = the one with the deepest mean sustained depth.
    println!("\n## EMPIRICAL RESOLUTION σ — across-trial std of the sustained-depth statistic (champion estimator, 400 trials).");
    println!("σ is the SINGLE-DEPLOYMENT scatter on sustained depth. Resolution needs σ ≪ separation ({}%). Verdict per σ/separation.\n", SEPARATION as u32);
    println!("| spm | worst-sev sustained depth (mean±σ) | n breached/{} | σ/sep | verdict |", trials);
    println!("| --- | --- | --- | --- | --- |");
    for (si, &spm) in SPMS.iter().enumerate() {
        // pick the severity with the deepest mean among this spm's rows
        let rows: Vec<&(usize, usize, f64, f64, usize, f64)> =
            raw.iter().filter(|(s, ..)| *s == si).collect();
        let worst = rows.iter().filter(|(_, _, m, ..)| m.is_finite())
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
        match worst {
            Some((_, _, m, s, n, _)) => {
                let ratio = s / SEPARATION;
                let verdict = if ratio < 0.5 { "A: RESOLVES (σ≪sep)" }
                              else if ratio < 1.0 { "B: MARGINAL (σ<sep, needs trials)" }
                              else { "C: SUB-RESOLUTION (σ≳sep)" };
                println!("| {} | {:+.0} ± {:.0} | {}/{} | {:.2} | {} |", spm as u32, m, s, n, trials, ratio, verdict);
            }
            None => println!("| {} | no breach in any severity | 0 | — | (never breaches — sub-spiral by depth) |", spm as u32),
        }
    }
    println!("\nREAD: σ is the EMPIRICAL single-deployment scatter on sustained depth, replacing the analytic 100/√N (single-snapshot, wrong");
    println!("quantity). guard rows (spm2,4) σ/sep ≥ 1 ⇒ outcome C (sub-resolution) CONFIRMED empirically; σ/sep < 1 ⇒ the analytic 32%");
    println!("over-stated it and the question is B (marginal, resolvable with trials) or A. dense (spm30) is the control: must be A/B (the");
    println!("regime the depth-read worked). The guard-vs-dense σ contrast is the calibration: same method, resolves where data supports it.");
}
