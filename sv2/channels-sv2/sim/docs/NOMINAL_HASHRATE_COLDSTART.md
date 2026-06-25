# Using `nominal_hash_rate`: cold-start seed (RETRACTED) vs. runtime fusion

Written 2026-06-24, retraction added same day after measurement.

> **RETRACTION — Path A (cold-start seed) is withdrawn. Measurement falsified
> its premise.** I led with the seed as the headline "nearly-free win." That was
> wrong, and the error is instructive enough to keep: I took THEORY §8.5's
> 65-min ramp as a *production* phenomenon without checking how the channel
> actually sets the opening difficulty. It doesn't ramp — it opens at the
> nominal. See "Why Path A is wrong" below. The seed was coded, deployed to
> slots 3/4, then rolled back (pool commit `e23df760` reverted by `46d8ec68`).
> The vardiff-internal constructors (`new_seeded` etc.) remain in the library
> as additive dead code — harmless, unused, no caller. **The whole design now
> rests on Path B (runtime fusion of the downward step), which the open-target
> path does NOT already solve.**

The miner declares `nominal_hash_rate` in `OpenStandardMiningChannel` /
`OpenExtendedMiningChannel`, and may revise it mid-run via `UpdateChannel`. The
channel uses it to set the *opening* difficulty and — crucially — this is
already the separate-state principle in production: **the nominal informs the
operating point, never the belief.**

---

## Why Path A is wrong (the falsification)

**The open target already encodes the nominal.** Both channel constructors —
`server/standard.rs:199` and `server/extended.rs:214`, verified identical —
set the opening target via `hash_rate_to_target(nominal_hashrate,
expected_share_per_minute)`, i.e. `D_open = nominal / r*`. So at open the
channel sets the *operating point* from the nominal while leaving the EWMA
*belief* cold (the `EwmaEstimator` `n_ticks==0` fast-path,
`composed/estimator.rs:386`, snaps belief to the first observed count in one
tick). **The nominal→operating-point path is the separate-state design already
implemented.** Path A's error was not "trusting the hint" — it was injecting the
nominal into *belief* on top of the operating point, carving an exception to
"belief stays share-only." The sim closed that exception.

**The mechanism of harm is exactly that violation.** The controller's error
signal is `e = ln(operating_point / belief)`. Seeding `h_estimate` from the
nominal makes belief *agree* with the open operating point, which **zeroes `e`**.
When the open nominal is wrong (the 2.0× over-declared arm), the seeded arm
reads `e ≈ 0` and sits at over-difficulty, while the cold arm's fast-path reads
the real low share rate and corrects *down* immediately. The seed does not so
much *add* over-difficulty as **suppress the correction** of an already-wrong
open. The sign falls straight out of the mechanism — it is not a number taken on
faith.

**The measurement** (`sim/src/bin/seed-rampup.rs`, champion cold vs seeded,
per-tick `e` over the first 65 min, 400 trials/cell):
  - **decl = 1.0× (accurate open):** cold shows `e = 0.0` at *every* head tick.
    Nothing to ramp — the operating point opens at truth. Seed saves +21–36% of a
    tiny base (0.5–4.5 e-min) — only nudging the known steady under-difficulty
    offset, not a ramp.
  - **decl = 2.0× (over-declared):** seed is **−31 to −39%** (worse), in the
    dangerous over-difficulty direction — the suppression above.
  - **decl = 0.5× (under-declared):** seed is −6 to −10% (worse).

**The §8.5 ramp is not a pure artifact — but the seed can't rescue it.** A
5-order open gap (sim `ColdStart`: 1e10 vs 1e15) is what a *placeholder open
nominal* produces — e.g. the SRI translator demo opens with
`nominal_hash_rate = 5_000_000.0` (~5 MH/s, six orders below a real ASIC). So
the ramp is real *where opens are placeholders*. But there the seed **is** the
placeholder, and it additionally suppresses the fast-path correction. Honest
retraction, sharpened: the ramp is real for placeholder opens, the seed can't
fix it there, and it is redundant-or-negative everywhere else.

**Scope note (reference-impl generality):** slots 3/4 are *extended* channels,
so this is settled for the deployed case. The standard-channel open path
(`standard.rs:199`) was confirmed identical, so the generality claim holds for
both channel types — neither ramps on an accurate open.

---

## Path B — runtime fusion (requires an API change)

**What it does.** Treat `nominal_hash_rate` — including mid-run `UpdateChannel`
revisions — as a *second observation channel* feeding the controller
continuously: a fast, unverified hint alongside the slow, honest share stream.
The payoff is **closing the detection gap on a genuine downward step**: when a
miner's real hashrate drops, the share stream takes ~τ to notice, but an
`UpdateChannel` carrying the new (lower) nominal is available immediately.

**Why it needs an API change.** The `Vardiff` trait today receives `hashrate`
as a *per-call argument* to `try_vardiff`; it holds no separate notion of "what
the miner claims" vs. "what I believe from shares." Fusion requires the
controller to carry **two pieces of state** — `controller_belief` (from shares)
and `miner_hint` (from declarations) — and a derived operating point. That
means either:
  - new trait surface (`observe_hint` / `update_nominal`) so the channel can
    push declaration updates into vardiff as they arrive, **and/or**
  - a channel accessor (`get_nominal_hashrate`) plus a vardiff state split so
    the two channels don't collapse into one register.

This is exactly the boundary the cold-start seed avoids. Fusion *cannot* be
done as a per-call arg, because the hint arrives on a different clock than the
`try_vardiff` cadence and must persist between calls.

**Why it is genuinely harder than "just read the field":**
  - **Asymmetry must be enforced per-message, not per-window.** An upward hint
    ("I'm now faster") moves difficulty in the dangerous over-difficulty
    direction on the miner's unverified say-so; it must be near-inert (corroborate
    with shares before acting). A downward hint is safe (eases) and can act
    faster. This is the §6 eager-ease/reluctant-tighten rule applied to the hint
    channel — but now at the granularity of individual `UpdateChannel` messages.
  - **Provenance decides coherence.** Fusion is only sound if the nominal is
    *device telemetry* (an independent measurement) — not a re-derivation of the
    share rate (circular) or a static config echo (stale). We have direct
    evidence this varies per deployment: of four observed slots, two carried
    telemetry-shaped nominals and one carried the `nominal = 1` sentinel. There
    is no protocol field that asserts provenance; `aggregated_device_count` is a
    weak discriminator at best. A fusion that trusts a config-echo as telemetry
    is worse than no fusion.
  - **Runtime plausibility guard.** The same garbage declarations that the
    cold-start gate rejects once at open must be rejected *continuously* for
    fusion, including reconnect/re-open collisions.

**What it buys that the open-target path does NOT already solve:** the
downward-step detection gap. The open target is set *once* at open and moves
thereafter only via share-driven `SetTarget`. So a mid-run hashrate *drop* is
not reflected until the share statistics reveal it — ~τ of over-difficulty lag,
every genuine decline. An `UpdateChannel` carrying the lower nominal is
available immediately. This is the recurring cost the seed could never touch
(the seed only acted at open), and it is the entire reason Path B exists.

---

## The recommendation to maintainers (post-retraction)

1. **Do not ship the cold-start seed.** Measurement showed the open target
   already encodes the nominal (`D_open = nominal/r*`), so seeding *belief* is
   redundant on accurate opens, cannot fix placeholder opens, and is
   net-negative on over-declared opens (it suppresses the cold fast-path's
   correction). The vardiff-internal `new_seeded` constructors are left as
   additive, uncalled dead code; the pool call-site change was reverted.

2. **Path B (runtime fusion of the downward step) is the whole game — and it
   does NOT need a trait change.** It is where the only payoff the open-target
   path leaves on the table lives, and the downward-hint measurement (below)
   established it as a pool-loop write to the operating-point register the
   fire-path already owns — no `Vardiff` trait change, hard-set (α=1) the ship
   form. What it *does* need is a real per-device `UpdateChannel` data source
   (provenance) and the plausibility guard, both spelled out below. The trait/
   channel API change is only required for the *upward* leg (recovery), which we
   declined on safety grounds — so the API argument is scoped to a payoff we are
   not pursuing.

The honest one-line framing: **the channel already does the safe thing with the
nominal (sets the operating point, not the belief); the only thing left worth
doing is easing the operating point on a plausible downward revision — and that
is a pool-loop write, not a new interface.**

---

## Path B measured: the downward-hint ceiling, and the damped-blend null

**The hint works (perfect-telemetry ceiling).** `downward-hint.rs`, champion
Ewma360/s1.5. On a mid-run drop the pool eases the operating-point register
(Ĥ ← declared) before `try_vardiff` — a pool-loop write to the register the
fire-path already writes, NO `Vardiff` trait change. Decline leg: eliminates
60–100% of the over-difficulty (starvation) area, largest where the share-only
controller is slowest (sparse rates). Recovery leg: share-driven in all arms and
statistically identical — the hint is one-sided, helps the decline, neutral on
recovery. Steady state: converges across arms — transient-only, champion
selection / τ-valley / lever / band do not reopen. Q1 (b-vs-c): the estimator
rescale is a strict precision give-back (discards the EWMA's banked smoothing),
so the zero-trait-change variant (no rescale) is as good or better — no trait
hook needed.

**The damped-blend hypothesis was pre-registered and REFUTED.** Proposed
mechanism (an agent's): hard-set (Ĥ ← declared) over-commits to a possibly-wrong
declared value, so under telemetry noise a damped blend
(Ĥ ← (1−α)·Ĥ + α·declared, α<1) should beat hard-set by averaging out the
misread. Pre-registered as a crossover shape (P1), a noise-vs-lag distinction
(P2), a gate⟷α substitution (P3), and an spm scaling (P4), each with a stated
failure condition, locked on the realistic range σ∈[0,0.30] before any numbers.
Result (`VARDIFF_DH_SWEEP=1`, 400 trials/cell):

- **P1 — FAILED (null).** No interior α CI-separates from α=1 at any σ in the
  locked range; α=1 (hard-set) is strictly best everywhere, monotone in α.
- **P2 — REVERSAL (not merely a null).** Damping monotonically *worsens* the lag
  axis (lower α → higher per-minute bias-rate at every lag/spm). This *disproves*
  the over-commitment mechanism directly, rather than failing to support it.
- **P3 — FAILED (null).** Gate and α are independent, not substitutes. The null
  is meaningful *because* of the floor: observed fire-rate 1.00 against the
  pre-registered 0.80 exclusion floor — zero cells excluded, so this is a real
  null, not the fire-collapse case the floor exists to screen out.
- **P4 — vacuous.** P1(a) never separates in-range, so there is no onset to shift.

**Why hard-set wins — the cost asymmetry (this explains the null, not just
reports it).** The two errors are not symmetric in *consequence* before any
magnitude is measured: under-easing leaves you in the *self-reinforcing*
over-difficulty direction (the blinding, starvation one the whole controller
exists to escape), while over-committing to a noisy-low value puts you in the
*self-correcting* under-difficulty direction. So the signs alone say under-easing
is the worse error — and at realistic noise (σ≤0.30) the magnitudes confirm it:
the over-difficulty area is large and a 30%-wrong eased value simply doesn't hurt
as much as easing only partway does. Damping fails for the *same reason* the
design eases fast and tightens slow — it is the §6 eager-ease/reluctant-tighten
asymmetry again, not a separate empirical fact.

**Where the crossover actually sits (post-hoc envelope, descriptive — NOT a
pre-registered test).** Extending σ past the locked range (0.45, 0.60; added
after the in-range nulls, labeled, changing no verdict): the α=1 advantage
*narrows* with σ, and the α=1 / α=0.75 CIs first overlap at **σ≈0.60**. So a
crossover region exists — but at σ≈0.60 the declared value lands below 0.5×
truth ~20% of the time, which is precisely the plausibility gate's reject
condition. **The damping knob and the plausibility gate address the same failure
from opposite ends, and the gate wins: by the time noise is large enough to
justify damping, it is large enough to reject the hint.** That is the structural
reason α is not a useful knob — not "it didn't help in our range" but "the regime
where it could help is the regime the guard already excludes."

**Scope of the recorded claim (and what is NOT claimed):**
1. Hard-set (α=1) is the ship recommendation in the realistic σ≤0.30 range,
   dominant on both cost axes (over-diff area, under-diff wobble) across all
   tested gates and spm.
2. The α=1 advantage *narrows* with σ; α=1/α=0.75 CIs first overlap at σ≈0.60 —
   a crossover region outside trustable telemetry noise and inside the gate's
   reject band.
3. The universal "damping never helps" claim is explicitly NOT made — the trend
   is toward convergence past σ≈0.60, just past where the hint is usable. A
   finite grid cannot establish the universal claim and we do not reach for it.

**Status (sim-only; what closes the open task is specified, not gestured at):**
The decline trigger is a perfect-telemetry-to-σ0.60 envelope in sim. Hardware
validation is pending native-sv2 `UpdateChannel` traffic carrying a *real
mid-run drop* **with per-device channel visibility at the pool** — NOT via the
translator/shape-proxy aggregate. Native-sv2 miners pointing in is necessary but
not sufficient: if they arrive *through* the proxy as part of the aggregate, the
pool still sees one channel and the per-device drop is blurred (the same
per-worker-carriage gap as the 0x0002 discussion). So the validation is not
unblocked the moment a native miner connects — it needs per-device carriage. The
open-time plausibility guard is a *more distant* horizon: sim-demonstrable but
unfalsifiable on the current topology (one aggregate channel, no per-device open
declarations). The two halves do not share a validation status.
