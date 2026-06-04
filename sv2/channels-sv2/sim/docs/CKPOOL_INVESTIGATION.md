# Ckpool Vardiff Investigation

## Background

[ckpool](https://github.com/ckolivas/ckpool) is a mature Stratum V1 pool
by Con Kolivas with a well-regarded vardiff implementation. A [Rust
reimplementation](https://github.com/parasitepool/para/blob/master/src/vardiff.rs)
by @paratoxicdev made the algorithm more configurable. This investigation
ports ckpool's core ideas to our tick-based SV2 framework, benchmarks
them against the existing roster, and identifies which ideas transfer
and which are context-dependent.

## Ckpool's Algorithm (src/stratifier.c)

### Core: Exponentially Decaying Shares-Per-Second

ckpool tracks `dsps` (difficulty-weighted shares per second) using an EMA
updated on every share submission via `decay_time()`:

```c
fprop = 1 - e^(-elapsed / interval)
f += (share_diff / elapsed) * fprop
f /= (1 + fprop)
```

Five parallel EMAs with different time constants (1m, 5m, 1h, 1d, 1w) are
maintained per client.

### Adaptive Window Switching

When shares flood in (`ssdc >= 72`), the 1-minute EMA is used for
evaluation — enabling rapid ramp-up. Otherwise the 5-minute EMA provides
conservative steady-state tracking. The threshold 72 = `240s / 3.33s`
(shares expected in 80% of the long window at target rate).

### Hysteresis Band

No retarget if the diff-rate-ratio (`drr = dsps / current_diff`) falls
within [0.15, 0.4] around the target 0.3 (~1 share per 3.33s). The band
is asymmetric: 0.5× below target, 1.33× above.

### Oscillation Guard

Suppress difficulty decrease if only 1 share has been observed since the
last change — prevents premature drops after idle periods.

### Time-Bias Warmup Correction

`bias = 1 - e^(-elapsed/period)` compensates for EMA suppression when the
client has been active for less than one full window period.

## Mapping to the Three-Stage Pipeline

### First Attempt: Direct Translation (Batch EMA + Time-Bias)

The initial port treated one tick as one "observation" — batching all shares
into a single EMA step, then dividing by the time-bias factor to compensate
for warmup:

| Stage | Component |
|-------|-----------|
| Estimator | Dual-window EWMA (τ_short=60s, τ_long=300s) with time-bias correction |
| Boundary | HysteresisGate [0.5, 1.33] with data gate (72 shares OR 240s) |
| Update | CkpoolRetarget (full retarget + oscillation guard) |

**Result: Catastrophic.** The hysteresis band was too wide for 60s-tick
evaluation — rate ratios wandered far from 1.0 while staying "inside" the
band. Settled accuracy 59-73% at SPM 6-12. Overshoot 100-200%.

### CkpoolRemedy (ckpool estimator + FullRemedy boundary/update)

Kept ckpool's estimator but replaced the boundary and update with proven
components (PoissonCI + PartialRetarget η=0.2):

**Result: Also bad.** Settled accuracy 177-275%, overshoot 350-427%.
The time-bias correction (`1 / (1 - e^(-60/300)) ≈ 1/0.18 ≈ 5.5×`)
massively amplified the rate estimate on every tick after a fire.

### Root Cause: Time-Bias Is Not Portable

The time-bias formula was calibrated for per-share evaluation (~3.33s
intervals) where `dt` grows smoothly. In the tick-based framework,
`dt` after a fire is always exactly 60s. With τ=300s:

```
time_bias(60, 300) = 1 - e^(-0.2) ≈ 0.18
correction = rate / 0.18 = rate × 5.5
```

This amplification is the overshoot source: the EMA is naturally low
after a fire (only one tick of data), and dividing by 0.18 inflates it
by 5.5× — a 450% bias on the first tick after every retarget.

## The Fix: Per-Share `decay_time()` Simulation

Instead of batch updates with post-hoc bias correction, we simulate
ckpool's exact per-share EMA updates within `snapshot()`:

When N shares arrive in a 60s tick, run N individual `decay_time()` calls
with `elapsed = 60/N` seconds each. This faithfully reproduces what
ckpool's EMA would have seen — the decay accumulates organically through
per-share updates rather than needing artificial amplification.

```rust
fn snapshot(&self, dt_secs: u64, ctx: &EstimatorContext) -> EstimatorSnapshot {
    // ... 
    if pending > 0 {
        let inter_share_secs = self.tick_secs as f64 / pending as f64;
        for _ in 0..pending {
            dsps_s = Self::decay_time(dsps_s, 1.0, inter_share_secs, self.tau_short as f64);
            dsps_l = Self::decay_time(dsps_l, 1.0, inter_share_secs, self.tau_long as f64);
        }
    }
    // ...
}
```

**Result:** Completely fixed the bias. Settled accuracy dropped from
177% to 4.6-7.5% — matching FullRemedy. Overshoot collapsed from 350%
to 0-9%.

### Lesson

When porting a continuous-time algorithm (per-share evaluation) to a
discrete-time framework (per-tick evaluation), the correct approach is to
simulate the original update cadence within each tick, not to apply a
time-domain correction factor. The `on_fire()` feedback mechanism of
the Estimator trait enables this: the estimator knows when retargets
happened and can simulate share arrivals between ticks accordingly.

## Parameter Sweep

With the per-share simulation working correctly, we swept the key axes:

| Variant | τ_short | τ_long | ft | Boundary | η | Key insight |
|---------|---------|--------|-----|----------|-----|-------------|
| CkpoolRemedy | 60 | 300 | 72 | PoissonCI | 0.2 | Zero overshoot, weak reaction |
| CkpoolRemedy-ft12 | 60 | 300 | 12 | PoissonCI | 0.2 | Better reaction, more overshoot |
| Ck-tl120-eta20 | 60 | 120 | 12 | PoissonCI | 0.2 | **Best balanced** |
| Ck-tl120-eta35 | 60 | 120 | 12 | PoissonCI | 0.35 | Accuracy/overshoot degrades |
| Ck-cusum-eta20 | 60 | 300 | 12 | AsymCUSUM | 0.2 | 96-98% reaction, high jitter |
| Ck-tl120-cusum-eta30 | 60 | 120 | 12 | AsymCUSUM | 0.3 | Reaction good, fitness poor |
| Ck-ts30-tl120-eta30 | 30 | 120 | 8 | PoissonCI | 0.3 | Shorter τ_short helps ramp |
| Ck-tl120-accel | 60 | 120 | 12 | PoissonCI | 0.2→0.6 | Overshoot too high from η ramp |

### Results at SPM 4-5 (where ckpool's ideas matter most)

| Variant | Reaction -50% | Accuracy p50 | Overshoot p99 | Jitter | Fitness |
|---------|:---:|:---:|:---:|:---:|:---:|
| FullRemedy | 70 / 82% | 7.6 / 7.4% | 7.7 / 7.1% | 0.042 / 0.032 | **0.691 / 0.732** |
| Ck-cusum-eta20 | **96 / 98%** | 9.3 / 9.1% | 24.5 / 15.1% | 0.158 / 0.140 | 0.612 / 0.644 |
| Ck-tl120-eta20 | 67 / 77% | 9.7 / 7.4% | 20.1 / 19.3% | 0.065 / 0.057 | 0.656 / 0.675 |
| CkpoolRemedy | 55 / 53% | 9.1 / 7.7% | **0 / 0%** | **0.009 / 0.031** | 0.679 / 0.682 |
| VardiffState | **99 / 99%** | 12.3 / 11.8% | 44.4 / 32.4% | 0.143 / 0.118 | 0.626 / 0.658 |
| AdaCUSUM η=0.2 | **99 / 99%** | 8.4 / 6.4% | 29.5 / 28.2% | 0.220 / 0.192 | 0.633 / 0.662 |

### Key Findings

1. **The boundary is the decisive axis.** CUSUM gives 96-99% reaction at
   the cost of 3-5× jitter. PoissonCI gives low jitter at the cost of
   67-77% reaction. No ckpool estimator tuning escapes this trade-off —
   it's the same fundamental boundary-axis trade-off that differentiates
   FullRemedy from AdaCUSUM.

2. **Shorter τ_long (120s) helps more than lower fast-threshold.** Once
   the long-window EMA is responsive enough (τ=120s), the dual-window
   switching becomes redundant — you're effectively running a single
   EWMA(120s) that happens to have slightly different decay dynamics.

3. **Higher η degrades everything except reaction rate.** η=0.35 improves
   reaction by ~3% but costs 7% accuracy and 15% overshoot. The
   AcceleratingPartialRetarget variant is even worse (91% overshoot) because
   it ramps η during cold-start when the estimator is already noisy.

4. **CkpoolRemedy (default ft=72) achieves zero overshoot** because the
   long-window EMA (τ=300s) dampens cold-start ramp so aggressively that
   the target never overshoots truth. But this same conservatism kills
   reaction rate (55% at SPM 4).

## What Transfers and What Doesn't

### Transfers (worth keeping)

- **Per-share `decay_time()` simulation** — the correct technique for
  porting continuous-time EMAs to tick-based evaluation. Available via
  `CkpoolEstimator` for future composition experiments.

- **Oscillation guard** — `CkpoolRetarget`'s suppression of difficulty
  decrease on insufficient data is a sound principle, though
  PartialRetarget's damping (η=0.2) already limits per-fire moves enough
  to make the guard redundant in practice.

### Does not transfer

- **Time-bias warmup correction** — calibrated for per-share evaluation
  intervals (~3.33s); catastrophic at 60s ticks. The per-share simulation
  makes it unnecessary.

- **Wide hysteresis band [0.5, 1.33]** — designed for a context where the
  estimator converges tightly before the gate opens. At 60s ticks, the
  estimator is noisier when evaluated, so the band must be narrower or
  replaced entirely with a statistical boundary.

- **Dual-window adaptive switching** — an elegant idea in ckpool's native
  context (short window for fast ramp-up, long window for stability), but
  once τ_long is shortened to match the tick cadence, the switching adds
  complexity without performance benefit. A single EWMA(120s) achieves the
  same balance.

- **Share-count data gate (72 shares)** — meaningless in the tick framework
  where evaluation happens on a fixed schedule regardless of share arrival.

## Conclusion

ckpool's vardiff is well-optimized for its native per-share evaluation
context but yields no Pareto improvement over FullRemedy in the tick-based
SV2 framework. The per-share simulation technique is the correct way to
port continuous-time EMAs and is preserved in the codebase as
`CkpoolEstimator`. The production recommendation remains unchanged:

```rust
Composed::new(
    EwmaEstimator::new(120),
    PoissonCI::default_parametric(),
    PartialRetarget::new(0.2),
    min_allowed_hashrate,
    clock,
)
```

## Files Added

### Production components (`src/vardiff/composed/`)

- `estimator.rs` — `CkpoolEstimator` (per-share `decay_time()` simulation),
  `TimeBiasEwmaEstimator` (single-window EWMA with time-bias correction)
- `boundary.rs` — `HysteresisGate` (ckpool's binary fire/no-fire gate)
- `update.rs` — `CkpoolRetarget` (full retarget with oscillation guard)

### Grid registrations (`sim/src/grid.rs`)

- `AlgorithmSpec::ckpool()` — native ckpool composition
- `AlgorithmSpec::ckpool_remedy()` — ckpool estimator + PoissonCI + PartialRetarget
- `AlgorithmSpec::ckpool_remedy_ft(n)` — with custom fast-threshold
- `AlgorithmSpec::ckpool_narrow_hyst()` — tightened hysteresis variant
- `AlgorithmSpec::ckpool_with(τs, τl, lo, hi)` — parametric exploration
- `AlgorithmSpec::time_bias_remedy()` — isolated time-bias test
