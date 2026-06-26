//! SHARE-INDEXED estimator — the rate-aware window, measured as a TRADEOFF.
//!
//! ===========================================================================
//! PRE-REGISTRATION (written BEFORE any number; the discipline that carried the
//! whole vardiff arc — lock the prediction, name it a tradeoff not a payoff).
//! ===========================================================================
//!
//! THE QUESTION. The fixed champion (Ewma360, TIME-indexed: α=exp(−tick/τ)) is
//! the gentlest decline-safe FIXED window under minimax-over-rate. tau-family.rs
//! measured that the per-rate over-difficulty optimum SLIDES (τ*∝1/r): a fixed
//! window is off-valley at the band edges. A SHARE-INDEXED window — decay per
//! share-batch, span counted in SHARES not seconds — is rate-aware BY
//! CONSTRUCTION and should track the per-rate valley floor. This binary measures
//! what that tracking costs and whether it stays admissible. It does NOT, and
//! cannot, decide whether share-indexing is "better" — see SCOPE.
//!
//! WHY SHARE-INDEXED, NOT τ-SCHEDULED (the build choice, and it is not taste).
//! The lazy alternative — make τ a function of measured rate, `τ_eff = k/r*` —
//! reintroduces the operand problem the arc kept hitting: `r*` there is the
//! NOISY share-derived estimate, so the time constant depends on the very
//! quantity it is smoothing (a rate-estimate → τ → rate-estimate FEEDBACK path
//! with its own stability question), and it is noisiest at LOW rate — exactly
//! the regime the feature targets. Share-indexing has no such loop: counting
//! shares instead of seconds makes the window rate-aware without ever computing
//! an `r*` to schedule on. So we build the share-indexed estimator (a new
//! `Estimator` impl alongside EwmaEstimator — NO trait change, NO channel
//! change), not the τ-schedule. The confound-avoidance is the reason, not
//! elegance.
//!
//! SCOPE — WHAT THIS SETTLES AND WHAT IT STRUCTURALLY CANNOT.
//!   (a) OVER-DIFFICULTY TRACKING — a CONSTRUCTION CHECK, not a finding. A
//!       constant-shares window tracks the per-rate optimum BY DEFINITION
//!       (that is what share-indexing IS), and tau-family already measured the
//!       optimum slides τ*∝1/r. So "does share-indexed track the sliding
//!       optimum" is near-vacuous as an empirical test — it passes unless the
//!       estimator is BUGGY. Relabel honestly: this verifies the code realizes
//!       the intended behavior (build-verification); it does NOT establish a
//!       result with a real chance of failing. If it does NOT track, that is a
//!       BUG, not a finding. The sim's real empirical content is NOT here.
//!   (b) ADMISSIBILITY of the JUMPY regime — the real test, and clean ONLY if the
//!       envelope stresses the RIGHT failure mode. The decline-safety gate is
//!       binary/one-sided (catches over-difficulty, not wobble), so a jumpy
//!       window passes it while being wobble-worse — fine, that is the tradeoff.
//!       BUT the fixed champion's risk is SLEEPY LAG; share-indexing's risk is
//!       JUMPY OVERSHOOT — a DIFFERENT failure mode. CONFIRMED AGAINST SOURCE:
//!       `slow-decline.rs` builds only `Phase::Hold` segments (mature → stepped
//!       declines → floor) and gates on SETTLED-e after recovery — it stresses
//!       sleepy lag and AVERAGES OUT the transient trough where a jumpy window is
//!       fragile. So passing THAT envelope is passing the cells that stress the
//!       OLD window's weakness, SILENT on the new one's. REQUIREMENT (locked):
//!       the sweep MUST add a scenario that stresses jumpy overshoot — a rate
//!       SPIKE / rapid increase or a recovery TRANSIENT — gated on the transient
//!       TROUGH (or peak over-difficulty during the transient), NOT only settled-e.
//!       Without that, P3's "admissible at every cell" is scoped to the wrong
//!       failure mode and is not a real pass. This is the load-bearing addition.
//!
//!       TROUGH-GATE DEFINITION (pin BEFORE numbers — the symmetric trap). "Gate
//!       on the trough" has a free parameter the settled-e gate didn't: the
//!       WINDOW over which the trough is measured. Too short → catches single-tick
//!       Poisson noise (every jumpy window "fails" on one unlucky sample — false
//!       fail); too long → re-averages the trough back toward settled-e and
//!       misses the transient again (the original disease). So the trough gate
//!       must pre-register: (window) the worst over-difficulty `e` SUSTAINED over
//!       a rolling W-minute window (W tied to operational meaning, NOT to what
//!       makes the comparison clean — e.g. the duration of over-difficulty that
//!       actually risks the starvation spiral or materially starves the miner;
//!       the settled-e gate's own +5%/120-min recovery is the precedent to set W
//!       and the threshold commensurately); (threshold) the over-difficulty `e`
//!       level that counts as a breach, chosen so the gate expresses the SAME
//!       admissibility criterion as the settled-e gate (don't sit over-difficulty
//!       dangerously), measured for the transient rather than the steady state —
//!       NOT a stricter or looser bar. An uncalibrated W or threshold makes the
//!       pass/fail an artifact of the pick, as arbitrary as a fabricated heatmap.
//!
//!       CHAMPION AS BASELINE ON THE NEW SCENARIO (the control — non-optional).
//!       The fixed champion cleared the SETTLED-e gate; it was NEVER tested for
//!       transient overshoot. So run the CHAMPION through the new jumpy-overshoot
//!       stressor too, as the baseline arm. Three outcomes, only interpretable
//!       WITH the control: (champ PASS, SI FAIL) = real finding, SI introduces a
//!       transient breach the champion doesn't; (both PASS) = jumpy risk bounded,
//!       the tradeoff is purely the wobble-magnitude one; (both FAIL) = the
//!       trough-gate is TOO STRICT (it fails a config already shipped) → recalibrate
//!       W/threshold, do not report SI as inadmissible. Share-indexing's pass/fail
//!       on the new gate is UNINTERPRETABLE without the champion's pass/fail on
//!       the SAME gate — that is the bar-calibration control, same "verify against
//!       the right baseline" move as the rest of the arc.
//!   REAL EMPIRICAL CONTENT of the sim, stated precisely: (i) the MAGNITUDE of the
//!   wobble cost across the band (the tradeoff curve's y-axis), and (ii) whether
//!   the jumpy regime clears a gate whose envelope ACTUALLY stresses jumpy
//!   overshoot. Not (a) (construction), and not (c) below.
//!   NOT settleable here — and not for want of a better rig:
//!     (c) NET SUPERIORITY ("is it the better controller"). Share-indexing tracks
//!         the per-rate OVER-DIFFICULTY optimum, which at high rate IS the short,
//!         jumpy window tau-family-safety.rs already flagged at 2–3× the
//!         champion's WOBBLE. So the predicted result is the SAME two-primitive
//!         tradeoff, now spread across the band: share-indexing rebalances toward
//!         over-difficulty at every rate, PAYING wobble for it. Whether that net
//!         trade is "better" bottoms out in the over-diff/wobble scalar weighting
//!         the project DELIBERATELY refuses to fix (constraint-not-cost, §9.3) —
//!         the same place the champion's selection bottomed out in a tie-break,
//!         not an optimum. So the honest OUTPUT is a measured TRADEOFF CURVE
//!         (over-difficulty recovered, wobble paid, both inside the gate), NOT a
//!         payoff and NOT a verdict. Calling it a "payoff" is the one-axis
//!         overclaim the tau_tradeoff caption took four passes to scrub out;
//!         do not reintroduce it here.
//!
//! PRE-REGISTERED PREDICTIONS (each falsifiable; locked before numbers):
//!   P1 (CONSTRUCTION CHECK, not a finding — see SCOPE(a)): across the band,
//!      share-indexed's worst over-difficulty area is ≤ fixed-360's at the band
//!      EDGES. This passes unless the estimator is buggy; a FAIL here is a BUG
//!      (the optimum-tracker fails to track), not an empirical result. Kept as a
//!      build-verification, NOT counted as a settled finding.
//!   P2 (the cost — a MEASURED MAGNITUDE, the real output): share-indexed's
//!      wobble is HIGHER than fixed-360's where it tracks a shorter effective
//!      window (high spm), buying over-difficulty with wobble — the same trade as
//!      τ=30 in tau-family-safety. The DELIVERABLE is the magnitude of that
//!      wobble cost per rate (the tradeoff curve's y-axis), not a pass/fail.
//!      FAILS IF: it lowers over-difficulty WITHOUT raising wobble — a free lunch
//!      the two-primitive structure says doesn't exist (would need explaining,
//!      not celebrating).
//!   P3 (admissibility — the binary gate, AGAINST A JUMPY-OVERSHOOT ENVELOPE):
//!      share-indexed clears the decline-safety gate at EVERY rate×spm cell, ON A
//!      SWEEP THAT INCLUDES the jumpy-overshoot stressor (rate spike / recovery
//!      transient, gated on the transient trough) — NOT only the sleepy-lag
//!      decline `slow-decline.rs` currently sweeps. FAILS IF: any cell breaches —
//!      the rate-aware window is INADMISSIBLE somewhere fixed-360 was safe, and
//!      the feature is gated out regardless of over-difficulty tracking
//!      (admissibility is necessary, §9.2). A pass on the SLEEPY-LAG-ONLY
//!      envelope does NOT satisfy P3 — that would be scoped to the wrong failure
//!      mode (SCOPE(b)). The gate's trough WINDOW+THRESHOLD must be pre-pinned and
//!      the CHAMPION run through the SAME stressor as the baseline (SCOPE(b)); a
//!      breach is only a finding if the champion CLEARS it (else the gate is too
//!      strict, recalibrate). This is the load-bearing test.
//!   P4 (the EXPECTED SHAPE — a reporting commitment, NOT a weighting): the
//!      result is a RATE-STRUCTURED tradeoff — FAVORABLE at low rate (over-diff
//!      stakes high, wobble already low) and UNFAVORABLE at high rate (over-diff
//!      already small, wobble cost 2–3×), net value UNASSERTED throughout. This
//!      is the inverted-attractiveness point from §8.3 (the trade is worst where
//!      the over-diff RATIO is largest). Pre-committing this shape locks against
//!      reading a clean LOW-rate number post-hoc as a GLOBAL win. It does NOT fix
//!      the weighting (still refused); it fixes that the CONCLUSION reports the
//!      tradeoff's rate-structure, which the prior already predicts. FAILS IF:
//!      the tradeoff is NOT rate-structured (e.g. uniformly favorable) — which
//!      would contradict the §8.3 inverted-ratio finding and need reconciling.
//!   P4b (the NOT-WORTH-BUILDING-ANYWHERE outcome — pre-registered as a REAL
//!      possible result, not a failure to explain away). The favorable regime is
//!      LOW rate — but the per-rate slide there is only 360→~240 (spm 2),
//!      MODEST; the DRAMATIC slide (360→30) is at HIGH rate, exactly the
//!      UNFAVORABLE regime (jumpy, wobble-costly, small absolute stakes). So the
//!      live possibility, pre-registered: share-indexing's BIG over-difficulty
//!      recoveries are all where the trade is bad, and its favorable-regime
//!      (low-rate) recovery is MARGINAL — in which case the honest conclusion is
//!      "not worth the wobble cost ANYWHERE: big wins in the unfavorable regime,
//!      marginal wins in the favorable one." The question P4 must FORCE is not
//!      just "is the low-rate trade favorable in DIRECTION" (yes, by the
//!      asymmetry) but "is the low-rate over-difficulty recovery LARGE ENOUGH to
//!      be worth ANY wobble at all" — and 240-vs-360 modesty says that is
//!      genuinely open, possibly NEGATIVE. A clean "tracks the optimum + stays
//!      admissible" result must NOT be read as "worth building" if the
//!      recovery-where-favorable is marginal. Pre-registering P4b stops that read.
//!   NOTE: there is NO "share-indexed wins" prediction, by construction — net
//!   value is the unfixed weighting. P1 is a build-check; P2's magnitude + P4's
//!   rate-shape ARE the tradeoff; P3 is whether the tradeoff is even on the table
//!   (admissible against the RIGHT failure mode) at all.
//!
//! HARDWARE CAVEAT (record with the result, do not soften). Even a clean sim
//! here is the CEILING of what is knowable without a different deployment — and
//! plausibly the ceiling FULL STOP. The share-indexing payoff is a SECOND-ORDER
//! effect: the DIFFERENCE between two windows that BOTH sit on the 1/√(r*τ) floor.
//! The fixed champion's FIRST-order hardware claims (settled offset, lever
//! √-scaling) already came back sim-only — below significance at the converged
//! sample counts this topology produced. A second-order effect needs MORE power
//! to resolve a SMALLER difference, on a deployment that couldn't resolve the
//! larger ones, AND needs per-device multi-rate channels the translator-aggregate
//! topology does not provide (the per-device-carriage gap, cf. the telemetry-hint
//! live-validation block). So the honest status of any result here is
//! "sim-validated; hardware-pending AND possibly below hardware resolution in
//! principle" — not a waystation to hardware truth, but likely its ceiling.
//!
//! Usage: cargo run --release --bin tau-share-indexed
//! Env: VARDIFF_SI_TRIALS (default 80 base, CI-scaled), VARDIFF_SI_OUT.
//!
//! STATUS: SPEC ONLY — the ShareIndexedEstimator and the sweep are not yet
//! written. This header locks the framing (tradeoff not payoff; P1/P2/P3/P4/P4b;
//! admissibility is the gate, wobble is a measured output not a verdict) before
//! any number exists. Implement against this; do not relax it to a "payoff."
//!
//! THE HEADLINE (the answer to "how hard to build and settle"): the build is
//! HOURS (estimator) plus a DAY-PLUS (the jumpy-overshoot scenario + its
//! trough-gate calibrated against the champion baseline — that calibration is
//! where the last mis-scope would hide, not a separable extra). But "SETTLE" in
//! the sense of "is rate-aware the BETTER vardiff" is NOT REACHABLE — not for
//! want of effort, but because the question bottoms out in (i) the over-diff/
//! wobble weighting the project refuses to fix, and (ii) a SECOND-ORDER effect
//! the deployment may be unable to resolve. The feature can be promoted to
//! "sim-validated, with a measured rate-structured TRADEOFF, admissibility
//! cleared against the right failure mode" — i.e. CHARACTERIZED — and no
//! further. That ceiling is a property of the QUESTION and the DEPLOYMENT, not
//! the effort: it is a day from CHARACTERIZED, never a day from SETTLED.

fn main() {
    eprintln!("tau-share-indexed: SPEC ONLY — pre-registration locked, estimator + sweep not yet built.");
    eprintln!("See the module header for the tradeoff framing and P1/P2/P3 before implementing.");
}
