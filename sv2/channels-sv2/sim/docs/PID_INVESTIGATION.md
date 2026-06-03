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

## What We Learned From PID

Despite the broken quantization, the PID *concept* revealed gaps in our
framework:

### 1. Integral Term → AcceleratingPartialRetarget

PID's integral term accelerates correction when error persists in one
direction. Our `PartialRetarget(η=0.2)` always moves exactly 20% of the
gap regardless of history. The new `AcceleratingPartialRetarget` captures
this insight: η ramps from 0.2 → 0.4 → 0.6 on consecutive same-direction
fires.

**Parameter sweep results** (500 trials, 5 SPM × 10 scenarios):
- `acceleration=0.2, eta_max=0.6` is optimal
- Convergence improved 9-40% across SPM=6-30
- Jitter: zero cost (identical to baseline)

### 2. Operating in SPM-Space → SpmRatioEstimator

PID operates on `realized_spm` directly without converting through
hashrate/target. Our `SpmRatioEstimator` does the same: EWMA smoothing
on the raw SPM signal, then `h_estimate = current_h × (realized/expected)`.

**Result**: Behaviorally identical to `EwmaEstimator` (confirmed via paired
simulation). The benefit is code simplification — no `hash_rate_from_target`
U256 path in the estimator.

### 3. Sub-threshold Persistence → SignPersistenceCusumBoundary

In PID, errors below the dead zone still accumulate in the integral. Our
`SignPersistenceCusumBoundary` adapts this: when deviation sign persists
across ticks, the threshold decreases slightly.

**Result**: +6% detection rate on ±10% steps, but +23% jitter on stable
load. Marginal net benefit — needs further tuning to be production-viable.

## New Composition: "BestOfBest"

```rust
Composed::new(
    SpmRatioEstimator::new(120),
    AsymmetricCusumBoundary::new(1.5, 0.05, 3.0),
    AcceleratingPartialRetarget::new(0.2, 0.6, 0.2),
    min_allowed_hashrate,
    clock,
)
```

### Head-to-Head vs Production (1000 trials, 8 SPM × 10 scenarios)

| Metric | VardiffState (η=0.5) | BestOfBest (η=0.2→0.6) |
|--------|---------------------|------------------------|
| Step ±50% convergence | 0.943 | 0.933 |
| Step ±50% reaction | 0.906 | 0.892 |
| Stable jitter | 0.042 | 0.042 |
| Settled accuracy (SPM≥15) | 0.077 | 0.032 |
| Cold start p50 (SPM=12) | 4 min | 6 min |

### Tradeoff

BestOfBest trades ~2 minutes slower cold start for 2-3× better steady-state
accuracy at high SPM. Cold starts happen once per connection; steady-state
runs for hours/days.

## Files Added

### Production components (`src/vardiff/`)

- `pow2_pid.rs` — Reference Pow2-PID implementation for simulation
- `pid_tuned.rs` — Well-tuned PID implementation (P+I+D active)
- `composed/update.rs` — `AcceleratingPartialRetarget` (new UpdateRule)
- `composed/estimator.rs` — `SpmRatioEstimator` (new Estimator)
- `composed/boundary.rs` — `SignPersistenceCusumBoundary` (new Boundary)

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
