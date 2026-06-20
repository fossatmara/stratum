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
direction) + a detection rate, scored per scenario class.**

Convention: *theorems and lemmas* are proved from the model; *definitions
and choices* are our decisions; *observations* are facts about particular
algorithms established in simulation (named inline). Mathematical claims
are never "validated numerically" — they are proved or they are not.

---

## 1. Setup

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

**Theorem 3 (detection is not a functional of the scored error paths).**
*The probability of catching a small drop on the settled-aged scenario is
independent of the regret terms computed on the stable and step
scenarios.*

*Proof.* Catching a small drop requires the share counter to be *young*
when the drop lands: a matured counter averages the weak post-drop signal
against a long pre-drop baseline and never crosses threshold. Counter age
at the drop is determined by the fire history of a *matured, on-target*
loop — a state the stable scenario (never perturbed) and the step
scenarios (perturbed while young or with a large signal) never enter. A
functional of `e(t)` on scenarios that avoid that state cannot constrain
behavior in it. ∎

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

**Proposition.** *Over-difficulty is worse than under-difficulty, and
tightening is worse than easing.*

*Proof.* (i) `e < 0` (difficulty low): shares run a little fast; all work
stays valid; cost is mild inefficiency. `e > 0` (difficulty high): the
miner is starved of valid work, and if its hashrate is genuinely falling,
raising difficulty further removes its remaining valid work — a runaway.
(ii) A tightening fire (`s>0`) invalidates in-flight shares aimed at the
old, easier target — fraction `1 − e^{−s} > 0` lost; an easing fire leaves
prior work valid. ∎

This proves the *direction* of the asymmetry. Its *magnitude* is a
judgment. **Choice 3:** weight both at `3:1` (over:under and up:down),
matching the production `tighten_multiplier = 3` (commit `a1d3fa7b`).
**Observation (`champion-weights`):** the best algorithm is the same for
every ratio in `[1:1, 4:1]`; only an ungrounded `5:1` changes it. So the
ranking does not hinge on the exact value.

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

scalarized, when needed, as

```
  cost = 3·regret_over + 1·regret_under
       + ρ·(3·effort_up + 1·effort_down)
       + w·(1 − detection),         ρ = w = ½ (overridable).
```

**Effort, and an honest note on its shape.** `effort = Σ s²` penalizes
churning the difficulty. Squared is the right shape *here* — opposite to
§4 — because the two terms answer different questions: regret asks "how
bad is being wrong?" (must not vanish as `e→0`, hence linear), while effort
asks "how should we charge for actuation?" A square makes one large
retarget cost more than several small ones summing to the same total
(`S² > k(S/k)²`), i.e. it prefers gentle, distributed correction. It is
*not* a model of work lost to a tighten — that loss is `1−e^{−s} ≈ s`,
linear in the step; the lost-work *asymmetry*, not its size, is what the
directional split carries.

**Per class, never pooled.** Cold-start cost dwarfs steady-state cost, so a
pooled average erases every distinction that matters; each class is
reported on its own.

This metric meets its obligations: it is computable from the trajectory
for any algorithm; the detection term blocks the blind-but-numerically-fine
algorithm that defeats a pure-regret score; the asymmetry prices the
runaway direction; and it contains no free constants beyond the three
declared `3:1`/`½` weights, which are shown not to swing the ranking.

---

## 8. Reading the metric: two plots

The metric is a five-vector per scenario class. A table of five numbers
times five classes times a dozen algorithms is unreadable, and it hides
the one thing the scalar `cost` integrates away: *behavior over time*. Two
plots recover both — one for the trade-off **shape**, one for the
time-domain **story**. Both are built only from the trajectory `{(e,
fired, s)}`, so they apply to every algorithm.

### 8.1 The trade-off radar (`regret-radar`)

Five axes, each a real cost from §7, each oriented so outward = better:
**tracking** (`−regret`), **gentleness** (`−effort`), **detection**,
**over-difficulty safety** (`−regret_over`), **tighten-care**
(`−effort_up`). The §3 corollary is what licenses a radar at all: these
are not arbitrary axes but the frontier coordinates the six old ones
collapsed into.

*Why it reads cleanly.* Each axis is scored `ref/(ref+cost)` against a
**fixed reference algorithm** — the real upstream classic vardiff — so the
reference sits on a mid-ring and every contender reads directly as "beats
/ loses to it" on each axis. This is deliberately not hull (best-in-set)
normalization, which would rescale every vertex whenever the candidate set
changes and would need a deliberately-bad strawman to show any spread. The
reference is fixed, the picture is stable run-to-run, and there are no
arbitrary axis ceilings (the O4 obligation). Detection is plotted raw, as
a probability, since normalizing it would force the best-in-set to 1.

*Companion panel.* Beside it, the same algorithms on the **familiar**
measurements operators already trust — convergence time, settled accuracy,
reaction, jitter — on a per-axis log scale (each axis spans the observed
range), purely as an intuition bridge from the abstract regret/effort axes
to quantities people have priors about.

*What it shows.* The champion encloses the classic reference on every
axis, with its area concentrated in gentleness and tighten-care — it
corrects in small, distributed steps — while merely matching on raw
tracking. That shape *is* the §6 asymmetry made visible: it spends its
budget avoiding the costly direction, not chasing the last percent of
accuracy.

### 8.2 The trajectory plot (`trajectory-plot`)

A single run on one timeline that visits three regimes in sequence:
**cold-start** (belief far from truth), **settle** (counter matures), then
a **−10% aged drop**. Plotted: each algorithm's implied hashrate `Ĥ(t)`
(median over trials), the true `H(t)` (dashed), an **accuracy-ceiling**
line, a **cost-optimal corridor**, and a **fire raster** beneath.

*Why this is the honest picture.* The scalar metric is a time-integral; it
cannot show *when* error or effort occurs. But the three regimes map
one-to-one onto the theory: cold-start speed is **agility** (§3), the
settle offset is the **§3 price / §6 asymmetry**, and the drop response is
**detection** (§5). One plot exercises all three forces the metric is
built from, in the order an operator would experience them.

*Two reference marks, and why they are not the same thing.*
- The **accuracy-ceiling** is a policy-free estimator that fires every
  tick — it reaches the §3 information floor (≈−0.8% offset) but at
  unbounded effort. It is labeled an accuracy *bound*, not a target:
  reaching it is possible only by paying effort the metric charges for.
- The **cost-optimal corridor** marks where §7's objective actually wants
  the algorithm to sit. Drawing both, separated, is the visual statement
  that *minimum error ≠ minimum cost* — the central trade-off of the whole
  document.

*The fire raster* draws one tick mark per fire, height `∝ |s|`. It makes
"violent vs gentle" literal: classic fires rarely but in large jumps (few
tall marks); the champion fires often in small steps (many short marks).
A scalar `effort` says the champion is gentler; the raster shows *how*.

*What it shows — the finding.* The picture separates the algorithms
exactly where the theory says it should:

| regime | classic (real vardiff) | champion |
| --- | --- | --- |
| cold-start ramp | ~11 min | ~15 min |
| settle offset | ≈0% | ≈−7% (cost-optimal, §9) |
| aged −10% drop | **never detects** (in window) | detects ~9 min |

Classic tracks truth tightly in steady state and rejects the champion's
small offset — but it **never reacts** to the slow drop, because a
matured counter has diluted the signal (Theorem 3 made flesh). The
champion accepts a small, deliberate, cost-optimal under-difficulty offset
and in exchange catches the drift that is the operator's real loss. No
single scalar conveys "tracks tighter yet is operationally blind"; the
trajectory plot does, at a glance.

---

## 9. One consistency check, and how to break the result

**The offset is optimal, not a defect (`confirm-debias`).** The best
algorithm sits at a steady ≈−7% under-difficulty, short of the ≈−0.8%
floor a policy-free estimator reaches by firing every tick. Multiplying its
belief by `b ≥ 1` closes the offset smoothly (to 0 near `b≈1.10`), but the
cost rises monotonically from `b = 1`: `regret_under` falls while
`regret_over` rises faster under the `3:1` weight. The unbiased belief is
the cost minimum, so the offset is the deliberate price of the asymmetry
(§6), not an error — exactly as §3 predicts. An independent knob that
*could* push accuracy toward the floor is correctly scored *worse*, which
is the metric being self-consistent.

**Falsifiers.** The result should be revised if: (a) some `∫f(e)` on the
scored scenarios reproduces the detection ranking, refuting Theorem 3;
(b) an estimator beats `1/(r*τ)`, refuting Theorem 2; (c) the best
algorithm changes within `w_over:w_under ∈ [1:1,4:1]`, breaking §6's
robustness; or (d) a real failure mode falls outside the scored ensemble —
a coverage gap to declare, not a soundness error.

---

## 10. Status of each claim

| Claim | Kind | Source |
| --- | --- | --- |
| Observable depends only on `e` | Theorem 1 | §2 |
| Precision floor `1/(r*τ)` | Theorem 2 | §3 |
| Quality axes are one trade-off | Corollary + obs. | §3, `31a9dbc1` |
| Squared norm blind to small drops | Lemma | §4 |
| …a 1%-detector scores well on `e²` | Observation | `regret-effort` |
| Detection not a functional of `e(t)` | Theorem 3 | §5 |
| …real algorithms go blind | Observation | `70fcb260` |
| Over>under, tighten>ease | Proposition | §6 |
| `3:1` weight; robust over `[1:1,4:1]` | Choice + obs. | `a1d3fa7b`, `champion-weights` |
| −7% offset is cost-optimal | Observation | `confirm-debias` |

Every clause of the metric — linear sign-split regret, quadratic
direction-split effort, an explicit detection rate, per class, `3:1`
weights — is proved from §§2–3, forced by §§4–5, or a judgment shown
robust in §6.
