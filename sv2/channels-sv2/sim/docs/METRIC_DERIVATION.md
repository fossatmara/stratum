# Vardiff at the Information Floor: What a Share-Rate Controller Can and Cannot Do

**Abstract.** A variable-difficulty (vardiff) controller is unusual: the only
thing it can measure is its own tracking error. We take that one fact to its
conclusion and arrive at a result that is structural, not a horse race. Two
theorems — a *measurement identity* (the observable depends on hashrate only
through the controller's error) and an *information floor* (the error estimate's
variance is Cramér–Rao-bounded by `1/(r*τ)`) — together imply that across the
operating band (`r* ≈ 4–30` spm) **every reasonable controller is pinned to the
same floor**: the spread between best and worst is a narrow band in composite
cost — about 12% — the residual differences are *gentleness and safety* rather
than *agility*, and the one lever that moves the floor is the share rate `r*`
itself. The metric we use
to score algorithms — **linear sign-split tracking regret + direction-split
effort, per scenario class, with detection reported separately as a
rate-dependent diagnostic rather than folded into the production score** — is
derived from the same two theorems; an earlier version that scored detection
inside the production scalar was corrected after measurement showed the term is
floor-saturated (carries no ranking signal) below ~10 spm.

The champion we ship, `Ewma360/s1.5`, is **not the hero of this story**: it is
the *existence proof* that the safe corner of the achievable frontier is
occupiable. It is deliberately the **gentlest decline-safe configuration**, and
it pays for that with slow transients — ~65-min cold-start ramp and ~28-min
detection latency at 12 spm — which read as a regression only if one expects "our
best algorithm" and read as the thesis once one expects "the gentlest config
that is provably safe on a sustained decline." The discipline that produced this
is itself part of the result: three candidate hero claims were drawn and killed
by measurement before they could be published (§8), and the surviving claims are
trustworthy precisely because the casualties are named rather than buried.

Scope, stated once and honestly: the *behavioral* layer (counter-age blindness,
champion-beats-incumbent on a live drop) is confirmed on real hardware. For the
present champion specifically, the decline response is hardware-confirmed *in
direction* — it eases the safe way on a sustained 50% drop, with no rejection
spike — while its settled over-difficulty figure and its behavior on a slow,
moderate decline (the regime the safety gate binds on) remain simulation results
(§9.4). The cost-model weights remain a calibrated judgment.

Convention: *Theorem* and *Lemma* are reserved for results genuinely proved from
the model (Theorems 1–2, the §4 Lemma); everything else load-bearing is
an *Argument*, *Rationale*, or *Choice* — our reasoning or our decisions — and
*Observation* is a fact about particular algorithms established in simulation
(named inline). Mathematical claims are never "validated numerically": they are
proved or they are not.

Note on terminology: *regret* here means the time-integrated tracking loss
`∫|e|`, **not** online-learning regret against a comparator; read it as "tracking
loss."

---

## 1. The problem, and the model

Variable difficulty exists because the miners on one pool span many orders of
magnitude in hashrate, while the pool needs every connection to deliver shares at
roughly the same rate. A share is a proof-of-work that clears a per-connection
difficulty `D`; a miner of hashrate `H` clears it and submits valid shares at a
rate proportional to `H/D`. A single global `D` would bury the pool under shares
from its biggest miners and starve its smallest of shares entirely — and starving
a connection of shares means no timely hashrate estimate for it and a
high-variance, lumpy reward. Per-connection vardiff fixes this by moving each `D`
so that connection's share rate stays near a target `r*`.

That makes `r*` the one design constant doing three jobs at once: it caps the
bandwidth and CPU each connection costs, it fixes the variance of the pool's
per-miner hashrate estimate, and it bounds the miner's reward variance. The
controller's whole job is to hold the realized rate at `r*` as `H` drifts. **`r*`
is also, as the rest of this document shows, the only quantity that moves the
fundamental limit on how well any controller can do that job** — which is why it
recurs from the model here (§1) to the floor (§3) to the lever (§8). A connection
delivering only a few shares per minute has a hashrate estimate so noisy (its
band is `1/√(r*τ)`, §3) that reward variance and detection both suffer regardless
of vardiff — which is *why* `r*` is set above that floor, and why the
controller's worst regime (§9, the sub-guard analysis) is one the system is
provisioned to avoid.

Two modeling choices fall out of the physics; neither is a free knob. **Share
arrivals are Poisson** because each hash is an independent trial with a tiny
success probability against an enormous number of trials per window, so over any
window short enough that `H` is steady, the count is Poisson with rate equal to
hashrate over work-per-share. Under `D = Ĥ/r*` that rate is `r_obs = r*·H/Ĥ` —
equation (1) — with the proportionality constant folded into the units of `D`,
which is why nothing downstream depends on it. And **the natural coordinate is `e
= ln(Ĥ/H)`** because the controller only ever acts on `D` multiplicatively, the
only thing it can observe depends on `Ĥ` solely through the ratio `H/Ĥ`, and only
in log units is a 2× over-difficulty the same distance from target as a 2×
under-difficulty. Working in `e` turns a retarget into an additive step `s` and
makes the two signs honest, symmetric coordinates instead of artifacts of where
you sit on the difficulty axis.

This is an *evaluation* model, not a control-design model. To rank algorithms we
need only the path each produces — the sequence of `(error, did-it-fire,
step-size)` — and the Poisson-in-log model produces exactly that: cheaply,
repeatably, and identically for an algorithm whose internals we can read and one
we cannot, because the only signal any controller has is a function of `e` alone
(Theorem 1). That is what lets one metric rank the whole field without privileged
knowledge of anyone's implementation.

What the model leaves out — because the metric covers exactly the situations the
model can express. It assumes `H` is constant within a window (true over one
retarget, false across a real ramp, which is why scenarios drive `H(t)`
explicitly); one worker per connection (real connections pool many workers,
raising the effective rate and reshaping the noise); retargets that are instant
and lossless — which, against this implementation, is exact, not an idealization:
a retarget rejects no in-flight work (shares validate against a per-job target
snapshot, §6), so there is no transition cost to model; and a continuous
difficulty with no floor and no rate limit on how often it can change (real pools
clamp `D` from below and cap fire cadence). The cadence cap and the churn/usability
cost land on the effort term and are taken up in §7.
Anything beyond this list is a coverage gap to declare, not a hole in the
derivation.

### Setup

A **miner** has true hashrate `H > 0`. The **controller** holds a belief `Ĥ > 0`
and sets difficulty `D = Ĥ/r*`, where `r*` is the target share rate
(shares/min). Shares then arrive as a Poisson process of rate

```
  r_obs = r* · H/Ĥ.                                                  (1)
```

Over a window of `τ` minutes the controller sees a single number: the share count
`N ~ Poisson(λ)`, `λ = r_obs·τ`. It periodically **fires** — rescales `Ĥ` by a
factor, i.e. adds a **log-step** `s = ln(Ĥ⁺/Ĥ⁻)` to its belief; `s > 0`
*tightens*, `s < 0` *eases*.

**Definition 1 (error coordinate).** The controller acts multiplicatively (it
scales `Ĥ`) and, by (1), the observable depends on `Ĥ` only through the ratio
`H/Ĥ`. A multiplicative quantity is linearized by its logarithm, and only in log
coordinates are an over- and an under-shoot by the same *factor* equidistant from
the target. So work in

```
  e = ln(Ĥ/H),     so that     r_obs/r* = e^{−e}.                    (2)
```

`e = 0` is exact; `e > 0` is over-difficulty, `e < 0` under-difficulty. The
control goal is `e → 0`.

A **scenario** specifies `H(t)` and the initial belief; the named classes
(`baseline.rs`) are *cold-start*, *stable*, *step ±p%*, *settled-aged drop*
(truth holds long enough to mature the share counter, then drops), and *sustained
decline* (a continuous `−ρ %/hr` ramp — the safety scenario of §9). A **metric**
ranks algorithms by averaging over a fixed ensemble of classes, **per class,
never pooled**, and — as §3 forces — **per share rate `r*`**, because `r*` moves
the floor that everything else is measured against.

---

## 2. The controller measures only its own error

**Theorem 1.** *The sole observable `N` has a distribution depending on `H` and
`Ĥ` only through the error `e`.*

*Proof.* By (1)–(2), `N ~ Poisson(r*τ·e^{−e})`. The parameter is a function of
`e` alone. ∎

There is no measurement of the miner separate from the error being controlled.
Every familiar quality statistic — accuracy, jitter, reaction, overshoot — is
therefore a functional of the one scalar process `e(t)` together with the fire
sequence the controller derives from it. This is why a small set of `e`-based
terms can suffice, and why the metric is written entirely in `e` and `s`.

---

## 3. Precision costs time, and time costs agility — the result everything rests on

**Theorem 2 (information floor).** *Any unbiased estimate of `e` from a window of
`τ` minutes has `Var(ê) ≥ 1/(r*τ)`.*

*Proof.* The window's count is `N ~ Poisson(λ)`, `λ = r*τ·e^{−e}`. For a Poisson
observation the log-likelihood in `e` is `N ln λ − λ`, with score `∂_e(N ln λ −
λ) = (N/λ − 1)·λ' = λ − N` since `λ' = −λ`. The Fisher information is `E[(λ−N)²] =
Var(N) = λ`; at the operating point `e≈0`, `λ = r*τ`. Cramér–Rao gives `Var(ê) ≥
1/(r*τ)`. ∎

(The crate encodes this as `SettledAccuracy::poisson_floor`, `metrics.rs:864` —
`1/√(r*τ)` as a percentile.)

This is the load-bearing result of the entire document. Read structurally, it
says the controller's information per unit time is fixed by `r*` alone; a
controller can spend that information on precision (long window) or on agility
(short window), but it cannot manufacture more of it. Three consequences follow,
and they are the spine of everything below.

**Corollary (the central trade-off).** Steady-state precision improves only by
enlarging the averaging window `τ`; but the same `τ` is the lag in following a
real change. **Accuracy and agility are bought from one budget at a fixed rate
`r*`.**

- *The apparent quality axes are one trade-off.* The estimator's window trades
  accuracy against lag; the fire threshold trades false alarms against detection
  delay; the retarget gain trades convergence against overshoot. Each is one knob
  on the same accuracy-vs-agility line. Scoring six such projections with equal
  weight (the deprecated `EqualWeightFitness`) rewards the *middle* of the
  trade-off curve, not the *frontier*. **Observation (commit `31a9dbc1`):** four
  independent parameterizations of the pipeline all saturate at the same quality
  ≈0.55 — one wall seen four ways. So score the frontier's own coordinates:
  **tracking error** and **control effort**, nothing derived from them.
- *A steady offset has a price, not a defect.* An algorithm sitting short of the
  floor is paying for it elsewhere (agility, effort); §10 uses this.
- *The field is flat across the operating band; detection is floor-limited at
  production rates.* This is the structural headline, and it is a direct reading
  of the bound: when `r*τ` is small (production rates, monitoring-length windows),
  the noise band
  `1/√(r*τ)` is wide enough that controllers differing in window or threshold
  produce nearly the same achievable tracking error and nearly the same
  (in)ability to see a small change. §8 measures this directly — a ~12% best-to-
  worst spread in composite cost across the 4–30 spm operating band, and a
  detection signal that is statistically zero below ~10 spm and opens up only as
  `r*` rises (Figure, §8.4: the lever).

The floor `1/√(r*τ)` is a **noise band** — one standard deviation of an
*unbiased* estimator — not a systematic offset. When §8/§10 quote an accuracy
ceiling, that is the *width* of this band at the operating `r*τ`, the scatter a
perfect tracker still shows, against which a real algorithm's *systematic* offset
(§10) is measured.

**The achievable frontier, in one line each.** Two floors follow from Theorem 2
and bound what *any* algorithm can do; the §9 sub-guard analysis places the
champion against them.

- *Static (stable load).* On steady `H`, the cost-minimizing settled offset under
  the §6 asymmetry is not zero but a quantile of the noise band: `e* ≈ −0.67·σ`,
  `σ = 1/√(r*τ)`. At 60 spm, `σ≈8%`; at 2 spm with `τ≈2.5 min`, `r*τ≈5` so
  `σ≈45%` — the band swamps any few-percent offset, i.e. at low share rate the
  asymmetry-optimal target is itself lost in the noise.
- *Dynamic (declining load).* On a ramp of `ρ` per minute, an estimator of
  effective averaging time `τ_eff` lags truth by `≈ ρ·τ_eff`, while its noise is
  `1/√(r*τ_eff)`. Shrinking `τ_eff` to cut the lag widens the noise; their sum
  `L(τ_eff) ≈ ρ·τ_eff + z/√(r*τ_eff)` has a floor at `τ_eff ∝
  (z/ρ)^{2/3}·r*^{-1/3}` — a minimum tracking error no algorithm beats on a
  decline of that rate and share count. This is the bound the §9 sub-guard cells
  are read against, and it is the physics behind the τ-safety-valley of §8: too
  long a window lags a sustained decline into the dangerous direction, too short
  a window is too noisy to act on, and the safe window is the minimum of `L`.

---

## 4. The error norm: squared is blind, linear is not

Tracking cost is `∫ f(e) dt` for some norm `f`. The choice of `f` is a judgment
about how harm scales with error — but one judgment is inadmissible.

**Lemma (blindness of the square).** *A persistent, undetected fractional
hashrate drop of size `g` produces a steady error `e = −ln(1−g) = g + O(g²)`.
Under `f(e)=e²` it costs `e² = g² + O(g³)`; under `f(e)=|e|` it costs `|e| = g +
O(g²)`.*

*Proof.* The drop sends `H → (1−g)H` with `Ĥ` fixed, so `e = ln(Ĥ/((1−g)H)) =
−ln(1−g)`; expand. ∎

Operational harm from a difficulty error (lost or excess work) scales with its
**magnitude** `g`, i.e. linearly. The squared norm undervalues it by a factor
`1/g`, which diverges as `g → 0`: a small *persistent* leak — a failing or
throttling ASIC, the case operators care about most — is essentially free under
`e²`. **Observation (`regret-effort`):** an algorithm that detects a −10% drop
~1% of the time scores *better* on `e²` than one that detects it always, because
the miss costs only `(ln0.9)² ≈ 0.01`/min. The linear norm removes this blind
spot (the same miss costs `≈0.10`/min) and only reorders the middle of the
ranking, never the top.

**Choice 1.** Use `f(e) = |e|`. Report it split by sign, since the two signs
carry different harm (§6):

```
  regret_over  = ⟨|e|⟩ over time with e > 0       (over-difficulty)
  regret_under = ⟨|e|⟩ over time with e < 0       (under-difficulty)
```

---

## 5. Detection is a separate axis — and at production rates the floor flattens it

Linear regret narrows the blind spot but does not close it: a fast algorithm with
occasional large errors can still outscore a chronically blind one on `∫|e|`. The
deeper reason is structural.

**Argument (detection is not derivable from the scored error paths).** *There is
no functional `F` with `detection = F(e on stable, e on step)` holding across all
algorithms.*

*Why.* Catching a small drop requires the share counter to be *young* when the
drop lands: a matured counter averages the weak post-drop signal against a long
pre-drop baseline and never crosses threshold. Counter age at the drop is
determined by the fire history of a *matured, on-target* loop — a state the
stable scenario (never perturbed) and the step scenarios (perturbed while young
or with a large signal) never enter. Two controllers can therefore produce
*identical* stable and step error paths yet differ on detection, because they
differ only in that matured-counter regime. So detection must be carried
explicitly:

```
  detection = P[ fire within W min | counter matured, then −g drop ].
```

**But detection must be measured against its own false-alarm rate, and at
production rates that correction sends it to zero.** The honest quantity is not
the raw fire probability — which a twitchy controller inflates by firing often
regardless of any drop — but the **excess**:

```
  EXCESS = P[fire within W | −g drop] − P[fire within W | no drop].
```

**Observation (`detection-control`, `excess-lever`):** at production rates the
information floor (Theorem 2) is so coarse that a −10% drop is statistically
invisible within a monitoring window — `EXCESS = 0.00` at a 60-min window and
`+0.05` at a 15-min window at 4–6 spm, *for the whole field*. The raw detection
number is then an artifact of fire cadence (the window straddles scheduled
settling fires whether or not a drop occurred), which is exactly the trap an
earlier version of this metric fell into. The correction has two parts, and both
matter:

**Choice 2 (corrected).** *Detection is removed from the production score.* Below
~10 spm it is floor-saturated (Theorem 2) and carries no ranking signal; folding
it into the scalar there merely rewards twitchiness. It is instead reported as a
**rate-dependent diagnostic** — `EXCESS` versus `r*` — where it does discriminate
(§8, the lever): the same `EXCESS` climbs monotonically to `+0.75` at 60 spm as
the floor recedes. Detection thus stops being a scoring axis and becomes the
visible measure of what raising `r*` buys.

---

## 6. The two directions are not symmetric

**Argument.** *Over-difficulty is worse than under-difficulty, and a controller
should be reluctant to tighten and eager to ease.*

*Why — the operating-point asymmetry (§6(i)), which is the whole basis.* `e < 0`
(difficulty low): shares run a little fast; all work stays valid; cost is mild
inefficiency, and it is the *safe* side. `e > 0` (difficulty high): the connection
is starved of valid shares, which inflates both the miner's reward variance and
the pool's hashrate-estimate variance for it — risking an offline misread — and
compounds when `H` is genuinely falling (the death-spiral, §9). This grounds *both*
halves of the asymmetry directly, without any appeal to transition cost:
- **Eager to ease** (`s<0` fires readily) is death-spiral avoidance — the exact
  mechanism the §9 safety story rests on: when `H` falls, ease fast to follow it
  down rather than starve the miner.
- **Reluctant to tighten** (`s>0` requires more evidence) suppresses false
  excursions to the dangerous over-difficulty side, and when it lags a genuine
  hashrate *increase*, it lags into *under*-difficulty — the benign, safe-but-
  slightly-wasteful side. So tighten-reluctance is also a safety bias.

**Killed premise — the lost-in-flight-work argument (was §6(ii)).** Earlier
versions justified the tightening penalty with a *proof* that a tightening fire
invalidates in-flight shares aimed at the old target (fraction `1−e^{−s}` lost).
**That is false against the implementation and is retracted.** A retarget mutates
only the channel's current target (`extended.rs:341`); each job snapshots the
target *at its creation* into `job_id_to_target` (`extended.rs:510/549/668`), and
shares are validated against that per-job snapshot (`extended.rs:724`), not the
current target, until the map is cleared on a new prev-hash (`extended.rs:586`).
So a difficulty change rejects no in-flight work — old jobs stay valid until the
next block — and it does not even force a job switch (it rides the normal job
cadence), so there is no pipeline flush to charge for. Moreover, **share value is
proportional to difficulty**: a share clearing a higher threshold is worth
proportionally more, and a miner's credited work-rate equals its hashrate
regardless of the difficulty the pool sets, so even a genuinely rejected
low-difficulty share is not lost *earnings*. In expectation, churn costs the miner
**no value at all**. Its only real costs are *variance* (overshoot into transient
over-difficulty — which is §6(i), the safe-vs-dangerous-side concern, not a
transition cost) and *usability* (rejections read as errors and muddy monitoring).

**Choice 3 (re-homed and split).** The asymmetry's *existence and direction*
stand on §6(i) safety, as above — not on lost work and not on simulation fitness.
But the three "3:1" weights the paper carried were never one thing, and the killed
premise lands surgically on exactly one of them:
- **`regret_over:regret_under` (operating-point regret weighting): survives.**
  Pure §6(i) — over-difficulty is the dangerous side. Unaffected.
- **`tighten_multiplier` (boundary evidence asymmetry): survives, re-homed.**
  Directional reactivity for safety (eager-ease / reluctant-tighten, above). The
  carve-out does not touch it.
- **`effort_up:effort_down` (the effort-*direction* asymmetry): loses its basis.**
  This one was justified by lost in-flight work; with churn value-neutral, there
  is no transition-cost reason to charge an upward step more than a downward one
  of equal size. Its directional split is retired.

*Magnitude is a tuning judgment, and the finding argues it down.* The `3.0`
multiplier leaned on a fitness function weighting "stability" — which bundles
`step_magnitude_safety` (overshoot → over-difficulty; pure §6(i), fully
justified) with `jitter` (fire frequency). Value-neutrality specifically guts the
*jitter* half: if firing costs no value, penalizing it heavily rests only on
usability and minor pool overhead — much softer than a value cost. **Observation
(`champion-weights`):** the best algorithm is the same for every ratio in `[1:1,
4:1]`, and under balanced weights `1.5–2.0` ranks at least as well; only an
ungrounded `5:1` changes the ranking. So the ranking does not hinge on the exact
value, and the deflated jitter cost argues for the lower end of that range. The
price of a smaller multiplier is being slower to track a genuine hashrate
*increase* — but that lag lands on the benign under-difficulty side, so it is a
price worth paying. The asymmetry's *direction* is safety-load-bearing; its
*magnitude* is a soft tuning choice, no longer propped up by a transition cost
that does not exist.

**The retraction was stress-tested by a full champion-hunt reopen, and the
champion held — but the headline is *why*.** `sweep-recalibrated.rs` re-ran
selection under the corrected cost (effort-direction forced to `1:1`; `λ` swept
*and* dropped), with the decline gate as the selector. **The load-bearing
result: selection never rested on the cost weighting in the first place.** With
`λ`'s anchor gone the *cost*-optimal config is now `λ`-sensitive across the whole
window range (it swings from `Ewma150/s0.3` at `λ=0` to `Ewma720/s2` at `λ=2`) —
so a champion defined by the cost would be arbitrary now. But the champion was
defined by the *binding decline-safety gate* (§9), which is pure §6(i)
operating-point safety and untouched by the retraction. The cost-leaders the
recalibration favors at the long-window end fail that gate **decisively**:
`Ewma720/s2` (the `λ=1–2` cost-winner) settles at **+9.6%** over-difficulty at
the worst sub-guard cell — multiple cells well over the 5% gate — while the
champion `Ewma360/s1.5` passes comfortably at **+2.7%**. (Honest margin note:
the short-window `λ=0` cost-winner `Ewma150/s0.3` lands *at* the gate line —
exactly +5.0% in a single worst cell, indistinguishable from the threshold — so
it is gate-*indeterminate*, not a decisive failure; the vindication rests on the
unambiguous +9.6% rejection, not on the boundary configs.) So the soft `λ` was
always free to wander; it simply never mattered, because the *gate*, not the
cost, was doing the selecting all along — a clean vindication of constraint-over-
cost. One nuance kept visible: a shorter window, `Ewma240/s1.5`, also passes the
gate (+3.5%), so the champion's specific `τ=360` is now justified as the *safest*
gate-passer (2.7% vs 3.5%) and by the §8.3 τ-valley floor — not by any cost
consideration, which `λ` no longer anchors.

*The under-difficulty side has its own cost, and the metric only partly prices
it.* Under-difficulty (`e<0`) is not free: at the realized rate `r = r*·e^{−e}`,
an `e=−0.07` offset runs the connection ~7% over its target share rate,
permanently — and bounding exactly that per-connection load is *why* `r*` is set
where it is (§1). The resource cost (extra bandwidth, CPU, share-accounting) is
linear in excess volume, and since `r − r* ≈ −r*·e` near the operating point,
**linear in excess volume is to first order linear in `|e|`** — so it adds no new
functional form, only a one-sided increase to the `regret_under` coefficient. The
clean way to settle it is one measured number — marginal cost `c` per extra share
from share-accounting telemetry — added as `c·r*·max(0,−e)`; this is left as the
one external-economics input the simulation cannot supply.

---

## 7. The metric

Per scenario class **and per share rate `r*`**, from the trajectory `{(e, fired,
s)}` alone — hence computable for every algorithm, transparent or opaque
(`LogErrorRegret`, `metrics.rs`):

```
  regret_over, regret_under  =  ⟨|e|⟩, split by sign of e            (§4)
  effort_up,  effort_down    =  Σs² and Σ|s|, split by sign of s     (below)
  [diagnostic] EXCESS(r*)    =  P[fire|drop] − P[fire|no drop]       (§5, NOT scored below ~10 spm)
```

The **vector is the primary object**; for ranking, the *production* scalar is

```
  cost = 3·regret_over + 1·regret_under
       + ρ·( (3·effort_up + 1·effort_down)_quadratic
             + λ·(3·effort_up + 1·effort_down)_linear ),     ρ = ½.
```

Two things changed from the earlier version, both forced by measurement:
**detection is no longer in the scalar** (§5 — floor-saturated at production
rates), and **effort now carries a linear `Σ|s|` term alongside `Σs²`**. The
linear term closes the dual of §4's blind spot: hold `Σs²` fixed, shrink each
step's amplitude, and raise the fire frequency, and `Σ|s| → ∞` while `Σs² → 0` —
so high-frequency, low-amplitude churn would otherwise score as "gentle," which
the quadratic term cannot see. That blind spot is real and the linear term still
closes it. What the term does **not** rest on — and an earlier version wrongly
claimed it did — is *lost work*: §6 retracts the lost-in-flight-work premise (a
retarget rejects no in-flight shares, and churn is value-neutral), so `Σ|s|` is
**not** "cumulative lost work in regret's currency." It is a **churn/usability
penalty** (frequent retargets read as errors, muddy monitoring, cost minor pool
overhead) — a real but *soft* cost, not a value cost. Consequently `λ` is **not**
anchored at `λ=1` "at face value"; it is a soft tuning weight. The quadratic term
still does its own job (penalizing overshoot and concentrated actuation — one
large retarget costs more than several small ones, `S² > k(S/k)²`, which is the
§6(i) variance/over-difficulty concern). The champion is stable across `λ ∈
{0,½,1,2}` (§9), so the deflation of `λ`'s justification does not move the result.

*Assumption (fire cadence is capped).* Real pools forbid the churn corner with a
**minimum inter-fire interval**, which the model adopts as an explicit
assumption; with the linear effort term *and* the cadence cap, the gentleness
reading of effort is honest from both sides.

**Per class and per rate, never pooled.** Cold-start cost dwarfs steady-state
cost, and the floor moves with `r*`, so a pooled average erases every distinction
that matters. This is not bookkeeping: §8 shows the champion is selected by a
*minimax over `r*`* — best worst-case across the band — precisely because no
single rate's ranking is the answer.

---

## 8. What the figures show — and what we tried to show and could not

The metric is a vector per class per rate. The hard part is not drawing it; it is
knowing *what claim each picture is allowed to make*. This investigation killed
three candidate principal figures by measurement before any of them was
published, and that record is not an embarrassment to hide — it is the strongest
evidence that the harness was permitted to invalidate its own conclusions. We
state the casualties first, then the survivors.

### 8.1 Three premises drawn and killed

- **The floor as an estimator bound under the field.** The natural hero figure
  was "performance versus `r*`, every controller hugging the Poisson floor from
  above." It is false as drawn: the cost-blind maximum-likelihood estimator (the
  policy-free controller that fires every tick and emits its raw belief) sits
  *above* the field, not below it — a real controller *holds* difficulty between
  fires, low-pass-filtering the very noise the every-tick MLE emits. The floor is
  not a line beneath emitted-difficulty error, so the figure cannot be drawn that
  way.
- **A thin ribbon in steady-state error.** The flatness is real but does not live
  on the steady-RMS axis: there the field spreads +89–161%, and worse, that axis
  crowns *classic* (which holds an effectively enormous window and so tracks a
  stable stream tightly while failing every transient and the safety gate). The
  ~12% flatness lives in the *composite* cost across rates, not in steady error.
- **The champion as the Pareto frontier.** On a steady-vs-transient scatter the
  champion is *inside* the field's Pareto front — five configurations are cheaper
  on steady cost *and* faster on transient lag. It becomes the frontier only once
  the points are colored by safety (§8.2): all five dominators fail the
  cross-rate decline gate. The champion is the *safe* frontier, not the cost
  frontier — a weaker and more honest claim.

Each premise died to the same discipline: render the measured points before
writing the caption. The figures that survived did so because they were checked
the same way.

### 8.2 The companion: the champion is the safe frontier (steady-vs-transient scatter)

`steady-transient.rs` plots every configuration at a fixed rate as a point: x =
steady-state cost (tracking + effort on a pure stable stream), y = transient lag
(cold-start ramp + aged-drop detection latency), each point colored by
decline-safety — the worst settled over-difficulty after a sustained decline,
measured over the *authoritative* rate×magnitude grid (identical to the safety
gate of §9; an earlier partial grid mis-colored the long-window family and was
corrected). Lower-left is better on both axes.

*What it shows.* A clean convex frontier exists, parametrized by the estimator
window. The champion is the **lower-left-most safe point**: the five
configurations that beat it on both axes all fail the decline gate (they are red)
— so no *safe* configuration dominates it. The safe configurations themselves
trace a convex envelope, and the champion is its gentle-steady corner. This is
the minimax-over-`r*`-plus-safety selection made visual: the champion is chosen
not because it wins the field but because it is the gentlest configuration that
is *safe everywhere*. Confirmed at 12 and 30 spm; at 4 spm the field is
window-degenerate (configurations of the same window collapse to one point, so
the neighborhood cannot be resolved there) — stated rather than implied.

![Steady cost vs transient lag at 12 spm. Each point is a configuration; green =
decline-safe (worst settled over-difficulty ≤ 5% over the cross-rate grid), hollow
red = fails the gate. The new champion (Ewma360/s1.5) is the lower-left-most green
point; the five configurations below-left of it — cheaper on steady cost *and*
faster on transient lag — are all red, so no *safe* configuration dominates it.
The dashed line is the envelope of the safe configurations.](steady_transient.svg)

### 8.3 The mechanism: why the champion's window is what it is (the τ-safety-valley)

The scatter shows cheaper-and-faster points colored red with no in-panel reason.
`tau-valley.rs` supplies the reason: worst settled over-difficulty after a
sustained decline is a **U-shaped function of the estimator window τ**, with its
minimum at the champion's window. Too long a window (sleepy) lags a sustained
decline into the dangerous over-difficulty direction; too short a window
(twitchy) overshoots it; the safe band is the minimum of the dynamic floor
`L(τ_eff)` from §3. **Observation:** the curve is *sensitivity-invariant* — worst
settled error is identical to 0.1% across boundary sensitivities `s∈{0.3…2}`,
because settled error after recovery is an estimator-window property, not a
firing-threshold one (a per-fixed-sensitivity sweep controls the window×threshold
confound: the floor sits at the champion's window at *every* fixed sensitivity).
This converts "we picked this window" from a selection outcome into a visible
physical reason.

![Worst settled over-difficulty (over the cross-rate decline grid) versus
estimator window τ. The curve is a U floored at the champion's window (τ=360,
ringed): both flanks — sleepy long windows that lag a sustained decline, twitchy
short windows that overshoot it — rise above the +5% runaway gate (dashed). The
green band is the safe region. The valley is sensitivity-invariant (identical to
0.1% across boundary sensitivities s0.3–s2), so it is a genuine window effect, not
a window×threshold confound.](tau_valley.svg)

### 8.4 The lever: raising `r*` buys agility (EXCESS vs `r*`)

The structural claim's other half — *the one lever that moves the floor is `r*`*
— is carried by `excess-lever.rs`: false-alarm-corrected detection `EXCESS` of a
−10% drop versus share rate. The honest object is detection-excess, **not** the
composite cost (which is non-monotone in `r*` and so cannot show a clean lever)
and **not** the stable-safe-window result (which is a *robustness* claim about
the champion, not a claim that `r*` buys anything). `EXCESS` climbs monotonically
from `+0.05` at 4 spm to `+0.75` at 60 spm, and the whole field is bunched near
the floor at the low end — the floor binds *everyone* at production rates and
recedes for *everyone* as `r*` rises.

*Caption obligation, because "floor-limited" is window-dependent.* At production
rates a −10% drop is at-or-near the detection floor: `EXCESS = 0.00` at a 60-min
monitoring window (the saturation finding — the drop is perfectly invisible) and
`+0.05` at a 15-min window (near-floor). Both numbers must appear or the +0.05
reads as contradicting the ≈0 saturation result, when in fact they are the same
finding at two window lengths.

![False-alarm-corrected detection EXCESS of a −10% drop (15-min window, false-alarm
control held at the same window) versus share rate r* (log axis, window fixed). The
champion's EXCESS climbs monotonically from +0.05 at 4 spm — inside the shaded
production band, where the drop is at the information floor — to +0.75 at 60 spm as
the floor recedes; the field (faint) is bunched near the floor at low r*, so the
limit binds *everyone* at production rates. At a 60-min window the production EXCESS
is 0.00 (the drop is perfectly invisible); the +0.05 here is the same finding at the
tighter window.](excess_lever.svg)

### 8.5 The trajectory, demoted

The single-timeline trajectory plot (`trajectory-plot`) — estimate chasing truth
through cold-start, settle, and an aged drop, with a fire-raster showing
"gentle versus violent" — is kept only as a §8 supporting detail. It is *not* a
principal figure: it shows absolute behavior at one rate, where "slow" is a
property of the rate, not the controller, and it makes the champion's deliberate
gentleness look like a regression. Its one genuine virtue is the fire-raster
(many short marks for the champion versus a handful of tall ones for classic);
that virtue does not earn it the opening of the paper.

---

## 9. Selection, safety, and validation

### 9.1 How the champion was selected (minimax over `r*`)

The target share rate is a static deployment parameter whose value is not known
in advance, so the champion is chosen by a **minimax over `r*`**: the
configuration whose *worst* gap to the per-rate best-in-field, across `r* ∈
{4,6,12,30}` (60 spm as a high-rate anchor, outside the minimax), is smallest.
`sweep-minimax.rs` scores the corrected metric (§7) at each rate independently —
so each configuration's own per-rate false-alarm behavior enters through the
stable-scenario effort term, with no false-alarm convention reused across rates.

Three findings, all consistent with §3. The field is **flat** (~12% best-to-worst
at every rate — Theorem 2 again). The cost-optimal configuration **walks with the
effort weight `λ`**, drifting toward the long-window "sleepy" corner as firing is
penalized more — so a configuration crowned by cost alone would be free-tuned by a
weight grounded only to within a factor. And the band-optimal cost lands in the
same sleepy corner the single-rate search did, so **the decline-safety gate is
the actual selector**, not the cost.

### 9.2 The decline-safety gate (the death-spiral test)

A sustained decline drives `e` positive (over-difficulty), the costly direction;
the death-spiral risk is self-reinforcing starvation. `slow-decline.rs` runs rate
∈ {1–40} %/hr × spm ∈ {2–30}, gating on the *settled* error after a 120-min
recovery window (not the transient trough). The gate is the selector among the
near-tied band-optimal configurations, and it is **uninherited**: a sleepier
easer is a different animal on a decline, so safety is re-cleared from scratch for
each candidate.

**Result.** Among the λ-robust band cluster, **only the champion `Ewma360/s1.5`
has zero runaway cells** (worst settled +2.7%); every sleepier configuration that
beats it on band-cost fails at the sub-guard 2–4 spm cells (settled +5.6% to
+9.6%) — the rates a single-rate or 12-spm view cannot see. The classic incumbent
fails hardest (settled +22%, transiently +109% — a starved miner). The
death-spiral risk is the *incumbent's*, not the candidate's, and the gate
confirms rather than re-selects the champion: the configurations that beat it on
cost are exactly the ones that are unsafe.

### 9.3 The gate is a constraint, not a weighted objective

Decline-safety enters as a **hard constraint satisfied across all plausible
failure magnitudes**, not as a weighted term calibrated to a failure-magnitude
distribution — and this is forced, not stylistic. The magnitude at which the
responsiveness gate would bind is **firmware/config-mix determined and
operator-movable**: an operator can classify miners into similar-profile proxies
and *normalize* the per-proxy distribution, flattening the very magnitude a tuned
controller would target. The pool operators, asked, **do not know** the current
distribution. A controller tuned to a distribution that is both unknown *and*
actively homogenizable would be optimizing against a target that moves out from
under it. So the gate stays a footnote: the champion satisfies it everywhere
(which is how it was selected), and "the distribution is unknown and
operator-movable" is a stronger reason not to tune to it than any measured weight
would have been.

### 9.4 What real hardware validates — and what it does not

Everything above is derived from a model and scored in simulation. The model's
*behavioral* predictions were checked against real miners — an Antminer S21 (~200
TH/s) on testnet4, driven through the **shape-proxy** tool against side-by-side
SRI pool instances.

**The classic algorithm's mechanics reproduced quantitatively:** steady-state
jitter zero over 30+ min; deterministic −16.7% per fire; exact 300 s cadence;
~60% post-staircase overshoot; ±50% symmetry. Most important, the
**counter-age dependence** the §5 mechanism rests on: a 5-min counter reacted in
4.4 min, a 51-min counter in 51.8 min — the matured-counter blindness, seen in
hardware. And a **previous** champion's win reproduced live: deployed beside
classic, both matured overnight, both miners halved at once — classic took hours
and first moved in the *wrong* direction; the champion responded in minutes and
settled correctly (the §5 detection and §6 wrong-direction claims, outside
simulation).

**The hardware re-confirmation — what now holds, and what is still owed.** That
first hardware test was run on the *previous* (`s0.3`) champion. The present
champion `Ewma360/s1.5` is a simulation re-selection under the corrected metric
(detection removed from the scalar, linear effort added, minimax over `r*`,
decline-gate-as-constraint). What transfers by construction is the **mechanism** —
counter-age blindness, the runaway direction, the gentleness/safety trade — all
architectural and rate-driven, not parameter-specific. The specific
parameterization's live behavior does not transfer, and was tested directly.

On a pool-only deployment (so the champion's `VardiffState` governs the miner with
no translator vardiff in the path), a sustained 50% hashrate drop at 6 and 30 spm
produced the §9.2 signature on iron: the difficulty-implied hashrate **eased
downward** to follow the decline — the safe direction, not the death-spiral — with
shares flowing throughout and **no rejection spike** during the drop (the cleanest
tell: a runaway would show rejections climbing as difficulty stayed too high for
the reduced hashrate; there were none). This reproduces for `Ewma360/s1.5` what
PR #2154 showed for the previous champion: a direction-correct, no-starvation
response to a sustained loss.

Two parts of the safety claim remain simulation-only, and the distinction matters:

- *The settled figure, not just the direction.* The dashboard shows the implied
  hashrate easing down and zero rejections — qualitatively safe — but not the
  settled implied-H/true-H ratio the sim reports as `+2.7%`. The run was also
  ~48 min, short of the sim's 120-min settle window: what is confirmed is *easing
  correctly*, not the final settle point. The quantitative match needs
  per-decision logging (pool `vardiff=debug`) over a longer drop.
- *The gate-stress decline, not just the easy one.* A 50% drop is the *large, fast*
  signal — the one every configuration catches quickly (§9.1, the −50% gate was
  non-binding). The death-spiral risk the gate actually binds on is the *slow,
  moderate* decline (the 1–40 %/hr sweep, board-shedding magnitudes ≈25–33%), where
  a sleepy controller lags into over-difficulty. That regime is confirmed in
  simulation only; the hardware test exercised the easy direction, not the gate's
  binding corner.

So the scope is precise: *the present champion's decline response is
hardware-confirmed in direction and starvation-avoidance on a sustained 50% drop;
its settled over-difficulty and its behavior on a slow moderate decline remain
simulation results.* The remaining open hardware tests are a slow moderate decline
on iron with `vardiff=debug` for the settled number, multi-connection operation
(the model assumes one worker per connection), and measuring `c` to close the §6
share-volume term. Production runs at `r* ≈ 4–6` spm with headroom, and the model
supports running faster: a higher `r*` tightens both the detection floor and the
estimate, at a share-volume cost the headroom absorbs.

---

## 10. One consistency check, and how to break the result

**The offset is optimal, not a defect (`confirm-debias`).** The champion sits at a
steady under-difficulty offset, short of the noise-band floor a policy-free
estimator reaches by firing every tick. Multiplying its belief by `b ≥ 1` closes
the offset smoothly, but the cost rises monotonically from `b = 1`:
`regret_under` falls while `regret_over` rises faster under the `3:1` weight. The
unbiased belief is the cost minimum, so the offset is not an error — exactly as §3
predicts. What it *is*, precisely: under the `3:1` asymmetry the cost-minimizing
center of the noise band is not its mean but a quantile below it, `≈ −0.67·σ_eff`;
the `3:1` weight fixes the coefficient and sign, `σ_eff` is set by the window
choice. `confirm-debias` verifies the *quantile condition* (`b=1` minimizes
cost), not the band *width* — so an independent knob that *could* push accuracy
toward the floor is correctly scored *worse*. The metric is self-consistent.

**Falsifiers.** The result should be revised if: (a) some `∫f(e)` on the scored
scenarios reproduces the detection ranking, refuting the §5 Argument; (b) an
*unbiased* estimator beats `1/(r*τ)`, refuting Theorem 2 (biased estimators
routinely beat the CRB on variance, so the qualifier is essential); (c) the
champion changes within `w_over:w_under ∈ [1:1,4:1]` or across `λ ∈ {0,½,1,2}`,
breaking the §6/§7 robustness; or (d) a real failure mode falls outside the
scored ensemble — a coverage gap to declare, not a soundness error. Three further
premises *did* fail and were retired (§8.1); that they were drawn, measured, and
killed is the mechanism by which the survivors earn trust.

**Declared coverage gap (d): the asymmetry-blind sub-guard.** Below spm6 the
guard is a symmetric PoissonCI, so it abandons the §6 safety asymmetry exactly
where data is sparsest — a known, bounded degradation (the offset is inside the §3
noise band there, σ≈45% at 2 spm), not a soundness error. The named fix
(`AsymmetricPoissonCI`, in the codebase) is *deferred*: taking it reopens champion
selection at the margin and owes a spm≥6 re-confirmation, a bad trade for cells
below the operating range. The trigger that would force it: real connection-rate
data showing a non-trivial tail of connections living at 2–4 spm.

---

## 11. Status of each claim

| Claim | Kind | Source |
| --- | --- | --- |
| Observable depends only on `e` | Theorem 1 | §2 |
| Precision floor `1/(r*τ)` | Theorem 2 | §3 |
| Quality axes are one trade-off | Corollary + obs. | §3, `31a9dbc1` |
| Field is flat across `r*` (~12% spread in composite cost) | Observation | §8, `sweep-minimax` |
| Squared norm blind to small drops | Lemma | §4 |
| Detection not derivable from `e(t)` | Argument | §5 |
| Detection floor-saturated at production rates → out of scalar | Observation | §5, `detection-control` |
| Detection EXCESS rises with `r*` (the lever) | Observation | §8, `excess-lever` |
| Over>under, eager-ease/reluctant-tighten — direction safety-justified (§6(i)) | Argument | §6 |
| Lost-in-flight-work premise (old §6(ii)) — RETRACTED: retarget rejects no in-flight shares (per-job target snapshot), churn value-neutral | Killed premise | §6, `extended.rs` |
| `effort_up:effort_down` direction asymmetry retired (rested on lost work) | Killed premise | §6 |
| `regret_over:regret_under` + `tighten_multiplier` survive on §6(i) safety; magnitude a soft tuning judgment, deflated toward `1.5–2.0` | Choice + obs. | `a1d3fa7b`, `champion-weights` |
| Linear `Σ\|s\|` effort term closes the churn blind spot — re-homed as churn/usability cost, not lost work | Argument | §7 |
| Champion = the *safe* frontier (not the cost frontier) | Observation | §8, `steady-transient` |
| Decline-safety is a τ-valley, floored at the champion's window | Observation | §8, `tau-valley` |
| Champion selected by minimax over `r*`, safety as constraint | Choice + obs. | §9, `sweep-minimax`, `slow-decline` |
| Steady under-difficulty offset (`≈−0.67·σ_eff`) is cost-optimal | Observation | §10, `confirm-debias` |
| Counter-age mechanism on real hardware | Observation (HW) | §9, PR #2154 |
| *Previous* champion beats classic on a live drop | Observation (HW) | §9, PR #2154 |
| *Present* champion: decline *direction* HW-confirmed (50% drop, no rejection spike) | Observation (HW) | §9.4 |
| *Present* champion: settled-e and slow-moderate-decline gate still sim-only | Scope note | §9.4 |
| Three hero premises drawn and killed by measurement | Method | §8.1 |

The structural finding — across the operating band the field is flat, the
residual axis is gentleness-and-safety not agility, detection is floor-limited at
production rates, and the one lever is `r*` — is proved from Theorems 1–2 and
confirmed by direct measurement.
The champion is the existence proof that the safe corner of the frontier can be
occupied, selected by minimax over `r*` with decline-safety as a hard constraint.
The behavioral layer — counter-age blindness, the direction-correct response to a
sustained loss — is externally confirmed on real hardware; for the present
champion specifically, that decline response is hardware-confirmed in direction
(a sustained 50% drop, eased the safe way, no rejection spike), while its settled
over-difficulty figure and its behavior on a slow moderate decline remain
simulation results, and the cost-model weights remain a calibrated judgment —
robust over the grounded range and open to those measurements and an economic
backtest.