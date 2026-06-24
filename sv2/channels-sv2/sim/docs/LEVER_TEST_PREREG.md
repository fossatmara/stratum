# Lever test — pre-registered formula (committed BEFORE the data)

Written 2026-06-23, before pulling the settled raw_count data from slots 3/4,
specifically to remove the post-hoc normalization degree of freedom. The dt_secs
are irregular (time between fires, driven by share arrivals: 43/103/104/164s
observed), and "normalize for dt" is NOT one operation — the choice determines
whether the lever appears to hold. So the test is fixed here, now.

## What Theorem 2 actually predicts

Floor: a raw count `N` over a window of length `dt` at true rate `r` (shares/min)
is Poisson with mean `μ = r·(dt/60)`. So:
  - `Var(N) = μ = r·dt/60`
  - the rate estimate `r̂ = N·60/dt` has `Var(r̂) = r·60/dt`  (= 3600·r/(60·dt) … i.e. Var(N)·(60/dt)²)

This is a PER-WINDOW prediction keyed to that window's own `dt`. It is NOT a
single floor across windows, because each window has a different `dt` → different
floor. Pooling windows of different `dt` into one SD confounds Poisson noise with
the dt distribution, and the dt distribution differs between slots (sparse 6 spm →
long/variable dt; dense 30 spm → short/tight dt). A pooled-SD ratio is therefore
contaminated exactly as the sign-bug was contaminated by orientation. Do not use it.

Also: `N` and `dt` are COUPLED (both driven by the same Poisson arrivals — a long
window is more likely a slow-share window), so any statistic treating them as
separable is suspect. The per-window test below conditions on each window's own dt
and is robust to this; a pooled statistic is not.

## THE TEST (Option 1 — per-window observed-vs-predicted; the only no-free-parameter form)

For each settled-window decision record `(N = raw_count, dt = dt_secs, r* )`:
  1. The controller has converged, so its set difficulty implies an expected
     count `μ̂ = r*·dt/60` (it is TRYING to make realized = r*). Use the realized
     rate's own mean over the flat window as `r_bar` if it differs from r* (the
     settled offset means realized sits slightly above r*); predicted count is
     `μ_pred = r_bar·dt/60`.
  2. Observed squared deviation: `(N − μ_pred)²`.
  3. Predicted variance (Poisson): `μ_pred`.
  4. Floor-holds statistic at a rate: the ratio
        F = mean over windows of (N − μ_pred)²  /  mean over windows of μ_pred
     F ≈ 1  ⟺  the per-window spread matches Poisson  ⟺  the floor is operating.
     (Equivalently: pooled Pearson dispersion of counts against their per-window
     Poisson means. F≈1 = at the floor; F≫1 = excess noise beyond Poisson;
     F≪1 = sub-Poisson / over-smoothed.)

## VERDICT RULES (pre-committed; no choice left after seeing numbers)

- **Lever CONFIRMED** iff F ≈ 1 (within sampling error, say [0.5, 2]) at BOTH
  rates. That means each rate's count noise sits at its own Poisson floor, and
  since the floor `1/√(r·τ)` is rate-dependent BY the formula, confirming the
  floor at both rates IS the rate-scaling — the lever. (We do NOT need to take a
  cross-slot SD ratio; the per-window test already carries the rate-dependence
  through μ_pred = r·dt/60.)
- **Lever REFUTED** iff F is reliably ≠ 1 at one or both rates in a way that
  contradicts Poisson (e.g. F≈1 at 30 but F≫1 at 6, meaning excess noise at low
  rate beyond what the floor predicts) — a real finding needing explanation.
- **UNMEASURABLE** iff: too few settled windows for F's sampling error to
  distinguish it from 1 (compute the SE on F and check), OR the belief isn't flat
  (still ramping → μ_pred wrong), OR realized-rate drift within the window makes
  r_bar ill-defined. This is a fully honest and LIKELY outcome given the dt
  irregularity and sparse fires; do not reach past it.

## SCOPE GUARDS (pre-committed)

- If slot 3 (6 spm) is not flat-converged over a long trailing window, it gives
  NOTHING for the lever. Slot 4 alone then yields only "floor holds at 30 spm"
  (single-rate), NOT the lever (which is rate-SCALING and needs ≥2 rates). Report
  that as single-rate-floor-confirmed, lever-pending — do not let "slot 4 carries
  it" slip into "we have the lever."
- F is computed against EACH window's own dt — never pool windows of different dt
  into one variance, never bin/resample to a fixed dt (that reintroduces a tunable
  bin width → the convention-tuning trap).
- Belief-flatness checked over a LONG trailing window (not 10 cycles), distinct
  h_belief values = 1.

## Why not the smoothed-e band

SD(e) is the EWMA-smoothed belief-vs-estimate residual; the EWMA's time-window
flattens the raw rate-dependence, so SD(e) is SILENT on the floor (measured 0.89
ratio earlier — neither confirms nor refutes). raw_count bypasses the smoothing;
that is the whole reason it was logged.
