//! EAGER-EASE DECOMPOSITION + RATE SWEEP — does the regime-dependence of upward
//! (self-deepening) fires live in the ESTIMATOR (presents up-excursions) or the
//! BOUNDARY (fires on weak evidence the reluctant half would refuse)? The count
//! alone is consistent with BOTH; only this decomposition is diagnostic.
//!
//! ===========================================================================
//! WHY (the under-determination the count can't resolve). eager-ease-mechanism.rs
//! found champion=0 upward-fires everywhere, and the two single-removals failing in
//! OPPOSITE regimes (boundary-jumpy fails dense spm6, estimator-jumpy fails sparse
//! spm2) — read as "complementarity: estimator-smoothing covers sparse, boundary-
//! reluctance covers dense." But a tighten-fire = (estimator PRESENTS an up-excursion)
//! AND (boundary FIRES on it), and BOTH conjuncts are rate-dependent. Two crossing
//! curves are consistent with the complementarity story (estimator data-availability
//! drives it) AND with "the boundary's false-crossing rate drives it, estimator is
//! second-order" (closer to substitutability). The count confounds them.
//!
//! THE DIAGNOSTIC (counterfactual: would the RELUCTANT boundary have refused this
//! fire?). At each upward fire, log delta (evidence) and threshold. The reluctant
//! counterfactual threshold = threshold × (8/tm): for a SYMMETRIC boundary (tm=1)
//! that is 8× the threshold it used; for the already-RELUCTANT boundary (tm=8) it is
//! itself. Split each upward fire:
//!   WEAK-REFUSABLE  : delta <  threshold×(8/tm) ⇒ the reluctant half WOULD have
//!     refused ⇒ the asymmetry is doing safety work (boundary fired on weak evidence).
//!     This is YOUR complementarity story's signature for the boundary's role.
//!   STRONG-UNREFUSABLE: delta ≥ threshold×(8/tm) ⇒ the reluctant half would have
//!     fired TOO ⇒ the estimator presented an excursion so extreme even reluctance
//!     can't refuse ⇒ the ESTIMATOR is the driver, the boundary-asymmetry is NOT what
//!     protects here. This is the ALTERNATIVE story's signature.
//! (For the reluctant boundary itself, every fire is STRONG-UNREFUSABLE by
//! construction — it fired through 8× — i.e. estimator-jumpy's fires are all
//! "estimator presented an excursion reluctance couldn't refuse," factor-a.)
//!
//! PRE-REGISTERED (decomposed, across rate {2,4,6,12,20,30}, real-decline — the
//! noisy diagnostic stimulus):
//!   - boundary-jumpy's rise-with-rate lives in WEAK-REFUSABLE (asymmetry would
//!     refuse, rising with rate) AND estimator-jumpy's sparse fires are STRONG-
//!     UNREFUSABLE (fast estimator presents extreme excursions) ⇒ CLEAN COMPLEMENTARITY
//!     confirmed at the mechanism level: boundary-reluctance does rate-dependent safety
//!     work (dense), estimator-smoothing does it (sparse), each its own regime.
//!   - the rate-dependence is mostly STRONG-UNREFUSABLE for BOTH arms (boundary fires
//!     on evidence the reluctant one would also act on) ⇒ the ESTIMATOR is the dominant
//!     mechanism, boundary-reluctance second-order ⇒ "complementarity" overstated;
//!     closer to "estimator-smoothing carries noise-safety, reluctance is a modulation."
//!   - curves non-monotone ⇒ two points were coincidence, neither story holds.
//!   - sparse-corner decomposition sub-resolution (few fires to classify at spm2) ⇒
//!     standing wall; decomposition resolvable in the denser regime, sparse behind it.
//!
//! The count is the COARSE instrument (consistent with multiple mechanisms); the
//! weak/strong decomposition is the FAITHFUL one (attributes rate-dependence to a
//! specific conjunct). The complementarity claim rides the faithful instrument — same
//! is-this-real discipline, and the attractive clean-partition story is exactly the
//! one to make diagnostic rather than merely consistent.
//!
//! Usage: cargo run --release --bin eager-ease-decompose
//! Env: VARDIFF_EED_TRIALS (default 200), VARDIFF_EED_THREADS.
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

const SPMS: &[f32] = &[2.0, 4.0, 6.0, 12.0, 20.0, 30.0];
const DECLINE_PPH: f32 = 40.0;
const DROP: f32 = 0.7;
const RELUCTANT_TM: f64 = 8.0; // the champion's tighten multiplier (the counterfactual)

fn variant(name: &str, tau: u64, tm: f64, s: f64) -> AlgorithmSpec {
    let nm = name.to_string();
    AlgorithmSpec::new(nm, move |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(tau),
            AdaptiveSignPersist::sign_persist(
                SignPersistenceCusumBoundary::new(s, 0.05, tm, 0.06, 0.6), 6,
            ),
            AcceleratingPartialRetarget::new(0.2, 0.6, 0.05), 1.0, clock,
        )))
    })
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() { return f64::NAN; }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// One real-decline trial → (total upward gated tighten-fires, weak-refusable,
/// strong-unrefusable). Gate: upward fire (new>before) while genuinely over-difficult
/// (e>5%, e from operating-point-vs-true, noise-free per eager-ease-mechanism's
/// verified gate). Decompose via the reluctant counterfactual delta vs threshold×(8/tm).
fn run(a: &AlgorithmSpec, tm: f64, spm: f32, seed: u64) -> (u32, u32, u32) {
    let mature = 60u64;
    let observe = 200u64;
    let rate = DECLINE_PPH / 100.0 / 60.0;
    let mut phases = vec![Phase::Hold { secs: mature * 60, h: TRUE_HASHRATE }];
    let mut dm = 0u64;
    for m in 0..600u64 {
        let frac = (rate * (m as f32 + 1.0)).min(DROP);
        phases.push(Phase::Hold { secs: 60, h: TRUE_HASHRATE * (1.0 - frac) });
        dm = m + 1;
        if frac >= DROP { break; }
    }
    let floor_h = TRUE_HASHRATE * (1.0 - DROP);
    phases.push(Phase::Hold { secs: observe * 60, h: floor_h });
    let scen = Scenario::Custom { name: "x".into(), phases, initial_estimate: None };
    let (proto, sched) = scen.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..proto };
    let clock = Arc::new(MockClock::new(0));
    let v = (a.factory)(clock.clone());
    let d_end = (mature + dm) * 60;
    let trial_end = d_end + observe * 60;
    let t = run_trial_observed(v, clock, config, &sched, seed);

    let (mut tf, mut weak, mut strong) = (0u32, 0u32, 0u32);
    for tk in &t.ticks {
        if tk.t_secs <= d_end || tk.t_secs > trial_end { continue; }
        let h_true = sched.at(tk.t_secs.saturating_sub(30)) as f64;
        let e = (tk.current_hashrate_before as f64 / h_true).ln() * 100.0;
        if !tk.fired || e <= 5.0 { continue; } // gate: genuinely over-difficult
        let newh = match tk.new_hashrate { Some(h) => h as f64, None => continue };
        if newh <= tk.current_hashrate_before as f64 { continue; } // upward fires only
        tf += 1;
        // counterfactual: would the reluctant boundary (tm=8) have refused?
        match (tk.delta, tk.threshold) {
            (Some(d), Some(thr)) => {
                let reluctant_thr = thr * (RELUCTANT_TM / tm); // 8× for symmetric, 1× for reluctant
                if d < reluctant_thr { weak += 1; } else { strong += 1; }
            }
            _ => strong += 1, // can't evaluate ⇒ conservatively attribute to estimator
        }
    }
    (tf, weak, strong)
}

fn main() {
    let trials: usize = env::var("VARDIFF_EED_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    let seed = DEFAULT_BASELINE_SEED ^ 0xDEC0_E5E;
    let nth: usize = env::var("VARDIFF_EED_THREADS").ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)).max(1);

    // (label, tau, tm, s)
    let variants: Vec<(&str, u64, f64, f64)> = vec![
        ("champion       (Ewma360, tm8)", 360, 8.0, 1.5),
        ("boundary-jumpy (Ewma360, tm1)", 360, 1.0, 0.3),
        ("estimator-jumpy(Ewma30,  tm8)", 30, 8.0, 1.5),
        ("both-jumpy     (Ewma30,  tm1)", 30, 1.0, 0.3),
    ];
    let jobs: Vec<(usize, usize)> = (0..variants.len()).flat_map(|vi|
        (0..SPMS.len()).map(move |si| (vi, si))).collect();
    let next = AtomicUsize::new(0);
    // (vi, si, med_tf, med_weak, med_strong)
    let out: Mutex<Vec<(usize, usize, f64, f64, f64)>> = Mutex::new(Vec::new());
    eprintln!("eager-ease-decompose: {} jobs × {} trials, {} threads, real-decline.", jobs.len(), trials, nth);
    std::thread::scope(|sc| {
        for _ in 0..nth {
            sc.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                if j >= jobs.len() { break; }
                let (vi, si) = jobs[j];
                let (_, tau, tm, s) = variants[vi];
                let a = variant(variants[vi].0, tau, tm, s);
                let (mut tfs, mut weaks, mut strongs) = (Vec::new(), Vec::new(), Vec::new());
                for i in 0..trials {
                    let (tf, w, st) = run(&a, tm, SPMS[si], seed.wrapping_add((j as u64) << 24).wrapping_add(i as u64));
                    tfs.push(tf as f64); weaks.push(w as f64); strongs.push(st as f64);
                }
                out.lock().unwrap().push((vi, si, median(tfs), median(weaks), median(strongs)));
                eprintln!("  {} spm{} done", variants[vi].0, SPMS[si]);
            });
        }
    });
    let mut raw = out.into_inner().unwrap();
    raw.sort_by_key(|(vi, si, ..)| (*vi, *si));

    println!("\n## EAGER-EASE DECOMPOSITION — upward(self-deepening) fires split by counterfactual 'would the reluctant boundary refuse?'");
    println!("WEAK-REFUSABLE = reluctant WOULD refuse (asymmetry does safety work; the BOUNDARY's role). STRONG-UNREFUSABLE = reluctant");
    println!("fires too (estimator presented an excursion reluctance can't refuse; the ESTIMATOR's role). Across rate, real-decline.\n");
    for (vi, label) in variants.iter().map(|v| v.0).enumerate() {
        println!("### {}", label);
        print!("| metric |"); for &s in SPMS { print!(" spm{} |", s as u32); } println!();
        print!("| --- |"); for _ in SPMS { print!(" --- |"); } println!();
        let row = |name: &str, pick: &dyn Fn(&(usize,usize,f64,f64,f64)) -> f64| {
            let mut s = format!("| {} |", name);
            for si in 0..SPMS.len() {
                let cell = raw.iter().find(|(v, sj, ..)| *v == vi && *sj == si).unwrap();
                s.push_str(&format!(" {:.0} |", pick(cell)));
            }
            println!("{}", s);
        };
        row("upward-fires (total)", &|c| c.2);
        row("  WEAK (asym would refuse)", &|c| c.3);
        row("  STRONG (estimator-driven)", &|c| c.4);
        println!();
    }
    println!("READ: complementarity (your story) needs boundary-jumpy's rise-with-rate in WEAK-REFUSABLE (the asymmetry doing");
    println!("rate-dependent safety work at dense rate) AND estimator-jumpy's sparse fires in STRONG-UNREFUSABLE (fast estimator");
    println!("presenting excursions reluctance can't refuse at sparse rate). If boundary-jumpy's rise is mostly STRONG, the estimator");
    println!("is the driver and the boundary-asymmetry isn't separately protecting ⇒ complementarity overstated, estimator dominant.");
    println!("Champion should be ~0 total (both mechanisms present). The verdict rides this decomposition, not the bare count.");
}
