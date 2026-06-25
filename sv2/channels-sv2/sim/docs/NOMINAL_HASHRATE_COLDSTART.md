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

2. **Path B (runtime fusion of the downward step) is the whole game.** It is
   where the only payoff the open-target path leaves on the table lives. It
   requires a trait/channel API change and rests on a provenance guarantee the
   protocol does not currently provide, so it must be argued on that payoff —
   with the asymmetry (eager-ease only), provenance, and runtime-guard
   requirements spelled out.

The honest one-line framing: **the channel already does the safe thing with the
nominal (sets the operating point, not the belief); the only thing left worth
doing is easing the operating point on a corroborated downward revision, and
that needs a new interface.**

---

## What's measured / unmeasured

**Measured (Path A, falsified):** `seed-rampup.rs`, 400 trials/cell — cold vs
seeded per-tick `e` over 65 min at spm {6,12,30} × decl {1.0,2.0,0.5}×. Result
above: redundant at 1.0×, −31…−39% at 2.0×, −6…−10% at 0.5×. Retires the seed.

**Unmeasured (Path B, pre-registered before any numbers):**
  - **Corroboration lag / detection-gap saving:** on a downward step with a
    simultaneous `UpdateChannel`, the over-difficulty regret saved by easing the
    operating point on the corroborated hint vs. shares-only. This is the number
    that would justify the API change.
  - **Upward-revision inertness:** confirm an upward hint changes nothing
    measurable (it can only corroborate at the current `D`; if shares confirm,
    shares already own the tighten). If the ablation shows it is pure dead weight,
    the fuse is downward-only by construction — a smaller, safer API surface.
