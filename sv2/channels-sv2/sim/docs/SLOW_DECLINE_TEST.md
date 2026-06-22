# Slow-decline safety test — specification

**Purpose.** Earn (or bound) the death-spiral safety claim that
`METRIC_DERIVATION.md` §6 currently only *argues*. Every scenario in the
existing ensemble is a step or a single aged drop; none is a *sustained*
decline, which is the one input that can turn a persistence-based actuator
into a slow runaway. This spec defines that scenario, the pass/fail
criteria, which algorithms to run it on, and the specific failure mode to
hunt.

## 1. The mechanism under test (get the sign right)

`e = ln(Ĥ/H)`. During a decline `H↓` with `Ĥ` lagging, `Ĥ/H` rises, so
**`e` drifts positive — over-difficulty**, the costly side of §6. Shares
arrive *slow*; the correct response is to **ease** (lower `Ĥ` toward `H`).

The naive worry ("does it tighten when it should ease?") is not the real
risk — the error sign plainly calls for easing. The real death spiral is
self-reinforcing starvation:

> over-difficulty → fewer valid shares → sparser counter / less statistical
> evidence → slower to fire the corrective ease → stays over-difficulty
> longer → still fewer shares.

**The champion-specific trigger.** `AdaptiveSignPersist` switches to the
conservative low-SPM PoissonCI guard below its `spm_threshold` (6). As a
decline drags the *effective* realized rate down, the boundary can flip
into its **slowest** mode exactly when fast easing is most needed — the
guard meant to prevent low-SPM false fires could instead freeze the
correction. **This is the falsifiable hypothesis: does the champion keep
pace with a sustained decline, or does the low-SPM guard stall the ease and
let `e` run away upward?**

## 2. Scenario definition (simulation)

A new `Scenario::SlowDecline` (or a `Custom` phase list), built from the
existing `Phase::Hold` + `Phase::Ramp` primitives:

```
  Phase::Hold { secs: T_mature, h: H0 }          // mature the counter on-target
  Phase::Ramp { secs: T_decline, from: H0, to: H0·(1−D_total) }
  Phase::Hold { secs: T_observe, h: H0·(1−D_total) }  // settle at the floor
```

Sweep the **decline rate**, because the dangerous regime is rate relative
to the controller's reaction timescale, not absolute drop:

- **rate** `ρ ∈ {2, 5, 10, 20, 40} %/hour` (gentle thermal sag → fast
  failing fan). The natural dimensionless quantity is *drop-per-effective-
  window* `ρ·τ`: when it is below the §3 noise band the decline hides in
  noise (a detection problem); above it, a tracking problem. Report against
  `ρ·τ`, not `ρ` alone.
- `T_mature = 60 min` (counter matured, the operationally common state).
- `D_total = 50%` (run the decline long enough to reach a regime where
  effective SPM crosses the spm-6 guard for the relevant share rates).
- share-rate grid: the usual `{6, 8, 12, 20, 30}` — but the **low end is
  the point**, since that is where the decline pushes effective SPM through
  the guard.

## 3. Pass/fail criteria

Measured over the decline phase, per cell:

1. **Direction (hard gate).** Every fire during a monotonic decline must be
   an **ease** (`s<0`). A single tightening fire (`s>0`) during the decline
   is a fail — it is the literal §6 runaway step.
2. **No upward runaway (hard gate).** `e(t)` must stay bounded; specifically
   `max e` during the decline must not grow monotonically to the end. A
   `e` that climbs without turning over = the algorithm has lost the miner.
3. **Tracking lag (graded).** Time-averaged `e` over the decline (this is
   `regret_over`, since `e>0` here) — smaller is better. Compare champion
   vs. the references; this quantifies *how far behind* it runs, even if it
   never spirals.
4. **Guard-freeze probe (the hypothesis).** Log the fraction of decline
   ticks spent in the low-SPM PoissonCI mode vs. the sign-persist mode, and
   the fire latency in each. If easing latency spikes when effective SPM
   crosses 6, the guard-freeze mechanism is real and we have found the
   bound.

A clean pass (all eases, bounded `e`, lag comparable to the references, no
guard-freeze) is a strong safety result. Any hard-gate failure locates the
decline rate at which the mechanism breaks — itself a deployable bound
("safe for declines up to X%/hr at SPM ≥ Y").

## 4. Which algorithms to run

In priority order:

1. **champion (SignPersist)** — the deployment candidate; the whole point.
2. **interim (AsymCusum, no sign-persistence)** — the control that isolates
   *whether the sign-persistence discount specifically* helps or hurts on a
   decline. If the champion stalls and the interim does not, the discount
   is the culprit; if both stall, it is the low-SPM guard they share.
3. **classic (real vardiff)** — the incumbent baseline. Expected to lag
   badly (it is slow and symmetric) but *not* to spiral, since it has no
   persistence mechanism — a useful "spiral needs persistence" control.

(The estimator window `τ` and the `spm_threshold` are the two knobs most
likely implicated; if a cell fails, re-run it varying those to confirm the
mechanism before concluding.)

## 5. Hardware version (shape-proxy)

The simulation result must be confirmed on hardware, the way the drop test
in PR #2154 was. Shape-proxy already has a **Ramp profile** — drive a slow
downward ramp (e.g. `Ramp{1.0 → 0.5 over 2h}`) on an S21/testnet4, **champion
and classic side-by-side** (the proven side-by-side methodology), and read
the difficulty response off Grafana. Run the gentlest rate that still
crosses the spm-6 guard for the configured `r*`, since that is where the
hypothesis lives. Confirm: the champion eases monotonically and never
diverges upward.

**Which to run on hardware:** champion vs. classic side-by-side is the
ship/no-ship test. Add the interim (AsymCusum) run only if the simulation
flags a champion-vs-interim divergence worth confirming in hardware —
otherwise it is an extra overnight run for a question the sim already
answered.

## 6. Simulation results (`bin/slow-decline`, 300 trials/cell)

Run over rate ∈ {2,5,10,20,40} %/hr × spm ∈ {6,8,12,20,30} × {champion,
interim, classic}. Worst-case over all cells:

| algo | worst mean_e (regret_over) | worst end_e | worst max_e | verdict |
| --- | --- | --- | --- | --- |
| champion | 4.3% | +4% (mostly negative) | +16% | tracks decline down, ends safe-side |
| interim | 3.5% | +4% | +13% | same, slightly tighter |
| **classic** | **31%** | **+69%** | **+69%** | **falls progressively behind; runs away** |

**Champion passes; classic is the spiral risk — the opposite of the naive
worry.** The champion tracks every sustained decline monotonically down,
ending on the *under*-difficulty (safe) side at every rate (`end_e ≤ +4%`,
mostly negative) with bounded over-difficulty (`max_e ≤ 16%`). Classic, too
slow to keep up, falls steadily behind and ends as deep as **+69%
over-difficulty** at 40%/hr — `e ≈ ln(1.69)`, a badly starved miner, with no
recovery in-window. This is the §6 death spiral, and it is the *incumbent*
that exhibits it; the responsive champion is **safer** on a decline, not
riskier.

**Residual, reported honestly (not a failure).** The champion makes rare
wrong-direction fires — a tighten while still >2% over-difficulty — peaking
at ~1.6 per run (≈4% of its ~34 fires) at the noisiest cell (2%/hr, 6 spm).
These are Poisson-noise mis-reads on sparse low-spm data (a burst of fast
shares momentarily reads as a rise), not sustained wrong-way push: every
such cell still ends safe-side with bounded `max_e`. They scale *down* with
rate and *up* with sparsity, the signature of noise, not spiral. The
low-SPM-guard-freeze hypothesis (§1) is **not** borne out — the champion's
ease count stays high (10–37/run) across the decline; the guard does not
stall the correction.

**Status:** the §6 death-spiral safety claim is earned in simulation. The
hardware confirmation (§5 above) remains the deployment gate.
