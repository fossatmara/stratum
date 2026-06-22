# A Derivation of the Vardiff Evaluation Metric

**Abstract.** A variable-difficulty (vardiff) controller is unusual: the
only thing it can measure is its own tracking error. From that one fact we
derive the quantities used to score vardiff algorithms in this crate. Two
theorems (the measurement identity and an information floor) show the
problem reduces to two coordinates — tracking error and control effort.
One lemma shows the obvious error norm (squared) is blind to the failure
operators care about most, forcing a linear norm plus a separate detection
term. A short asymmetry argument fixes the directional weights. The result
is: **linear log-error regret (split by sign) + quadratic effort (split by
direction) + a detection rate, scored per scenario class.** The model's
behavioral predictions — counter-age dependence, and the champion beating
the incumbent on a live hashrate drop — are confirmed on real mining
hardware (§9); the cost-model weights remain a calibrated judgment.

Convention: *Theorem* and *Lemma* are reserved for results genuinely
proved from the model (Theorems 1–2, the §4 Lemma, §6(ii)); everything
else load-bearing is an *Argument*, *Rationale*, or *Choice* — our
reasoning or our decisions — and *Observation* is a fact about particular
algorithms established in simulation (named inline). Mathematical claims
are never "validated numerically": they are proved or they are not. A
future, more formal treatment would restore Theorem framing to the
Arguments and sketch their proofs.

Note on terminology: *regret* here means the time-integrated tracking loss
`∫|e|`, **not** online-learning regret against a comparator; read it as
"tracking loss."

---

## 1. The problem, and the model

Variable difficulty exists because the miners on one pool span many orders
of magnitude in hashrate, while the pool needs every connection to deliver
shares at roughly the same rate. A share is a proof-of-work that clears a
per-connection difficulty `D`; a miner of hashrate `H` clears it and
submits valid shares at a rate proportional to `H/D`. A single global `D`
would bury the pool under shares from its biggest miners and starve its
smallest of shares entirely — and starving a connection of shares means no
timely hashrate estimate for it and a high-variance, lumpy reward.
Per-connection vardiff fixes this by moving each `D` so that connection's
share rate stays near a target `r*`. That makes `r*` the one design
constant doing three jobs at once: it caps the bandwidth and CPU each
connection costs, it fixes the variance of the pool's per-miner hashrate
estimate, and it bounds the miner's reward variance. The controller's whole
job is to hold the realized rate at `r*` as `H` drifts — which is exactly
what the rest of this document scores.

Two modeling choices fall out of the physics; neither is a free knob.
**Share arrivals are Poisson** because each hash is an independent trial
with a tiny success probability against an enormous number of trials per
window, so over any window short enough that `H` is steady, the count is
Poisson with rate equal to hashrate over work-per-share. Under `D = Ĥ/r*`
that rate is `r_obs = r*·H/Ĥ` — equation (1) — with the proportionality
constant folded into the units of `D`, which is why nothing downstream
depends on it. And **the natural coordinate is `e = ln(Ĥ/H)`** because the
controller only ever acts on `D` multiplicatively, the only thing it can
observe depends on `Ĥ` solely through the ratio `H/Ĥ`, and only in log
units is a 2× over-difficulty the same distance from target as a 2×
under-difficulty. Working in `e` turns a retarget into an additive step `s`
and makes the two signs honest, symmetric coordinates instead of artifacts
of where you sit on the difficulty axis. A metric written in raw `D` or raw
share rate would punish the same percentage error differently depending on
absolute difficulty — exactly the bug log coordinates remove.

This is an *evaluation* model, not a control-design model. To rank
algorithms we need only the path each produces — the sequence of `(error,
did-it-fire, step-size)` — and the Poisson-in-log model produces exactly
that: cheaply, repeatably, and identically for an algorithm whose internals
we can read and one we cannot, because the only signal any controller has
is a function of `e` alone (Theorem 1). That is what lets one metric rank
the whole field without privileged knowledge of anyone's implementation.

What the model leaves out — because the metric covers exactly the
situations the model can express. It assumes `H` is constant within a
window (true over one retarget, false across a real ramp, which is why
scenarios drive `H(t)` explicitly rather than leaning on the within-window
assumption); one worker per connection (real connections pool many workers,
raising the effective rate and reshaping the noise); retargets that are
instant and lossless except for the in-flight work the §6 asymmetry already
charges for (real retargets fight network and propagation latency,
producing stale shares the model does not separately represent); and a
continuous difficulty with no floor and no rate limit on how often it can
change (real pools clamp `D` from below and cap fire cadence). The first is
handled by construction; two of the rest — the cadence cap and the
stale-share channel — land on the effort term and are taken up in §7.
Anything beyond this list is a coverage gap to declare, not a hole in the
derivation.

### Setup

A **miner** has true hashrate `H > 0`. The **controller** holds a belief
`Ĥ > 0` and sets difficulty `D = Ĥ/r*`, where `r*` is the target share
rate (shares/min). Shares then arrive as a Poisson process of rate

```
  r_obs = r* · H/Ĥ.                                                  (1)
```

Over a window of `τ` minutes the controller sees a single number: the
share count `N ~ Poisson(λ)`, `λ = r_obs·τ`. It periodically **fires** —
rescales `Ĥ` by a factor, i.e. adds a **log-step** `s = ln(Ĥ⁺/Ĥ⁻)` to its
belief; `s > 0` *tightens*, `s < 0` *eases*.

**Definition 1 (error coordinate).** The controller acts multiplicatively
(it scales `Ĥ`) and, by (1), the observable depends on `Ĥ` only through the
ratio `H/Ĥ`. A multiplicative quantity is linearized by its logarithm, and
only in log coordinates are an over- and an under-shoot by the same
*factor* equidistant from the target. So work in

```
  e = ln(Ĥ/H),     so that     r_obs/r* = e^{−e}.                    (2)
```

`e = 0` is exact; `e > 0` is over-difficulty, `e < 0` under-difficulty. The
control goal is `e → 0`.

A **scenario** specifies `H(t)` and the initial belief; the named classes
(`baseline.rs`) are *cold-start*, *stable*, *step ±p%*, and *settled-aged
drop* (truth holds long enough to mature the share counter, then drops). A
**metric** ranks algorithms by averaging over a fixed ensemble of classes.

---

## 2. The controller measures only its own error

**Theorem 1.** *The sole observable `N` has a distribution depending on `H`
and `Ĥ` only through the error `e`.*

*Proof.* By (1)–(2), `N ~ Poisson(r*τ·e^{−e})`. The parameter is a function
of `e` alone. ∎

There is no measurement of the miner separate from the error being
controlled. Every familiar quality statistic — accuracy, jitter, reaction,
overshoot — is therefore a functional of the one scalar process `e(t)`
together with the fire sequence the controller derives from it. This is
why a small set of `e`-based terms can suffice, and why the metric is
written entirely in `e` and `s`.

---

## 3. Precision costs time, and time costs agility

**Theorem 2 (information floor).** *Any unbiased estimate of `e` from a
window of `τ` minutes has `Var(ê) ≥ 1/(r*τ)`.*

*Proof.* The window's count is `N ~ Poisson(λ)`, `λ = r*τ·e^{−e}`. For a
Poisson observation the log-likelihood in `e` is `N ln λ − λ`, with score
`∂_e(N ln λ − λ) = (N/λ − 1)·λ' = λ − N` since `λ' = −λ`. The Fisher
information is `E[(λ−N)²] = Var(N) = λ`; at the operating point `e≈0`,
`λ = r*τ`. Cramér–Rao gives `Var(ê) ≥ 1/(r*τ)`. ∎

(The crate encodes this as `SettledAccuracy::poisson_floor`,
`metrics.rs:864` — `1/√(r*τ)` as a percentile.)

**Corollary (the central trade-off).** Steady-state precision improves
only by enlarging the averaging window `τ`; but the same `τ` is the lag in
following a real change. **Accuracy and agility are bought from one budget
at a fixed rate `r*`.** Two consequences:

- *The apparent quality axes are one trade-off.* The estimator's window
  trades accuracy against lag; the fire threshold trades false alarms
  against detection delay; the retarget gain trades convergence against
  overshoot. Each is one knob on the same accuracy-vs-agility line.
  Scoring six such projections with equal weight (the deprecated
  `EqualWeightFitness`) rewards the *middle* of the trade-off curve, not
  the *frontier*. **Observation (commit `31a9dbc1`):** four independent
  parameterizations of the pipeline all saturate at the same quality
  ≈0.55 — one wall seen four ways. So score the frontier's own
  coordinates: **tracking error** and **control effort**, nothing derived
  from them.
- *A steady offset has a price, not a defect.* An algorithm sitting short
  of the floor is paying for it elsewhere (agility, effort); §7 uses this.

The floor `1/√(r*τ)` is a **noise band** — one standard deviation of an
*unbiased* estimator — not a systematic offset. The ideal estimator is not
biased downward (its log-of-count Jensen bias here is `~ −1/(2r*τ)`,
negligible); when §8/§10 quote the accuracy ceiling as "≈−0.8%," that is the
*width* of this band at the operating `r*τ`, the scatter a perfect tracker
still shows, against which a real algorithm's *systematic* offset (§10) is
measured.

---

## 4. The error norm: squared is blind, linear is not

Tracking cost is `∫ f(e) dt` for some norm `f`. The choice of `f` is a
judgment about how harm scales with error — but one judgment is
inadmissible.

**Lemma (blindness of the square).** *A persistent, undetected fractional
hashrate drop of size `g` produces a steady error `e = −ln(1−g) = g +
O(g²)`. Under `f(e)=e²` it costs `e² = g² + O(g³)`; under `f(e)=|e|` it
costs `|e| = g + O(g²)`.*

*Proof.* The drop sends `H → (1−g)H` with `Ĥ` fixed, so `e = ln(Ĥ/((1−g)H))
= −ln(1−g)`; expand. ∎

Operational harm from a difficulty error (lost or excess work) scales with
its **magnitude** `g`, i.e. linearly. The squared norm undervalues it by a
factor `1/g`, which diverges as `g → 0`: a small *persistent* leak — a
failing or throttling ASIC, the case operators care about most — is
essentially free under `e²`. **Observation (`regret-effort`):** an
algorithm that detects a −10% drop ~1% of the time scores *better* on `e²`
than one that detects it always, because the miss costs only `(ln0.9)² ≈
0.01`/min. The linear norm removes this blind spot (the same miss costs
`≈0.10`/min) and only reorders the middle of the ranking, never the top.

**Choice 1.** Use `f(e) = |e|`. Report it split by sign, since the two
signs carry different harm (§6):

```
  regret_over  = ⟨|e|⟩ over time with e > 0       (over-difficulty)
  regret_under = ⟨|e|⟩ over time with e < 0       (under-difficulty)
```

---

## 5. Detection is a separate axis, not a smaller `f`

Linear regret narrows the blind spot but does not close it: a fast
algorithm with occasional large errors can still outscore a chronically
blind one on `∫|e|`. The deeper reason is structural.

**Argument (detection is not derivable from the scored error paths).**
*There is no functional `F` with `detection = F(e on stable, e on step)`
holding across all algorithms.*

*Why.* Catching a small drop requires the share counter to be *young* when
the drop lands: a matured counter averages the weak post-drop signal
against a long pre-drop baseline and never crosses threshold. Counter age
at the drop is determined by the fire history of a *matured, on-target*
loop — a state the stable scenario (never perturbed) and the step scenarios
(perturbed while young or with a large signal) never enter. Two controllers
can therefore produce *identical* stable and step error paths yet differ on
detection, because they differ only in that matured-counter regime — so no
`F` on the scored paths can recover it. (A formal version would exhibit
such a pair explicitly.)

So detection must be carried explicitly:

```
  detection = P[ fire within W min | counter matured, then −g drop ].
```

It is an absolute probability — reported raw, never normalized against the
candidate set. **Choice 2.** Operationalize as the `settled 60min, −10%`
cell (`settled_reaction_rate`); **observation (commit `70fcb260`):** this
reproduces the known counter-age failure (one algorithm 30%→1% across
share rates, another ~0%), so it measures the real mode.

---

## 6. The two directions are not symmetric

**Argument.** *Over-difficulty is worse than under-difficulty, and
tightening is worse than easing.*

*Why.* (i) `e < 0` (difficulty low): shares run a little fast; all work
stays valid; cost is mild inefficiency. `e > 0` (difficulty high): the
connection is starved of valid shares, which inflates both the miner's
reward variance and the pool's hashrate-estimate variance for it — risking
an offline misread — and compounds when `H` is genuinely falling. (ii)
[proved] A tightening fire (`s>0`) invalidates in-flight shares aimed at
the old, easier target — fraction `1 − e^{−s} > 0` lost; an easing fire
leaves prior work valid. ∎

This proves the *direction* of the asymmetry. Its *magnitude* is a
judgment. **Choice 3:** weight both at `3:1` (over:under and up:down),
matching the production `tighten_multiplier = 3` (commit `a1d3fa7b`).
**Observation (`champion-weights`):** the best algorithm is the same for
every ratio in `[1:1, 4:1]`; only an ungrounded `5:1` changes it. So the
ranking does not hinge on the exact value.

*The under-difficulty side has its own cost, and the metric only partly
prices it.* Under-difficulty (`e<0`) is not free: at the realized rate
`r = r*·e^{−e}`, an `e=−0.07` offset runs the connection ~7% over its
target share rate, permanently — and bounding exactly that per-connection
load is *why* `r*` is set where it is (§1). The resource cost (extra
bandwidth, CPU, share-accounting) is linear in excess volume, and since
`r − r* ≈ −r*·e` near the operating point, **linear in excess volume is to
first order linear in `|e|`** — so it adds no new functional form, only a
one-sided increase to the `regret_under` coefficient. The current metric
charges this as part of `regret_under = ⟨|e|⟩` at weight 1, but does not
*separately* price the share-volume resource cost beyond that, and the
`3:1` weight was derived for the death-spiral asymmetry alone. The honest
consequence: the cost-optimal offset (§10) is the value under a model that
under-charges the under side, so pricing share volume explicitly can only
*shrink* it. The magnitude is expected to be modest — for pools run with
per-connection headroom (the common case) the resource curve is linear and
far from its convex knee, and a partial offset works the other way (more
volume tightens the hashrate estimate, `Var ∝ 1/N`, the concave benefit the
"under is safe" intuition already banks). The exception is a hard
per-connection quota-with-buffer (flat-then-steep, convex): there the
correction is large, and that is the regime to model for stressed/older
hardware. The clean way to settle it is one measured number — marginal cost
`c` per extra share (CPU-ms / bytes / `$`) from share-accounting telemetry —
added as `c·r*·max(0,−e)` to the objective; this is left as the one
external-economics input the simulation cannot supply.

---

## 7. The metric

Per scenario class, from the trajectory `{(e, fired, s)}` alone — hence
computable for every algorithm, transparent or opaque
(`LogErrorRegret`, `metrics.rs`):

```
  regret_over, regret_under  =  ⟨|e|⟩, split by sign of e       (§4)
  effort_up,  effort_down    =  Σ s², split by sign of s        (below)
  detection                  =  P[fire | matured, small drop]   (§5)
```

The **five-vector is the primary object**; for ranking only, a *rough
summary* scalar is

```
  cost = 3·regret_over + 1·regret_under
       + ρ·(3·effort_up + 1·effort_down)
       + w·(1 − detection),         ρ = w = ½ (overridable).
```

A caveat that keeps the scalar honest: the three families are not
naturally commensurable — `regret` is a time-average, `effort` is a *sum*
over fires, `(1−detection)` is `O(1)` — so at `ρ = w = ½` the raw scalar
is detection-dominated and scales with the window length. To compare scalars
across window choices, express effort as a rate (mean `s²` per fire, or
`Σs²/T`) and rescale the three families to comparable ranges before
weighting. We do not lean on the scalar past coarse ranking; §8 explains
why, and the five-vector is what is reported.

**Effort, and an honest note on its shape.** `effort = Σ s²` penalizes
churning the difficulty. Squared is the right shape *here* — opposite to
§4 — because the two terms answer different questions: regret asks "how bad
is being wrong?" (must not vanish as `e→0`, hence linear), while effort
penalizes overshoot and concentrated actuation — one large retarget costs
more than several small ones summing to the same total (`S² > k(S/k)²`), so
it prefers gentle, distributed correction. It is *not* a model of work lost
to a tighten — that loss is `1−e^{−s} ≈ s`, linear in the step; the
lost-work *asymmetry*, not its size, is what the directional split carries.

*Assumption (fire cadence is capped).* `Σs²` read as "gentleness" has a
blind spot that is the exact dual of §4's: hold `Σs²` fixed, shrink each
step's amplitude, and raise the fire frequency, and the *true* linear lost
work `Σ|s| ≈ Σs/2` diverges while `Σs²→0` — high-frequency, low-amplitude
churn would score as "gentle." Real pools forbid this with a **minimum
inter-fire interval**, which the model adopts as an explicit assumption; it
bounds fire frequency and closes the blind spot. (Absent that cap, `Σs²`
should be read purely as overshoot regularization, not as a charge for
total actuation.)

**Per class, never pooled.** Cold-start cost dwarfs steady-state cost, so a
pooled average erases every distinction that matters; each class is
reported on its own.

This metric meets its obligations: it is computable from the trajectory
for any algorithm; the detection term blocks the blind-but-numerically-fine
algorithm that defeats a pure-regret score; the asymmetry prices the
runaway direction; and it contains no free constants beyond the three
declared `3:1`/`½` weights, which are shown not to swing the ranking.

---

## 8. Three questions, three views — and why not one number or a radar

The metric is a five-number vector per scenario class. The question is not
*how* to draw five numbers — it is *what question each drawing answers* —
because no single picture answers all of them, and the wrong picture hides
the exact failure you most need to see. There are three questions, and they
want three different instruments. All three are built only from the
trajectory `{(e, fired, s)}`, so they apply to every algorithm.

**How much total cost? — the scalar.** You need it to rank, and it is
useless for anything else. A scalar is a time-integral: it gives the total
but throws away *when* the cost happened and *what kind* it was. Two
controllers with the same score can be opposites — one twitchy but
accurate, one calm but blind. Worse, the scalar cannot tell a slow leak
from a transient: a small error that persists forever and a large error
that heals in a minute can integrate to the same number, though the first
quietly drains the operator and the second fixes itself. Hand an engineer
"0.71" and they cannot tell whether the algorithm is slow to converge,
biased at rest, or asleep through a drift. So rank with the scalar; never
debug with it.

**Where does this algorithm spend its budget? — the radar, on a short
leash.** A radar is the natural reach for "show me the trade-off shape,"
and an unconstrained one is actively misleading in two ways engineers walk
straight into. First, its enclosed area depends on the *order* of the axes
— permute the spokes and the same data draws a different polygon with a
different area — so "bigger area is better," which is how everyone reads a
radar, is an artifact of layout, not a real aggregate. Second, best-in-set
normalization rescales every vertex whenever the candidate set changes, so
the same algorithm looks different depending on who else is in the
comparison, and you need a deliberately bad strawman just to make the
spread visible. The regret-radar (`regret-radar`) sidesteps both by
construction: a **fixed reference algorithm** — the classic vardiff you
already run — sits on a mid-ring; every contender reads directly as "beats
it / loses to it" on each axis (`ref/(ref+cost)`); detection is plotted raw
as a probability; and there are no invented axis ceilings (the O4
obligation). The §3 corollary is what licenses a radar at all: its five
axes — **tracking** (`−regret`), **gentleness** (`−effort`), **detection**,
**over-difficulty safety** (`−regret_over`), **tighten-care**
(`−effort_up`) — are not arbitrary but the frontier coordinates the six old
ones collapsed into. Constrained this way it answers one genuinely useful
question — does this algorithm spend its budget on accuracy, gentleness, or
safety? — and a companion panel on the familiar measurements (convergence,
settled accuracy, reaction, jitter, per-axis log-scaled) bridges it to
numbers people already have instincts about. But keep its limits in view:
it is still five numbers, and it still cannot show time. *What it shows
here:* the champion encloses the classic reference on every axis, its area
concentrated in gentleness and tighten-care (it corrects in small,
distributed steps) while merely matching on raw tracking — the §6 asymmetry
made visible.

**When does the cost happen, and what does the failure look like? — the
trajectory.** This is the only view that keeps the dimension the other two
destroy: sequence and duration. One run (`trajectory-plot`) walks through
cold-start, settle, and an aged −10% drop — the three regimes that map
one-to-one onto the three forces in the metric (agility §3; the settle
offset, the §3 price / §6 asymmetry; detection §5), in the order an
operator actually lives through them. The **fire raster** beneath turns
"gentle versus violent" from an adjective into something countable: one
tick mark per fire, height `∝ |s|` — classic's handful of tall marks
against the champion's many short ones. And it is the only picture that can
show the headline at a glance: an algorithm that tracks *tighter* in steady
state and is *blind* to a slow drop. Two reference marks make the
document's central point visual rather than asserted, drawn *separately* so
that minimum error and minimum cost are visibly not the same place:

- the **accuracy ceiling** — a policy-free estimator firing every tick,
  reaching the §3 information floor (≈−0.8% offset) but at unbounded
  effort, so it is an accuracy *bound*, not a target; and
- the **cost-optimal corridor** — where §7's objective actually wants the
  algorithm to sit.

*What it shows — the finding,* where the theory predicts the algorithms
separate:

| regime | classic (real vardiff) | champion |
| --- | --- | --- |
| cold-start ramp | ~11 min | ~15 min |
| settle offset | ≈0% | ≈−7% (cost-optimal, §10) |
| aged −10% drop | **never detects** (in window) | detects ~9 min |

Classic tracks truth tightly in steady state and rejects the champion's
small offset — but it **never reacts** to the slow drop, because a matured
counter has diluted the signal (§5 made flesh). The champion accepts a
small, cost-optimal under-difficulty offset and in exchange catches the
drift that is the operator's real loss.

These are not three views of one thing; they are a hierarchy of questions:
**rank** with the scalar, **characterize** with the constrained radar, and
**trust** with the trajectory — because trust comes from watching the
controller behave across the regimes you care about, not from a number that
has already integrated those regimes away. For a mining engineer the
trajectory is the one to put on the wall: it reads like the dashboards and
incident timelines they already live in.

---

## 9. Validation against real hardware

Everything above is derived from a model and scored in simulation. The
model's *behavioral* predictions were then checked against real miners —
an Antminer S21 (~200 TH/s) on testnet4, driven through the **shape-proxy**
tool (which gates a real share stream to impose a chosen rate profile
without touching firmware), against side-by-side SRI pool instances. This
tests the mechanism, not merely the metric that prefers the champion.

**The classic algorithm's mechanics reproduced quantitatively** (sim
prediction vs. live observation, all matching): steady-state jitter zero
over 30+ min; deterministic −16.7% per fire; exact 300 s fire cadence;
~60% post-staircase overshoot (sim p99 = 69%); ±50% symmetry. Most
important, the **counter-age dependence** the §5 mechanism rests on:
a 5-min counter reacted in 4.4 min, a 51-min counter in 51.8 min — the
matured-counter blindness, seen in hardware.

**The champion's win reproduced live.** Deployed side-by-side with classic,
both mining overnight to mature their counters, then both miners' hashrate
halved at once: **classic took several hours and first moved in the *wrong*
direction; the champion responded in minutes and settled correctly.** This
is the §5 detection claim and the §6 wrong-direction (runaway-risk) claim,
both confirmed outside the simulation.

**What this does and does not anchor.** It externally validates the
*behavioral* layer — reaction times, age-dependence, direction of response,
the relative ranking of classic vs. champion. It does **not** validate the
*economic weights* (the `3:1` asymmetry, the share-volume coefficient `c`):
those are operator-value calibrations the hardware tests do not probe. So
the honest scope is **behaviorally validated on real hardware; the cost-
model weights remain a calibrated judgment** — stronger than internal
consistency (§10) alone, short of a full economic backtest.

**Open hardware tests** (mapping to the residual risks): a *sustained slow
decline* (`−X%/hr` ramp, runnable via shape-proxy's Ramp profile) to earn
the death-spiral safety claim that §6 currently only argues — the one input
that could turn a persistence-based actuator into a slow runaway;
*multi-connection* operation, since the model assumes one worker per
connection while real connections aggregate many; and measuring `c` to
close the §6 share-volume term. Production runs at `r* ≈ 4–6` spm with
headroom (so the linear-small share-volume regime holds, and running
*faster* — higher `r*` — is supported by the model: it tightens both the
detection floor and the estimate at a volume cost the headroom absorbs);
older, stressed machines are the convex-quota exception to model
separately.

---

## 10. One consistency check, and how to break the result

**The offset is optimal, not a defect (`confirm-debias`).** The best
algorithm sits at a steady ≈−7% under-difficulty, short of the ≈−0.8%
floor a policy-free estimator reaches by firing every tick. Multiplying its
belief by `b ≥ 1` closes the offset smoothly (to 0 near `b≈1.10`), but the
cost rises monotonically from `b = 1`: `regret_under` falls while
`regret_over` rises faster under the `3:1` weight. The unbiased belief is
the cost minimum, so the offset is not an error — exactly as §3 predicts.

What the offset *is*, precisely: the algorithm runs inside a noise band of
width `σ_eff` (the §3 floor at its effective window), and under the `3:1`
asymmetry the cost-minimizing place to center is not the band's mean but a
**quantile below it** — roughly `−0.67·σ_eff` (the point where the
3×-weighted over-difficulty tail balances the 1×-weighted under tail). The
`3:1` weight fixes that coefficient and its sign; `σ_eff` is set by the
effort/agility choice (the short estimator window). `confirm-debias`
verifies the *quantile condition* (that `b=1` minimizes cost), not the
*width* of the band. So an independent knob that *could* push accuracy
toward the floor is correctly scored *worse* — the metric is
self-consistent. (The offset carries an unpriced share-volume cost noted in
the findings; it does not change the quantile result.)

**Falsifiers.** The result should be revised if: (a) some `∫f(e)` on the
scored scenarios reproduces the detection ranking, refuting the §5
Argument; (b) an *unbiased* estimator beats `1/(r*τ)`, refuting Theorem 2
(biased estimators routinely beat the CRB on variance, so the qualifier is
essential); (c) the best algorithm changes within `w_over:w_under ∈
[1:1,4:1]`, breaking §6's robustness; or (d) a real failure mode falls
outside the scored ensemble — a coverage gap to declare, not a soundness
error.

---

## 11. Status of each claim

| Claim | Kind | Source |
| --- | --- | --- |
| Observable depends only on `e` | Theorem 1 | §2 |
| Precision floor `1/(r*τ)` | Theorem 2 | §3 |
| Quality axes are one trade-off | Corollary + obs. | §3, `31a9dbc1` |
| Squared norm blind to small drops | Lemma | §4 |
| …a 1%-detector scores well on `e²` | Observation | `regret-effort` |
| Detection not derivable from `e(t)` | Argument | §5 |
| …real algorithms go blind | Observation | `70fcb260` |
| Over>under, tighten>ease | Argument (ii proved) | §6 |
| `3:1` weight; robust over `[1:1,4:1]` | Choice + obs. | `a1d3fa7b`, `champion-weights` |
| −7% offset is cost-optimal | Observation | `confirm-debias` |
| Share-volume cost is ~linear in `\|e\|`, one-sided | Argument | §6 |
| Counter-age mechanism on real hardware | Observation (HW) | §9, PR #2154 |
| Champion beats classic on a live drop | Observation (HW) | §9, PR #2154 |

Every clause of the metric — linear sign-split regret, quadratic
direction-split effort, an explicit detection rate, per class, `3:1`
weights — is proved from §§2–3, forced by §§4–5, or a judgment shown
robust in §6. The behavioral predictions are externally confirmed on real
hardware (§9); the cost-model weights remain a calibrated judgment, robust
over the grounded range and open to an economic backtest.
