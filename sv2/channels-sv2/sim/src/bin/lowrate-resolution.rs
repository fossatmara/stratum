//! LOW-RATE STUDY, STAGE 0 — threshold-grounding + resolution control, BEFORE the
//! full guard-regime escape sweep. Two checkable pieces, per the scope:
//!   (1) is the spiral threshold genuinely RE-DERIVED for the guard regime, or
//!       inherited from the dense-regime 30%?
//!   (2) can a depth-vs-threshold comparison DISTINGUISH known-spiral from
//!       known-safe at GUARD-REGIME share counts — or is the question sub-resolution
//!       by construction (outcome #3) before any real trace is read?
//! Both must hold before the full sweep is worth building; if (2) fails, the line
//! closes by un-measurability and the sweep is moot.
//!
//! ===========================================================================
//! PIECE 1 — THRESHOLD GROUNDING, AND THE DEGENERACY IT SURFACES (read before
//! trusting any guard-regime threshold). The dense regime's spiral floor was ~30%
//! (δ_clock=0.3, shallowest non-vacuous depth where the champion's escape stayed
//! safe). The scope says: do NOT inherit 30% — re-derive for the guard regime,
//! because the e^(−e) collapse may bite at a SHALLOWER depth when the baseline rate
//! is already sparse. Try to derive it from the information-floor detection-time
//! mechanism, the same machinery that grounded the trough-gate:
//!
//!   Under sustained over-difficulty e, the observed share rate collapses:
//!     r_obs = r* · e^(−e)              (THEORY §5.2; over-diff starves the stream)
//!   Detection time (floor-limited, resolve a deviation δ): N ≈ z²/δ² shares ⇒
//!     T_d(e, r*) = z²·e^(e) / (δ² · r*)    (e^(e) because r_obs carries the e^(−e))
//!
//!   ANCHOR (not inherit): the guard threshold e*(r*) is the depth at which the
//!   guard rate's detection time equals the dense rate's detection time at its 30%
//!   threshold — i.e. "equally hard to escape," through the mechanism:
//!     T_d(e*, r*) = T_d(0.30, r_dense)
//!     ⇒ e^(e*)/r* = e^(0.30)/r_dense
//!     ⇒ e* = 0.30 + ln(r* / r_dense)
//!
//!   THE DEGENERACY: for r* < r_dense, ln(r*/r_dense) < 0, so e* < 0.30 — and at
//!   the guard rates it goes NEGATIVE. spm2 vs dense spm30: e* = 0.30 + ln(2/30) =
//!   0.30 − 2.71 = −2.41. A negative spiral threshold says "ANY over-difficulty
//!   spirals at spm2" — which is FALSE (the guard regime demonstrably recovered:
//!   deploy-coupling-transient showed escape peaks then descends). So the
//!   floor-anchoring is WRONG, and its wrongness is informative:
//!
//!   WHY IT DEGENERATES (the real finding of piece 1). The anchoring assumed only
//!   DETECTION scales with 1/r*. But the over-difficulty's DEEPENING is driven by
//!   the controller's action through the SAME starved stream, so deepening ALSO
//!   scales ~1/r_obs. If BOTH detection and deepening scale together, their RATIO —
//!   which is what decides spiral-vs-recover — is rate-INDEPENDENT, and the
//!   threshold would be the SAME ~30% in both regimes (NOT shallower). What BREAKS
//!   that symmetry is that the guard regime runs a DIFFERENT controller (PoissonCI,
//!   confirmed boundary.rs:1023 — not the boundary detector), whose deepening/
//!   correcting dynamics do NOT necessarily scale like the floor. So the
//!   threshold's rate-dependence is NOT derivable from the information floor alone —
//!   it depends on PoissonCI's actual dynamics. CONCLUSION: the guard threshold
//!   cannot be cleanly DERIVED (floor gives a degenerate answer); it must be
//!   MEASURED from the guard controller's spiral behavior — and that measurement is
//!   itself subject to the sparse-regime resolution limit, which is piece 2. So the
//!   honest threshold grounding is "not floor-derivable, must be measured, and the
//!   measurement may not resolve" — which is already most of the way to outcome #3.
//!
//! PIECE 2 — THE RESOLUTION CONTROL (the decisive, cleanly-buildable check). Even
//! setting the threshold aside, ask the threshold-INDEPENDENT question: at
//! guard-regime share counts, do the SUSTAINED-DEPTH estimates of a known-SAFE
//! escape (+13%, like the dense regime) and a known-SPIRAL escape (+30%) SEPARATE,
//! or do their noise distributions OVERLAP? If they overlap, NO threshold placement
//! can tell them apart → sub-resolution by construction (outcome #3), and the full
//! sweep is moot.
//!   The estimator's scatter on e is floor-set: σ_e ≈ 100/√N_window %, where
//!   N_window = shares in the estimator window = r_obs · τ_window. At guard rates
//!   this is brutal: spm2, τ=360s, under +13% collapse → r_obs ≈ 2·e^(−0.13)/60 per
//!   sec → N_window ≈ 2·0.88·6 ≈ 10 shares → σ_e ≈ 100/√10 ≈ 32%. The safe-vs-
//!   spiral SEPARATION is 30−13 = 17%. σ (32%) > separation (17%) ⇒ the
//!   distributions overlap heavily ⇒ UNRESOLVABLE. This rig computes it exactly
//!   (real share counts, not this back-of-envelope) and Monte-Carlos the overlap.
//!
//! PRE-REGISTERED OUTCOMES (all three live; the 3rd is the one the dense-regime
//! "just read the depth" intuition would miss):
//!   A. distributions SEPARATE with margin ⇒ the depth comparison resolves at guard
//!      rates ⇒ the full sweep is worth building (it can give a real verdict).
//!   B. distributions TOUCH ⇒ borderline; the full sweep needs many trials + error
//!      bars but may resolve.
//!   C. distributions OVERLAP ⇒ SUB-RESOLUTION by construction ⇒ the low-rate
//!      question is unanswerable on this rig; the rate-aware line CLOSES by
//!      un-measurability (not "no value" — "no MEASURABLE value at achievable
//!      resolution"), the same sub-resolution wall as the high-rate p-value, now at
//!      low rate. The full sweep is MOOT — don't build it.
//!
//! Usage: cargo run --release --bin lowrate-resolution
//! ===========================================================================
//!
//! ===========================================================================
//! CORRECTION (read FIRST — piece 2's σ is REFUTED; piece 1 STANDS). Piece 2's
//! analytic σ_e ≈ 100/√N_window ≈ 32% (⇒ outcome C, sub-resolution) is the WRONG
//! QUANTITY and its verdict is overturned by measurement (lowrate-sigma.rs). The
//! lesson is TWO-SIDED — both the formula AND the naive fix are wrong, in opposite
//! directions:
//!   · analytic σ=32% is SINGLE-SNAPSHOT scatter (one tick's depth estimate). The
//!     resolution question is about the SUSTAINED depth = the MEAN of e over the
//!     escape's breached ticks. Snapshot scatter is too PESSIMISTIC — it ignores the
//!     averaging over the escape. (Would have CLOSED the line wrongly: sub-resolution
//!     when it resolves.)
//!   · the naive fix — σ_snapshot/√M_eff with M_eff = number of τ-windows — would be
//!     too OPTIMISTIC, because the windows are NOT independent (EWMA windows overlap
//!     and the estimator state is autocorrelated), so independent-averaging shrinks
//!     FASTER than the real autocorrelated path. (Would OPEN the line wrongly:
//!     resolves when it might be marginal.) NB: the verbal "√M_eff≈5" used to explain
//!     the gap is exactly this mirror error — fine as hindsight, WRONG if computed.
//!   · the RIGHT quantity (lowrate-sigma.rs): the ACROSS-TRIAL std of the per-trial
//!     sustained-depth, where each trial is a FULL SIMULATED ESCAPE TRAJECTORY — so
//!     the autocorrelation is in it BY CONSTRUCTION (no M_eff assumed; never
//!     decomposed into independent samples). MEASURED σ ≈ 6–7% at guard rates ⇒
//!     σ/separation ≈ 0.35–0.40 ⇒ outcome A: the depth comparison RESOLVES at guard
//!     share counts. The dense control resolves too (σ/sep=0.15). So the guard-regime
//!     question is NOT sub-resolution — it is ANSWERABLE, and the answer (the +22–25%
//!     sustained depth, spm2 tail reaching 30%) is outcome B: the line stays OPEN at
//!     low rate, pending the guard-regime threshold (piece 1).
//! PIECE 1 (the floor-anchored threshold is degenerate ⇒ the guard threshold must be
//! MEASURED from PoissonCI's dynamics, not inherited) STANDS, and is now MORE
//! load-bearing: lowrate-sigma measured the depth distribution at +25±7%, whose tail
//! reaches the INHERITED 30%; if PoissonCI's true guard threshold is BELOW 30% (piece
//! 1's prediction — collapse bites shallower at sparse baseline rate), the MEAN
//! reaches it, strengthening the bind from tail (spm2 only) to mean (spm2 & spm4).
//! The guard-regime threshold study is the TERMINUS that decides low-rate value.
//! KEEP this file: the two-sided σ lesson (snapshot too-pessimistic / naive-averaged
//! too-optimistic / full-trajectory-across-trial is the statistic) is the durable
//! method point, more instructive than the one-sided "trust the measurement."
//! ===========================================================================

const Z2: f64 = 3.84; // z² at 95%
const DENSE_RATE_SPM: f64 = 30.0;
const DENSE_THRESHOLD: f64 = 0.30; // δ_clock=0.3, the dense-regime spiral floor
const GUARD_SPMS: &[f64] = &[2.0, 4.0];
const TAU_WINDOW_SECS: f64 = 360.0; // champion estimator window
const SAFE_DEPTH: f64 = 13.0;   // % — known-safe sustained over-difficulty (dense-regime value)
const SPIRAL_DEPTH: f64 = 30.0; // % — known-spiral sustained depth (at the floor)

/// floor-anchored guard threshold (the DEGENERATE derivation — printed to show it).
fn anchored_threshold(r_spm: f64) -> f64 {
    DENSE_THRESHOLD + (r_spm / DENSE_RATE_SPM).ln()
}

/// shares in the estimator window at rate r_spm under sustained over-difficulty
/// depth e_pct (collapse r_obs = r*·e^(−e)).
fn window_shares(r_spm: f64, e_pct: f64) -> f64 {
    let e = e_pct / 100.0;
    let r_obs_per_sec = r_spm * (-e).exp() / 60.0;
    r_obs_per_sec * TAU_WINDOW_SECS
}

/// floor scatter on the e estimate: σ_e ≈ 100/√N_window %.
fn sigma_e(r_spm: f64, e_pct: f64) -> f64 {
    let n = window_shares(r_spm, e_pct);
    if n <= 0.0 { f64::INFINITY } else { 100.0 / n.sqrt() }
}

/// deterministic normal-overlap proxy: fraction of two equal-σ Gaussians (means
/// m1,m2) that overlap = 2·Φ(−|m1−m2|/(2σ)). Using erf via a rational approx.
fn erf(x: f64) -> f64 {
    // Abramowitz-Stegun 7.1.26
    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
        + 0.254829592) * t * (-x * x).exp();
    if x >= 0.0 { y } else { -y }
}
fn phi(z: f64) -> f64 { 0.5 * (1.0 + erf(z / 2f64.sqrt())) }

/// overlapping coefficient of two equal-σ Gaussians separated by |Δ|: 2·Φ(−Δ/2σ).
fn overlap_coeff(sep: f64, sigma: f64) -> f64 {
    if sigma <= 0.0 { return 0.0; }
    2.0 * phi(-(sep.abs()) / (2.0 * sigma))
}

/// Monte-Carlo confirmation: draw N sustained-depth estimates for safe & spiral
/// (each ~ Normal(true, σ)), report the fraction misclassified by the optimal
/// midpoint cut. Deterministic LCG seeded per call (no Date/rand).
fn mc_misclass(sep_true_lo: f64, sep_true_hi: f64, sigma: f64, n: usize, mut seed: u64) -> f64 {
    let cut = 0.5 * (sep_true_lo + sep_true_hi);
    let mut wrong = 0usize;
    // Box-Muller from an LCG.
    let mut next = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 11) as f64 / (1u64 << 53) as f64 };
    for _ in 0..n {
        let (u1, u2): (f64, f64) = (next().max(1e-12), next());
        let g = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        // one safe draw, one spiral draw
        let safe = sep_true_lo + sigma * g;
        let (u3, u4): (f64, f64) = (next().max(1e-12), next());
        let g2 = (-2.0 * u3.ln()).sqrt() * (2.0 * std::f64::consts::PI * u4).cos();
        let spiral = sep_true_hi + sigma * g2;
        if safe > cut { wrong += 1; }      // safe misread as spiral
        if spiral <= cut { wrong += 1; }   // spiral misread as safe
    }
    wrong as f64 / (2 * n) as f64
}

fn main() {
    println!("# LOW-RATE STUDY, STAGE 0 — threshold grounding + resolution control\n");

    // --- PIECE 1: the degenerate floor-anchoring (shown, not trusted) ---
    println!("## PIECE 1 — floor-anchored guard threshold e*(r*) = 0.30 + ln(r*/{}) [DEGENERATE — see header]", DENSE_RATE_SPM as u32);
    println!("| spm | floor-anchored e* | sane? |");
    println!("| --- | --- | --- |");
    for &r in &[DENSE_RATE_SPM, 12.0, 6.0, 4.0, 2.0] {
        let e = anchored_threshold(r);
        println!("| {} | {:+.2} | {} |", r as u32, e, if e > 0.0 { "ok" } else { "**NEGATIVE — degenerate**" });
    }
    println!("\nREAD: e* goes NEGATIVE at guard rates ⇒ floor-anchoring says 'any over-difficulty spirals at spm≤4', which is FALSE");
    println!("(guard regime demonstrably recovers). The threshold is NOT floor-derivable — both detection AND deepening scale ~1/r_obs,");
    println!("so their ratio is rate-independent UNLESS the controller differs — and the guard controller IS different (PoissonCI, not");
    println!("the boundary detector). So the threshold depends on PoissonCI's dynamics and must be MEASURED, not derived. → piece 2.\n");

    // --- PIECE 2: the resolution control ---
    println!("## PIECE 2 — RESOLUTION CONTROL: can sustained-depth distinguish safe (+{}%) from spiral (+{}%) at guard share counts?",
        SAFE_DEPTH as u32, SPIRAL_DEPTH as u32);
    println!("σ_e = 100/√(N_window), N_window = r*·e^(−e)·τ. Separation = {}−{} = {}%. Overlap = 2·Φ(−sep/2σ); MC misclass at midpoint cut.\n",
        SPIRAL_DEPTH as u32, SAFE_DEPTH as u32, (SPIRAL_DEPTH - SAFE_DEPTH) as u32);
    let sep = SPIRAL_DEPTH - SAFE_DEPTH;
    println!("| spm | N_window @safe | σ_e @safe% | N_window @spiral | σ_e @spiral% | σ (avg)% | overlap coeff | MC misclass% | verdict |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for &r in GUARD_SPMS {
        let (ns, np) = (window_shares(r, SAFE_DEPTH), window_shares(r, SPIRAL_DEPTH));
        let (ss, sp) = (sigma_e(r, SAFE_DEPTH), sigma_e(r, SPIRAL_DEPTH));
        let savg = 0.5 * (ss + sp);
        let ov = overlap_coeff(sep, savg);
        let mc = mc_misclass(SAFE_DEPTH, SPIRAL_DEPTH, savg, 200_000, 0xABCDEF ^ (r as u64));
        let verdict = if ov < 0.10 { "A: SEPARATE" } else if ov < 0.32 { "B: TOUCH" } else { "C: OVERLAP (sub-res)" };
        println!("| {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.2} | {:.0} | {} |",
            r as u32, ns, ss, np, sp, savg, ov, mc * 100.0, verdict);
    }
    // dense-regime reference: the same check where we KNOW it resolved (sanity).
    let savg_dense = 0.5 * (sigma_e(DENSE_RATE_SPM, SAFE_DEPTH) + sigma_e(DENSE_RATE_SPM, SPIRAL_DEPTH));
    println!("| {} (dense ref) | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.2} | {:.0} | {} |",
        DENSE_RATE_SPM as u32,
        window_shares(DENSE_RATE_SPM, SAFE_DEPTH), sigma_e(DENSE_RATE_SPM, SAFE_DEPTH),
        window_shares(DENSE_RATE_SPM, SPIRAL_DEPTH), sigma_e(DENSE_RATE_SPM, SPIRAL_DEPTH),
        savg_dense, overlap_coeff(sep, savg_dense),
        mc_misclass(SAFE_DEPTH, SPIRAL_DEPTH, savg_dense, 200_000, 0x123456) * 100.0,
        { let ov = overlap_coeff(sep, savg_dense); if ov < 0.10 { "A: SEPARATE" } else if ov < 0.32 { "B: TOUCH" } else { "C: OVERLAP" } });

    println!("\nREAD: if the guard rows are 'C: OVERLAP', the safe/spiral sustained-depth distributions cannot be separated at guard share");
    println!("counts ⇒ outcome #3 (sub-resolution) BY CONSTRUCTION ⇒ the full low-rate sweep is MOOT (it would over-read a point estimate");
    println!("whose noise straddles the threshold). The dense-ref row should resolve (A/B) — it's the regime where the depth-read WORKED.");
    println!("That contrast IS the calibration control: same method, resolves dense, fails guard ⇒ the line closes by un-measurability.");
}
