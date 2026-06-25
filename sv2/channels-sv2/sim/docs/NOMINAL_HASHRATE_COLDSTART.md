# Using `nominal_hash_rate`: cold-start seed vs. runtime fusion

Written 2026-06-24. A case, for the SRI/channels maintainers, that the
cold-start seed should land now as an internal vardiff change, and that the
larger runtime-fusion design is a *separate, later* decision that needs an API
change — with the trade-offs of each stated honestly so the second one can be
argued on its merits rather than smuggled in with the first.

The miner declares `nominal_hash_rate` in `OpenStandardMiningChannel` /
`OpenExtendedMiningChannel`, and may revise it mid-run via `UpdateChannel`.
Today the channel uses it to set the *opening* difficulty (`D = nominal / r*`)
and then discards it for vardiff's purposes: the EWMA estimator starts cold
(`rate = 0`, `n_ticks = 0`) and climbs to the truth from the floor over the
~65-min cold-start ramp (THEORY §8.5). There are two distinct things we could
do with the field, and they are NOT the same change.

---

## Path A — cold-start seed (internal to vardiff, ZERO API change)

**What it does.** At channel open, seed the EWMA's rate to `r*` shares/min, so
the very first `snapshot` believes ≈ the declared nominal instead of climbing
from zero. The cold-start ramp collapses from ~65 min to ~0.

**Why it costs no API change — the key structural fact.** The seed value is the
pool's own `shares_per_minute` (`r*`), **not the nominal**. The opening
difficulty was *already* derived from the declaration (`D = nominal / r*`), so
seeding the rate to `r*` reconstructs a belief of
`hash_rate_from_target(open_target, r*) ≈ nominal` *without the vardiff layer
ever reading the nominal*. The seed therefore **inherits the open target's
existing trust decision** — the same decision the channel already makes when it
sets the opening difficulty — and adds no new trust surface.

Concretely, both inputs are already in scope at the construction site
(`pool/.../mining_message_handler.rs`, the open-channel handlers):
  - `nominal_hash_rate` — read from the open message (used as the plausibility
    gate, below);
  - `shares_per_minute` — the pool's config `r*` (already passed to the channel
    constructor).

The change is purely additive:
  - `EwmaEstimator::new_seeded(tau, seed_spm, prior_ticks)` (vardiff-internal);
  - `champion_composed_seeded(...)` / `VardiffState::new_seeded(spm, prior_ticks)`
    (vardiff-internal constructors);
  - the open handler calls `build_vardiff(nominal, spm)` instead of
    `VardiffState::new()` — two call sites, values already in hand.

No change to `ExtendedChannel` / `StandardChannel`, no new accessor, no message
routing, no `Vardiff` trait method. Existing callers are untouched.

**Safety.** The seed is a TIGHTEN-from-the-floor (cold belief is *below* truth;
seeding raises it), i.e. it moves in the §6 *costly* (over-difficulty)
direction. It is bounded, not blind trust, by three independent facts:
  1. **`max_target` clamp.** The channel already clamps difficulty by the
     miner's own declared `max_target`. An inflated declaration cannot push
     difficulty past the floor the miner itself accepted.
  2. **Small `prior_ticks`.** The seed is given ~one tick of EWMA weight, so the
     first window of real shares overwrites it within ~τ. It is a prior, not a
     commitment.
  3. **Plausibility gate.** `build_vardiff` seeds only when the declaration is
     finite and ≥ `MIN_SEEDABLE_NOMINAL_HASHRATE`. A missing/sentinel
     declaration (we have observed deployments declaring `nominal = 1` as a
     placeholder) falls back to a cold start, which is always safe.

**What it does NOT do.** It only touches the *opening* belief. After τ it is
indistinguishable from a cold start — same steady state, same dynamics. A
seeded and an unseeded channel converge identically. It buys the ramp and
nothing else.

**Status.** Coded and compiling end-to-end (channels_sv2 builds; pool
type-checks against it via a local patch). The ramp-collapse *magnitude* is a
simulation measurement still to be run — see "What's unmeasured", below. Land
this behind that measurement, not ahead of it.

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

**What it buys that Path A cannot:** the downward-step detection gap. That is a
real, recurring cost (every genuine hashrate decline pays ~τ of over-difficulty
lag before the share stream catches up), not a one-time startup cost. If the
maintainers want that, it justifies the API change. If they don't, Path A still
captures the startup win for free.

---

## The recommendation to maintainers

1. **Take Path A now.** It is additive, internal, inherits an existing trust
   decision, and is gated against the garbage declarations we have actually
   observed in the field. The only open item is *measuring* the ramp-collapse
   benefit, not any safety question.

2. **Treat Path B as a separate proposal.** It is where the recurring payoff is
   (decline detection), but it requires a trait/channel API change and rests on
   a provenance guarantee the protocol does not currently provide. Argue it on
   that payoff, with the asymmetry + provenance + runtime-guard requirements
   spelled out — do not let it ride in on Path A's coattails, and do not let
   Path A's "we already touch this field" be mistaken for Path B being free.

The honest one-line framing: **Path A is a safe re-use of a trust decision the
channel already makes; Path B is a new trust decision that needs a new
interface to make safely.**

---

## What's unmeasured (pre-registered, before pulling any sim numbers)

These are simulation measurements, not yet run. Recording them here so the
results can't be retrofitted to the conclusion:
  - **Ramp-collapse magnitude (Path A payoff):** mean/median `|e|` over the
    first 65 min, seeded vs. cold, across the r* band. Predicted: seeded ≈ flat
    near 0; cold ≈ the §8.5 decay. The *size* of the integrated-regret saving is
    the number that justifies shipping A.
  - **Seed robustness to a wrong declaration:** seed at 2× and 0.5× truth;
    confirm convergence to the same steady state within ~τ (the `prior_ticks`
    claim) and that the `max_target` clamp bounds the over-difficulty excursion.
  - **Corroboration lag (Path B):** on a downward step with a simultaneous
    `UpdateChannel`, the detection-gap saving vs. shares-only — the number that
    would justify the API change.
