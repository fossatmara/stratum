# Decline-safety: does one dimensionless group govern the floor? — specification

**Purpose.** Adjudicate, against the live harness, the one claim in the
"information phase transition" note that is *not* already settled in
`METRIC_DERIVATION.md` / `THEORY.md`: that the decline behavior of the controller
field organizes around a single dimensionless ratio. Two of the note's three
load-bearing pieces are already closed (§0); this spec isolates the third — the
*sustained-decline* floor — frames it as the experiment it actually is, and pins
pass/fail to a **predicted curve** (self-consistent, convex), not an eyeball fit.

The headline correction over the note (and over an earlier draft of this spec):
the collapse onto a single group is **real but arm-dependent**, and the
interesting result is not whether the field collapses but **which group each
arm-class obeys**, because that ordering *is* the floor-vs-gap decomposition.

## 0. What is already settled — do not re-run this ground

The note frames two hypotheses (control-instability vs information-threshold) as
competitors and offers a similarity collapse + estimator-irrelevance as the
discriminators. Three of those pieces are already in the tree:

- **The information floor is derived, not analogized.** Theorem 2
  (`METRIC_DERIVATION.md` §3): `Var(ê) ≥ 1/(r*τ)`, Cramér–Rao, for any *unbiased*
  estimator. The *dynamic* (declining-load) version is also derived:
  `L(τ_eff) ≈ ρ·τ_eff + z/√(r*·τ_eff)`, floored at
  `τ_eff ∝ (z/ρ)^{2/3}·r*^{−1/3}`. And the detection-time floor
  `T_d(δ) ≈ 2·log(1/α)/(r*·δ²)` (`THEORY.md` eq. 2). The note's "information
  threshold" *is* these results; it adds no derivation and converges on the
  terminology we already use ("information floor," not "phase transition").
- **The strong collapse was pre-registered and REFUTED.** `THEORY.md` §3a/§8.4
  proposed exactly the note's unification — magnitude cancels
  (`δ²·T_d = 2·log(1/α)/r*`), cost reduces to closeness-to-floor, controller
  identity drops out. §5.8 killed it: post-step regret rises **`∝ δ²`, not flat**;
  the field runs *far inside* the floor (it is "real but diagnostic, not a
  binding budget"); and `METRIC_DERIVATION.md` §8.3's scope-correction shows
  settled-`e` **sign flips across boundary TYPE at fixed `(τ, r*)`** (−13% under
  SignPersistenceCusum vs +1.8% under AdaptivePoissonCusum at τ=120, spm6–8).
  Identity does **not** wash out. "Three things refused to be unified"
  (`THEORY.md` §10).
- **Detection is window-limited, not evidence-limited — the note assumes the
  opposite.** The note's `R = detection-time / disturbance-time` presumes
  *evidence-limited* detection (`T_d ∝ 1/δ²`, faster for bigger drops). §5.8
  measured the field to be **window-limited** (`T_d ≈ const` in `δ`) — which is
  *precisely why* the §3a collapse failed ("the data's `regret ∝ δ²` is the
  signature of window-limited detection"). **But read the scope of that result
  exactly: §5.8 measured *steps* on *fixed-window* controllers.** A sustained
  ramp is a different object (§1 below), and "window-limited" is a property of
  the fixed-window field, not a law about every estimator. This is the seam the
  open question lives in.

So the honest residue of the note is **one** open question, with two halves that
must be built separately because one is foregone and one is not.

## 1. The open question (STEP-refuted ≠ DECLINE-refuted; and L* is a *bias–variance* floor)

§3a/§8.4 were refuted on *step* and *aged-drop* scenarios. The regime the safety
gate actually binds on — and the one still sim-only for the present champion
(§9.4) — is the **sustained decline**, governed by the *dynamic* floor
`L(τ_eff)`, not the step detection-time `T_d`. Two refinements over the note, both
correcting a latent overclaim that is also in the source:

**(a) `L*` is a bias–variance floor, not an information floor — only half of it
is CRB-protected.** Write `L(τ) = ρ·τ + z/√(r*·τ)`. The second term is the
Cramér–Rao information floor (Theorem 2). The first term `ρ·τ` is a
**deterministic lag bias** — the estimator's window trailing a moving truth — and
it is **removable**: `METRIC_DERIVATION.md` §10 falsifier (b) already hedges that
"biased estimators routinely beat the CRB on variance." So the statement "no
algorithm beats `L*` on a decline" — as written in the note's §1 *and* latent in
METRIC §3 — is **scoped to uniform-window (level) estimators**. An estimator that
*models the slope* (Holt / Kalman-velocity / a matched detector) subtracts the
`ρ·τ` bias and can sit **below** `L*`. The right name for `L*` is `THEORY.md`
§6's: a **bias–variance detectability floor**, not an unbeatable information
floor. This is what makes Experiment B (§3) a *live* question rather than a
foregone re-confirmation.

**(b) Minimizing `L` gives a single group with exponent 1/3 — on the
*envelope*.** Setting `dL/dτ = 0`:

```
        τ*  ∝  (z/ρ)^{2/3}·r*^{−1/3},        L*  ∝  (ρ/r*)^{1/3}.        (★)
```

Both terms of `L` collapse to `(ρ/r*)^{1/3}` *at* `τ*`. `ρ/r*` is dimensionless
once shares are a pure count: the **fractional decline accrued per share
arrival** — how fast the miner fades per unit of evidence. **But (★) is the cost
of the arm that sits *on* the floor (per-cell-optimal τ). A *fixed*-window arm
does not obey it** — and that is the crux of the redesign in §2.

**(c) The `e^{−e}` starvation correction makes (★) convex, not broken.** (★) used
the `e≈0` Fisher info `λ = r*τ`, but a decline is exactly where `e` is not small:
the realized rate is `λ = r*τ·e^{−e}` (`THEORY.md` §5.2 — the death-spiral living
inside the floor), and the lag *is* `e* ≈ ρ·τ*`. So the self-consistent floor
suppresses `r*` by the very lag it measures. The key point: **on the envelope
`e*` is itself leading-order `∝ (ρ/r*)^{1/3}`**, so the corrected cost is

```
        L*_corrected  ∝  (ρ/r*)^{1/3} · exp( c·(ρ/r*)^{1/3} ).            (★★)
```

This is **still a one-group function of `ρ/r*`** — the collapse does not fail. It
is merely **convex on log–log** (an upturn) rather than a straight slope-1/3 line.
The exact curve needs the self-consistent τ-solve and should be computed
numerically, not asserted in closed form. Consequence for the test: the failure
mode to guard against is **fitting a power law to a convex curve** and reading the
upturn — which coincides with the gate-binding sub-guard cells — as a *collapse
breakdown* or *controller identity*. It is neither; it is the spiral term, and it
is a sharp, free prediction (§2, criterion 2).

**Hypotheses, as a decomposition not a contest.** The note's two hypotheses are
two *components* of the decline failure budget:

- a **floor** component — the irreducible `L*` of (★)/(★★), the bias–variance
  detectability floor (the note's "information threshold," real but beatable by a
  slope-modeling estimator per (a));
- a **gap** component — everything a *realized fixed-window* controller sits
  *above* `L*`: the unspent-information gap (§2), boundary type, sign-persistence,
  the EWMA Jensen bias (`SLOW_DECLINE_TEST.md` §6 sub-guard, +5%), the τ choice.

The question is **how much of the decline budget is floor vs gap, where on the
`(ρ, r*)` plane, and whether a slope-aware arm can push *below* the floor.**

## 2. Experiment A — the three-arm collapse test (which group does each arm obey?)

The note (and the earlier spec draft) framed A as "does the field collapse onto
`ρ/r*`?" with three *candidate groups* and a fixed-arm field. That is the wrong
shape, because **the fixed arms collapse — just not onto `ρ/r*`.** For a fixed
window the floor-relative excess is, algebraically,

```
        (L − floor)/floor  =  (ρ/z)·√r*·τ^{3/2}                          (fixed τ)
```

— a clean one-group law on **`ρ·√r*`**, *not* `ρ/r*`, and a gap that **grows with
`r*`**: a fixed window wastes information it does not spend, and wastes more of it
the more there is. So the experiment is not "collapse vs no-collapse"; it is
**three arm-classes, each on its own group, stacked vertically on one `ρ/r*`
axis** — and the vertical structure *is* the floor-vs-gap decomposition §1 asks
for.

**The three arm-classes (one figure, one `ρ/r*` axis).**

1. **Fixed-window (the field: champion `Ewma360`, interim, classic).** Predicted
   to lie on **`ρ·√r*`**, above the floor, gap growing with `r*`. This arm is
   §8.3's `τ*∝1/r` slide re-expressed (the fixed 360 is deliberately off the
   per-rate optimum). Its raw material **already exists**: `slow-decline.rs` emits
   `mean_e_pct` (= `regret_over` over the decline) on the ρ∈{1..40}×spm∈{2..30}
   grid; `bin/collapse` is a post-processor over it plus the normalization.
2. **Per-cell-optimal-τ EWMA (the arm that traces `L*` by construction).** An
   oracle level estimator with `τ = τ*(ρ, r*)` from (★) — i.e. a level estimator
   sitting *at* its bias–variance balance in every cell. Predicted to lie on the
   **`(ρ/r*)^{1/3}` envelope (★)**, bending to the convex **(★★)** in the
   sub-guard corner. This is the one piece of genuinely **new measurement** A
   needs: the *unconfounded `L*` depth*. `tau-family.rs` deliberately does **not**
   supply it — it reads argmin *positions* to dodge the clamp-magnitude confound
   (§8.3 RESULT) — so the depth requires its own per-cell τ-optimal depth sweep
   with the clamp confound controlled.
3. **Ramp-aware / matched (the arm that can go *below* `L*`).** Built and raced in
   Experiment B; plotted here for the vertical picture. Per §1(a) it removes the
   `ρ·τ` bias and can sit under the (★) envelope.

**Pass/fail.**

1. *Which-group, per arm (graded).* Fit each arm-class against both `ρ/r*` and
   `ρ·√r*`. **Prediction: fixed arms collapse on `ρ·√r*`; the τ*-EWMA collapses
   on `ρ/r*`.** A fixed arm that collapses on `ρ/r*` instead, or a τ*-EWMA that
   does not, falsifies the floor/gap split. (This *replaces* the note's "report
   all three groups, don't pre-commit": the algebra pre-commits each arm to a
   group, and the test is whether the arm obeys its predicted group.)
2. *Envelope exponent + convexity (hard, arm 2).* Fit the τ*-EWMA on log–log.
   **Fit the self-consistent convex (★★), not a power law.** A straight slope-1/3
   over the whole range means the `e^{−e}` term is negligible (benign regime
   only); the **convex upturn in the sparse/fast cells, coinciding with the
   gate-binding sub-guard corner, is the prediction** — the spiral in the floor.
   Do *not* read the upturn as collapse-failure or identity (the §1(c) misread).
3. *Floor-vs-gap, by vertical separation (hard).* The gap between arm 1 (fixed)
   and arm 2 (τ*-EWMA) at fixed `ρ/r*` is the **unspent-information gap**, and §1
   predicts it *grows with `r*`* (since arm 1 ∝ `ρ√r*` and arm 2 ∝ `(ρ/r*)^{1/3}`
   diverge). Color arm 1 by controller: a controller-*ordered* residual within
   the fixed arm is the §8.3 sign-flip extended to declines (identity persists);
   an unordered one supports "fixed controllers cluster." Either is publishable;
   §8.3 predicts ordered.

This is the figure §8.4 would have become had it not been refuted on steps first
— drawn now in the one regime where it survives, and as a *layered* picture
(which group each arm obeys) rather than a single-curve collapse.

## 3. Experiment B — the matched-detector race (the genuinely-open half)

**This is the half that is NOT foregone**, and §1(a) is why. The window-limited
result (§5.8, `regret ∝ δ²`) is *steps on fixed windows*. A sustained ramp has a
**removable lag bias** `ρ·τ`, and **no arm in the field is ramp-aware** —
champion, interim, and classic are all *level* estimators. A slope-modeling arm
is the dynamic form of §10 falsifier (b), untested, and the only result here that
can **reopen** sparse-estimator work rather than re-confirm §8.3.

> Build a genuine slope-aware tracker — Holt double-exponential / Kalman with a
> velocity state / a Page-CUSUM matched to a Poisson rate-decline — and race it
> against the fixed-window champion on the §2 sustained-decline grid.

This is the controlled head-to-head `THEORY.md` §5.3 sketched analytically (SPRT
discussed, never built; the champion is EWMA-window).

**Pin the response variable to ONE floor (the point-4 correction).** §3 of the
note scored "ease-fire **latency**" against `L*` in one breath — a category error:
latency is a **detection-time** object (floored by `T_d`, eq. 2), `L*` is a
**tracking-regret** object. Choose, per measurement:

- **Latency → vs `T_d`.** If the question is "how fast does it fire," measure
  ease-fire latency per cell and compare to `T_d(δ_eff) ≈ 2·log(1/α)/(r*·δ_eff²)`.
- **`regret_over` → vs `L*`, controller-vs-controller.** If the question is "does
  it track the decline cheaper" (the one that matters for the floor), the matched
  detector **must carry an actuator** (an update rule) so it is a controller, not
  a bare detector, and its `regret_over` races the champion's against the `L*`
  envelope of §2 arm 2.

**Use oracle-`ρ` for the clean ceiling.** Give the slope-aware arm the true ramp
rate. The clean read is: *if even the best-case (oracle) slope-aware arm does not
beat the champion's `regret_over` by more than the §3 noise band, the
floor-limited conclusion is airtight* — Theorem 2 hardens dynamically and the
slow window is not leaving tracking on the table. If the oracle arm *does* beat it
materially, run the estimated-`ρ` version to see how much of the win survives a
real slope estimate.

**Pass/fail (thesis-level).**

- **Floor-limited (note's thesis HOLDS, *and it is the stronger statement*):** the
  oracle slope-aware arm does **not** beat the champion's decline `regret_over` by
  more than the §3 band ⇒ the removable bias is not worth removing at these rates,
  `L*` binds even slope-aware arms, estimator R&D on declines is dead-ended by
  physics. This is *stronger* than §6.1's "the slow estimator is safe" — it would
  show no estimator is materially *faster* either.
- **Estimator-limited (the reopen):** the slope-aware arm substantially beats the
  champion ⇒ the decline regime is gap-dominated, the level-estimator window
  leaves detectability on the table, and `AsymmetricPoissonCI` / a slope-modeling
  sparse estimator are back on the table (the §10 reopen-trigger). Note this would
  reopen the champion at the margin and owe an spm≥6 re-confirm (§9.4 / §10(d)
  discipline).

Either outcome is decision-relevant; the current docs assert *safety* of the slow
estimator, not *optimality* of its latency or tracking.

## 4. What each outcome buys

| Result | Floor-dominated | Gap-dominated |
| --- | --- | --- |
| **A: which-group** | τ*-EWMA on `(ρ/r*)^{1/3}`, fixed arms on `ρ√r*` → the floor/gap split is real and the dynamic floor is publishable as Theorem 2's companion | a fixed arm collapses on `ρ/r*`, or τ*-EWMA doesn't → the algebra of §1–2 is wrong; investigate |
| **A: envelope** | τ*-EWMA fits convex (★★), upturn in the sub-guard corner → the spiral-in-the-floor is measured; publish | straight slope-1/3 everywhere → `e^{−e}` negligible in-grid; weaker but clean |
| **A: gap ordering** | unordered residual in the fixed arm → "fixed controllers cluster" extends to decline safety | ordered residual → §8.3 sign-flip extends to declines; identity persists (likely) |
| **B: detector** | oracle slope-aware arm does NOT beat champion → strongest safety claim available; estimator R&D dead-ended | slope-aware arm wins → champion's level window is suboptimal on a ramp; reopens sparse/slope estimator + boundary asymmetry |

## 5. Hardware version (shape-proxy)

Per `SLOW_DECLINE_TEST.md` §5, confirm the sim the way the drop test was. Drive
the shape-proxy **Ramp** on an S21/testnet4 at two `(ρ, r*)` points chosen to sit
at the **same `ρ/r*`** but **different `(ρ, r*)` individually**, and read the
difficulty response off Grafana. The fixed champion (a level estimator) obeys
`ρ√r*`, not `ρ/r*` (§2), so the prediction for *the shipped controller* is that
the two responses **differ** (the gap grows with `r*`) — the opposite of the
note's "same `ρ/r*` ⇒ same shape," and a direct hardware read of the
unspent-information gap. Champion vs classic side-by-side (the ship/no-ship
methodology). Add the matched detector to iron only if Experiment B flags a sim
divergence worth confirming.

## 6. Framing note (for the white paper, if any of this lands)

Keep the note's own good advice and our existing choice: **do not call it a phase
transition.** What Experiment A characterizes is a **bias–variance detectability
floor** (§1(a) — the right name), not a critical phenomenon: there is no order
parameter, the "transition" is graded (`SLOW_DECLINE_TEST.md` shows bounded
transient lag, not a recovery cliff; even classic recovers within the window), and
§1(c) shows the dangerous corner is a *convex upturn* of one smooth curve, not a
discontinuity. One quantitative refinement worth stating *if* the envelope holds:
the **width** of the transition region should scale as `1/√(r*τ)` — a finite-count
rounding of the floor — so it narrows as `r*` rises. That is a second, independent
prediction the same grid tests for free.

## 7. Build order and results (`bin/collapse`, `bin/matched-detector`)

**Build order — B over A** (inverting the earlier lean):

- **A is characterization only.** The fixed-arm collapse is §8.3 re-expressed
  (`ρ√r*`, not new), and the one new number — the unconfounded `L*` depth (arm 2)
  — does not reopen the champion: the rate-aware closure (§8.3 Net status) already
  settled that the window has *no admissibility content*. Build A only if you want
  the dynamic floor as the paper's companion to Theorem 2. It is a post-processor
  over `slow-decline.rs` / `tau-family.rs` plus one per-cell-optimal-τ depth
  sweep (clamp confound controlled).
- **B is the live question.** The window-limited result is steps-on-fixed-windows;
  a ramp has a removable bias and the field has no slope-aware arm. B is the only
  experiment here that can *reopen* rather than re-confirm. If one thing is built,
  build B, scoped as in §3 (one floor per response variable; actuator on the
  detector; oracle-ρ for the ceiling).

_Results to fill — Experiment A: per-arm group fits (fixed on `ρ√r*`, τ*-EWMA on
`(ρ/r*)^{1/3}`), the convex (★★) fit + sub-guard upturn, fixed-arm gap ordering.
Experiment B: `regret_over` (and/or latency) champion vs oracle matched detector
per cell, against the §3 floor; estimated-ρ follow-up if the oracle arm wins. HW
two-point same-`ρ/r*` shape overlay (predicted to differ for the level champion)._
