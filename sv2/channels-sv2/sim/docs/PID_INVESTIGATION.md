# PID Vardiff Investigation

## Background

Some open-source pool implementations use a PID controller for variable
difficulty, operating in difficulty-space with power-of-2 quantization.
This document records our investigation of the PID approach, comparison
to our three-stage pipeline, and the improvements derived from the analysis.

## The Pow2-PID Pattern

A common open-source pattern uses the `pid` crate (v4.0.0) with:

- **Setpoint**: shares per minute target (typically 10.0)
- **Kp**: `-difficulty × 0.01` (negative: over-target SPM → lower difficulty)
- **Ki**: `0.0` (disabled in production)
- **Kd**: `0.0` (disabled in production)
- **Output limit**: `difficulty × 10.0`
- **Quantization**: `nearest_power_of_2()` on every retarget
- **Interval**: 120s (configurable)
- **Measurement window**: 20-60s sliding window of share timestamps
- **On retarget**: rebuilds PID with gains proportional to new difficulty

### Critical Flaw: Power-of-2 Dead Zone

The quantization creates a ~41% dead zone. For difficulty to change, the PID
output must push past the geometric midpoint between adjacent powers of 2:

```
|pid_output| > 0.414 × current_difficulty
With Kp = -0.01 × diff:
  |SPM_target - realized| > 41.4
```

At SPM target of 10, realized SPM must exceed ~51 or drop to ~0 to trigger
a retarget. This means **hashrate changes of ≤5× go undetected**.

### Simulation Results

Across all 50 cells (5 SPM × 10 scenarios), Pow2-PID:
- Reaction rate: **0.000** — never fires on any step ≤ ±50%
- Jitter: **0.000** — never fires at all
- Effectively a fixed-difficulty system

## Why PID Fails: Lack of Stage Separation

A PID controller conflates all three pipeline stages into a single
feedback loop, making it impossible to diagnose or fix individual
failure modes:

| PID Term | Conflated Stages | Problem |
|----------|-----------------|---------|
| P (proportional) | Estimator + Boundary | Gain (`Kp`) simultaneously controls how noisy the "belief" is AND how much deviation triggers action. Tuning Kp for low jitter (small gain) kills reaction rate. Tuning for fast reaction (large gain) causes noise-driven fires. |
| I (integral) | Boundary (persistence) + Update (magnitude) | Accumulates sub-threshold error — a boundary concern (evidence strength) — but its output adds directly to the control signal — an update concern (move magnitude). Anti-windup limits are simultaneously clamping "how much evidence to accumulate" and "how far to move." |
| D (derivative) | Estimator (smoothing) + Update (damping) | Acts as both a noise filter on the measurement AND a damping term on the actuator. Cannot tune measurement smoothing independently of move damping. |

### The Dead Zone as a Stage Confusion

The 41% dead zone from power-of-2 quantization is instructive. In our
framework, this is clearly a *boundary* problem — the threshold for
action is too high. But in the PID implementation, the dead zone arises
from the interaction of:
1. Gain magnitude (Kp = -0.01 × diff) — an estimator/boundary concern
2. Quantization rounding — a post-update concern
3. Output limit (10 × diff) — an update concern

Because these aren't separated, the developer cannot identify "the boundary
is too wide" as the root cause. They would instead try to increase Kp
(breaking jitter), add integral (breaking stability), or reduce the
quantization (breaking the power-of-2 invariant the system depends on).

### The Well-Tuned PID (`pid_tuned.rs`)

Our `PidTunedVardiff` implementation with all three terms active
demonstrates the ceiling of the PID approach when carefully tuned:
- Rate-aware gain scheduling (√SPM noise scaling)
- Anti-windup with exponential decay + hard clamp
- Dead zone to suppress noise-driven fires
- Configurable presets (balanced, aggressive, conservative)

Even with these improvements, it cannot escape the fundamental coupling:
the dead zone (a boundary parameter) interacts with the integral
accumulation (a persistence parameter) which interacts with the gain
schedule (an estimator parameter). Tuning one axis shifts the others.
The three-stage pipeline makes these interactions explicit and allows
each to be optimized independently.

## What We Learned From PID

Despite the broken quantization, the PID *concept* revealed gaps in our
framework. Crucially, decomposing each PID term into the three-stage
pipeline let us evaluate each idea **in isolation** — something the
conflated PID design fundamentally cannot do. Three candidates were
extracted; only one survived rigorous re-evaluation.

### 1. Integral Term → AcceleratingPartialRetarget *(transferred)*

PID's integral term accelerates correction when error persists in one
direction. Our `PartialRetarget(η=0.2)` always moves exactly 20% of the
gap regardless of history. The new `AcceleratingPartialRetarget` captures
this insight: η ramps from 0.2 → 0.4 → 0.6 on consecutive same-direction
fires.

**Parameter sweep results** (500 trials, 5 SPM × 10 scenarios):
- `acceleration=0.2, eta_max=0.6` is optimal
- Convergence improved 9-40% across SPM=6-30
- Jitter: zero cost (identical to baseline)

This idea transferred cleanly because it addresses a concern within a
single stage (update-rule magnitude over time). No cross-stage calibration
is involved.

### 2. Operating in SPM-Space → SpmRatioEstimator *(discarded)*

PID operates on `realized_spm` directly without converting through
hashrate/target. Our `SpmRatioEstimator` did the same: EWMA smoothing
on the raw SPM signal, then `h_estimate = current_h × (realized/expected)`.

**Initial result**: Behaviourally indistinguishable from `EwmaEstimator`
on the head-to-head benchmark — the supposed benefit was code
simplification.

**Re-evaluation**: Further scenarios exposed regressions that the
paired-simulation harness missed. The component is retained in the
codebase as an experimental alternative but is **not** part of any
production composition.

### 3. Sub-threshold Persistence → SignPersistenceCusumBoundary *(discarded)*

In PID, errors below the dead zone still accumulate in the integral. Our
`SignPersistenceCusumBoundary` adapted this: when deviation sign persists
across ticks, the threshold decreases slightly.

**Initial result**: +6% detection rate on ±10% steps at the cost of +23%
jitter on stable load.

**Re-evaluation**: The jitter penalty outweighed the detection gain across
the full grid; tuning attempts could not move the Pareto frontier. The
component is retained for reference but is **not** part of any production
composition.

### Meta-observation

Decomposition into three stages is what *made the failure modes visible*.
Two of three extracted ideas looked promising in narrow tests and were
ultimately rejected only because each could be exercised on its own
boundary, estimator, or update axis without confounding the others. The
PID's monolithic structure offers no such diagnosis — its dead zone, gain
schedule, and integral windup all interact, so a failing parameter sweep
gives no actionable signal about which concern is broken.

## Proposed Composition (not adopted)

A speculative "BestOfBest" composition combined all three extracted ideas:

```rust
Composed::new(
    SpmRatioEstimator::new(120),
    AsymmetricCusumBoundary::new(1.5, 0.05, 3.0),
    AcceleratingPartialRetarget::new(0.2, 0.6, 0.2),
    min_allowed_hashrate,
    clock,
)
```

Head-to-head (1000 trials, 8 SPM × 10 scenarios) showed a tradeoff —
~2 min slower cold start for 2-3× better steady-state accuracy — that
initially looked favourable. However, once `SpmRatioEstimator` and
`SignPersistenceCusumBoundary` were independently rejected (see above),
this composition was abandoned. The production recommendation lives in
[`CKPOOL_INVESTIGATION.md`](./CKPOOL_INVESTIGATION.md): `EwmaEstimator` +
`PoissonCI` + `PartialRetarget(η=0.2)`. `AcceleratingPartialRetarget`
remains available for ad-hoc composition where extra cold-start
aggression is desired.

## Files Added

### Production components (`src/vardiff/`)

- `composed/update.rs` — `AcceleratingPartialRetarget` (new UpdateRule) — the
  single PID-derived idea that survived isolated re-evaluation

### Experimental / reference components (`src/vardiff/`)

- `pow2_pid.rs` — Reference Pow2-PID implementation for simulation
- `pid_tuned.rs` — Well-tuned PID implementation (P+I+D active)
- `composed/estimator.rs` — `SpmRatioEstimator` — discarded after
  re-evaluation; kept for reference
- `composed/boundary.rs` — `SignPersistenceCusumBoundary` — discarded after
  re-evaluation; kept for reference

### Simulation binaries (`sim/src/bin/`)

- `compare-pid.rs` — Pow2-PID and tuned-PID vs all algorithms
- `compare-best.rs` — BestOfBest vs production (final comparison)
- `convergence-time.rs` — Convergence time measurement
- `sweep-accelerating.rs` — Parameter sweep for AcceleratingPartialRetarget

### Grid registrations (`sim/src/grid.rs`)

- `AlgorithmSpec::pow2_pid(spm, hashrate)` / `pow2_pid_default()`
- `AlgorithmSpec::pid_balanced(spm)` / `pid_aggressive(spm)` / `pid_conservative(spm)`
- `AlgorithmSpec::ada_cusum_accelerating(tau, s, f, t, eta_base, eta_max, acc)`
- `AlgorithmSpec::spm_ratio_cusum(tau, s, f, t, eta)`
- `AlgorithmSpec::ewma_sign_persistence(tau, s, f, t, sd, md, eta)`
- `AlgorithmSpec::best_of_best()`
