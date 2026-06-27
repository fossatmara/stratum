//! GUARD-THRESHOLD STUDY, STAGE 0 — is the spiral threshold a well-defined,
//! NON-CIRCULAR, INDEPENDENT object, and does the depth-vs-threshold COMPARISON
//! resolve? Probe BEFORE building the full study, same as lowrate stage-0.
//!
//! ===========================================================================
//! THE CIRCULARITY CHECK (the load-bearing scoping decision). The terminus compares
//! the guard escape's sustained depth (+25±7%, measured in lowrate-sigma.rs) against
//! PoissonCI's spiral threshold. Both are the guard controller's behavior, so the
//! threshold MUST be derived from something INDEPENDENT of the +25±7% escapes, or
//! "does depth exceed threshold" is partly tautological (same shape as the W-in-τ
//! circularity caught earlier).
//!
//! WHAT THE THRESHOLD ACTUALLY IS (corrected against source). PoissonCI is NOT a
//! correction-rate — it is a FIRE TRIGGER: threshold() (boundary.rs:160) returns the
//! DEVIATION e must exceed before a retarget FIRES. The guard regime differs from
//! dense in the BOUNDARY ONLY (PoissonCI vs SignPersist); estimator (Ewma360) and
//! update rule (AccelPartialRetarget) are identical (composed.rs:262). So there is
//! no "PoissonCI gain curve" — the independent object is PoissonCI's OWN FIRE-
//! THRESHOLD FORMULA evaluated against the collapse:
//!     required_dev(e) = PoissonCI.threshold(dt, r·e^(−e), ·)   [boundary.rs:168]
//!                     = ((z·√λ̄ + 0.5)/λ̄ + margin)·100,  λ̄ = r·e^(−e)/60·dt
//! Under over-difficulty the stream collapses (r_obs=r·e^(−e)), λ̄ drops, and the
//! fire-threshold WIDENS (the z·√λ̄/λ̄ ~ 1/√λ̄ term blows up) — PoissonCI's documented
//! low-rate conservatism (boundary.rs:138). The SPIRAL THRESHOLD e* is the FIXED
//! POINT where required_dev(e*) = e*: below it the excursion exceeds what's needed to
//! fire (controller fires, corrects); above it the fire-threshold has OUTRUN the
//! excursion (it won't fire, the excursion is free to deepen — self-reinforcing).
//! This is a CLOSED-FORM curve crossing — ZERO escape trajectories — so it is
//! independent of +25±7% BY CONSTRUCTION. (Non-circular: required_dev comes from the
//! threshold formula, the depth from the escapes; different measurements.)
//!
//! THE COMPARISON-RESOLUTION CHECK (the one-level-up sub-resolution risk). The depth
//! RESOLVES (σ≈6-7%, lowrate-sigma). But the VERDICT is depth-vs-threshold, and the
//! threshold has its OWN uncertainty (z and margin are fixed, but dt and the effective
//! r entering λ̄ carry sparse-regime noise). Differencing two numbers COMPOUNDS their
//! bars: +25±7 vs e*±σ_thr resolves only if the distributions SEPARATE. Pre-register
//! THREE outcomes: depth clearly ABOVE e* (binds → line open, p-pinning next), clearly
//! BELOW (collapses → line closes), OVERLAP (sub-resolution at the COMPARISON level →
//! closes by un-measurability). The third is live — differencing compounds bars.
//!
//! THIS PROBE: (1) compute required_dev(e) closed-form at guard rates, find the e*
//! crossing, confirm it EXISTS and is well-defined (a real fixed point, not absent or
//! multi-valued); (2) report e* and bracket its sensitivity to dt and r (its bars);
//! (3) check whether e*±bars SEPARATES from +25±7% — if it overlaps, the comparison is
//! sub-resolution and the full study is moot. Same build-the-cheap-check-first as the
//! lowrate stage-0 that mooted the full sweep.
//!
//! Usage: cargo run --release --bin guard-threshold-probe
//! ===========================================================================

// PoissonCI threshold, VERBATIM from boundary.rs:160-171 (z, margin, λ̄ path).
fn poisson_required_dev_pct(z: f64, margin: f64, shares_per_min: f64, dt_secs: f64) -> f64 {
    let lambda_bar = (shares_per_min / 60.0) * dt_secs;
    if lambda_bar <= 0.0 { return 100.0; }
    let bound_fraction = (z * lambda_bar.sqrt() + 0.5) / lambda_bar + margin;
    bound_fraction * 100.0
}

// guard PoissonCI params: default_parametric (z=2.576, margin=0.05) — boundary.rs:127,
// the fallback the guard uses (AdaptiveSignPersist's low-spm arm).
const Z: f64 = 2.576;
const MARGIN: f64 = 0.05;
const GUARD_SPMS: &[f64] = &[2.0, 4.0];
const TICK_DT: f64 = 60.0; // champion tick_secs
const DEPTH_MEAN: f64 = 25.0; // measured guard sustained depth (lowrate-sigma)
const DEPTH_SIGMA: f64 = 7.0; // measured across-trial σ

/// required_dev(e): the deviation PoissonCI needs to FIRE, at depth e (stream
/// collapsed to r·e^(−e)). e in %, returns %.
fn required_dev(spm: f64, e_pct: f64, dt: f64) -> f64 {
    let r_obs = spm * (-e_pct / 100.0).exp();
    poisson_required_dev_pct(Z, MARGIN, r_obs, dt)
}

/// find the fixed point required_dev(e*) = e* by scanning e upward; the spiral
/// threshold is the lowest e where required_dev(e) <= e (excursion meets/exceeds the
/// fire requirement is the SAFE side; the crossing from required>e to required<=e is
/// e*). Returns None if no crossing in [0,100] (never fires in range = always-spiral,
/// or always fires = no spiral threshold in range).
fn fixed_point(spm: f64, dt: f64) -> Option<f64> {
    let mut prev_gap = required_dev(spm, 0.0, dt) - 0.0; // required - e at e=0
    let mut e = 0.0;
    while e <= 100.0 {
        let gap = required_dev(spm, e, dt) - e;
        if prev_gap > 0.0 && gap <= 0.0 {
            // crossing between e-step and e: required_dev fell to meet e (linear interp)
            let step = 0.25;
            let frac = prev_gap / (prev_gap - gap);
            return Some(e - step + frac * step);
        }
        prev_gap = gap;
        e += 0.25;
    }
    None
}

fn main() {
    println!("# GUARD-THRESHOLD STAGE 0 — does PoissonCI's spiral threshold exist, and does the comparison resolve?\n");
    println!("PoissonCI fire-threshold required_dev(e) = ((z√λ̄+0.5)/λ̄ + margin)·100, λ̄ = r·e^(−e)/60·dt, z={}, margin={}.", Z, MARGIN);
    println!("Spiral threshold e* = fixed point where required_dev(e*)=e* (below: fires+corrects; above: fire-threshold outruns excursion).\n");

    // PIECE 1: does e* exist, and the required_dev(e) curve shape.
    println!("## required_dev(e) at guard rates — find e* (the fixed point), confirm it's well-defined");
    println!("| spm | req_dev(0%) | req_dev(10%) | req_dev(25%) | req_dev(40%) | e* (fixed pt) | well-defined? |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for &spm in GUARD_SPMS {
        let fp = fixed_point(spm, TICK_DT);
        println!("| {} | {:.0} | {:.0} | {:.0} | {:.0} | {} | {} |",
            spm as u32,
            required_dev(spm, 0.0, TICK_DT), required_dev(spm, 10.0, TICK_DT),
            required_dev(spm, 25.0, TICK_DT), required_dev(spm, 40.0, TICK_DT),
            fp.map(|e| format!("{:.0}%", e)).unwrap_or_else(|| "NONE in [0,100]".into()),
            match fp { Some(_) => "yes (single crossing)", None => "**no crossing — see note**" });
    }

    // PIECE 2: e* uncertainty — bracket over plausible dt (the sparse-regime noise in
    // the threshold). dt varies because fires reset the window; bracket dt∈[30,120].
    println!("\n## e* sensitivity to dt (the threshold's OWN bars — dt carries sparse-regime noise)");
    println!("| spm | e*(dt=30) | e*(dt=60) | e*(dt=120) | e* spread (bars) |");
    println!("| --- | --- | --- | --- | --- |");
    for &spm in GUARD_SPMS {
        let (a, b, c) = (fixed_point(spm, 30.0), fixed_point(spm, 60.0), fixed_point(spm, 120.0));
        let vals: Vec<f64> = [a, b, c].iter().filter_map(|x| *x).collect();
        let spread = if vals.len() >= 2 {
            vals.iter().cloned().fold(f64::MIN, f64::max) - vals.iter().cloned().fold(f64::MAX, f64::min)
        } else { f64::NAN };
        println!("| {} | {} | {} | {} | {} |", spm as u32,
            a.map(|e| format!("{:.0}%", e)).unwrap_or_else(|| "—".into()),
            b.map(|e| format!("{:.0}%", e)).unwrap_or_else(|| "—".into()),
            c.map(|e| format!("{:.0}%", e)).unwrap_or_else(|| "—".into()),
            if spread.is_finite() { format!("±{:.0}%", spread / 2.0) } else { "n/a".into() });
    }

    // PIECE 3: the COMPARISON-resolution check — does e*±bars SEPARATE from depth +25±7?
    println!("\n## COMPARISON RESOLUTION — does e* (±its bars) SEPARATE from the depth distribution (+{}±{}%)?", DEPTH_MEAN as u32, DEPTH_SIGMA as u32);
    println!("Verdict needs separation; differencing compounds bars. depth ABOVE e* ⇒ binds (open); BELOW ⇒ collapses; OVERLAP ⇒ sub-res.\n");
    println!("| spm | e* (dt=60) | e* bars (dt spread) | depth | gap (depth − e*) | separates? | verdict |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for &spm in GUARD_SPMS {
        let estar = fixed_point(spm, TICK_DT);
        let (a, c) = (fixed_point(spm, 30.0), fixed_point(spm, 120.0));
        let thr_bar = match (a, c) { (Some(x), Some(y)) => (x - y).abs() / 2.0, _ => f64::NAN };
        match estar {
            Some(e) => {
                let gap = DEPTH_MEAN - e;
                // separation: |gap| vs combined bar √(σ_depth² + σ_thr²)
                let combined = (DEPTH_SIGMA * DEPTH_SIGMA + thr_bar * thr_bar).sqrt();
                let separates = gap.abs() > 2.0 * combined; // ~2σ separation
                let verdict = if !separates { "C: OVERLAP (sub-res at comparison)" }
                              else if gap > 0.0 { "A: depth ABOVE e* — BINDS (line open)" }
                              else { "B: depth BELOW e* — collapses (line closes)" };
                println!("| {} | {:.0}% | ±{:.0}% | +{:.0}±{:.0}% | {:+.0}% | {} | {} |",
                    spm as u32, e, thr_bar, DEPTH_MEAN, DEPTH_SIGMA, gap,
                    if separates {"yes"} else {"**NO**"}, verdict);
            }
            None => println!("| {} | NONE | — | +{:.0}±{:.0}% | — | — | (no fixed pt — see piece-1 note) |",
                spm as u32, DEPTH_MEAN, DEPTH_SIGMA),
        }
    }
    println!("\nREAD: if e* exists, is well-defined, and SEPARATES from +25±7% — the full study is worth building and the SIGN of the gap");
    println!("is the verdict (depth above e* = binds = line open). If e* doesn't exist (no crossing) the mechanism model is wrong (rethink).");
    println!("If e*±bars OVERLAPS +25±7% — the comparison is SUB-RESOLUTION (differencing compounded the bars), line closes by un-");
    println!("measurability at the comparison level, full study MOOT. This is the non-circular threshold (req_dev formula, not escapes)");
    println!("crossed with the measured depth — the independence is by construction (different measurements), per the scoping.");
}
