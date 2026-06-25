//! Downward-hint fusion measurement (Path B, the only surviving payoff).
//!
//! THE HUNCH (pre-registered, BEFORE reading any numbers): on a mid-run
//! hashrate DROP, an `UpdateChannel` carrying the lower nominal arrives ~at the
//! drop, long before the slowing share stream reveals it (~τ + boundary lag).
//! If the pool eases the OPERATING POINT (Ĥ, = the channel's nominal_hashrate
//! register, the `hashrate` arg to try_vardiff) toward the declared value at the
//! drop, the over-difficulty excursion e=ln(Ĥ/H)>0 should collapse in time
//! WITHOUT retuning the share-driven controller. This is a pool-loop write to
//! the SAME register the fire-path already writes (update_channel) — NOT a
//! belief injection (that was the retracted seed) and NOT a trait change.
//!
//! THE SUBTLETY THIS BINARY EXISTS TO SETTLE: a normal controller fire calls
//! `estimator.on_fire(new,old)`, which RESCALES the EWMA rate by the target
//! ratio so h_estimate stays consistent across the difficulty change
//! (estimator.rs:360). A pool-external operating-point write does NOT call
//! on_fire. So after a pool ease the estimator's smoothed `rate` is left
//! un-rescaled and inconsistent with the new easier target for ~τ. Predicted
//! consequence: h_estimate transiently UNDER-reads → controller thinks it's
//! still over-difficulty → OVER-eases slightly → a small UNDER-difficulty
//! (e<0, SAFE-side) wobble that self-corrects. The question is whether that
//! wobble is benign (→ downward-only truly needs zero trait change) or whether
//! the ease must also resync the estimator (→ a trait touch).
//!
//! THREE ARMS (champion Ewma360/s1.5 throughout; identical seeds per trial):
//!   (a) shares-only     — baseline. No hint. The status quo decline reaction.
//!   (b) ease-no-rescale — pool writes Ĥ←declared at the drop; estimator NOT
//!                         resynced. The zero-trait-change candidate.
//!   (c) ease-rescale    — pool writes Ĥ←declared AND calls estimator.on_fire
//!                         to rescale (the "do it properly" variant; would need
//!                         a trait hook in production).
//!
//! PRE-REGISTERED PREDICTIONS (so the result can't be retrofitted):
//!   - max over-difficulty (peak e>0) and over-difficulty area: a ≫ b ≈ c.
//!     The hint kills the over-difficulty excursion in both b and c.
//!   - under-difficulty wobble (min e<0 after the ease): b shows a SAFE-side
//!     dip that c does not (or c's is smaller). If b's dip is small (say |e|<5%)
//!     and self-corrects within ~τ, downward-only is clean → zero trait change.
//!     If b's dip is large or persistent, the rescale (c, trait touch) earns its
//!     keep.
//!   - settled e (end of window): all three converge to the same steady state
//!     (the hint is transient-only; steady state is share-driven, unchanged).
//!     If they DON'T converge, the hint is contaminating steady state — a bug in
//!     the mechanism, not a feature.
//!
//! Metric: e = ln(Ĥ / H_true), Ĥ = operating point before the tick's decision,
//! H_true = scheduled hashrate at the interval midpoint. Reported per (rate,
//! spm): peak over-diff e%, over-diff area (e-min, e>0 only), worst under-diff
//! e% after the ease, ticks-to-recover (e back within ±2%), settled e%.
//!
//! Usage: cargo run --release --bin downward-hint
//! Env: VARDIFF_DH_TRIALS (default 300), VARDIFF_DH_THREADS, VARDIFF_SWEEP_SEED.

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use channels_sv2::target::hash_rate_to_target;
use channels_sv2::vardiff::composed::{champion_composed, Estimator};
use channels_sv2::vardiff::{Clock, MockClock, Vardiff};
use vardiff_sim::baseline::TRUE_HASHRATE;
use vardiff_sim::rng::{sample_poisson, XorShift64};
use vardiff_sim::schedule::HashrateSchedule;
use bitcoin::Target;

const TICK: u64 = 60;
const MATURE_MIN: u64 = 60; // mature on-target so the counter is settled
const DROP_AT_MIN: u64 = 60; // the step DOWN happens at the end of maturation
// The hashrate RECOVERS back to full this many minutes after the drop. Long
// enough that the decline reaction has fully settled before recovery starts, so
// the two windows don't bleed into each other. Env-overridable.
fn recover_at_min() -> u64 {
    env::var("VARDIFF_DH_RECOVER_AT_MIN").ok().and_then(|s| s.parse().ok()).unwrap_or(120)
}
// Observe this many minutes AFTER recovery, so the upward (share-driven) leg
// fully settles too. Env-overridable for the convergence tail check.
fn observe_min() -> u64 {
    env::var("VARDIFF_DH_OBSERVE_MIN").ok().and_then(|s| s.parse().ok()).unwrap_or(180)
}
const DROP_FRAC: f32 = 0.50; // 50% hashrate drop (the slow-decline gate's depth)
const POST_EASE_PROBE_TICKS: u64 = 5; // Q1: ticks after the ease to sum |e| over
// The pool eases when declared < belief by more than this (a real downward
// revision, not noise). The hint is assumed to carry the true post-drop H
// (perfect telemetry) — the BEST case; a noisy-hint arm is future work.
//
// IMPORTANT — the gate is MAGNITUDE-only here; it is NOT the full trigger. The
// production trigger must gate on PLAUSIBILITY, not share-corroboration: ease
// is the safe direction (a false ease costs only the benign wobble measured
// below, self-correcting within τ), so we eager-ease on a *plausible* downward
// declaration with no share-wait — waiting to corroborate a drop means waiting
// to observe FEWER shares, which is exactly the detection gap the hint exists
// to skip. The real work is done by a static plausibility FLOOR (the slot-2
// lesson): "drop to nominal=1" is downward and would pass this magnitude gate,
// but is a sentinel — easing to it floors the estimator and collapses
// difficulty. So production = {reject implausible-downward outright; eager-ease
// on plausible-downward}. This sim assumes plausible perfect telemetry, so it
// models only the magnitude gate; the plausibility floor is specced separately.
const HINT_DROP_GATE: f32 = 0.10;

#[derive(Clone, Copy, PartialEq)]
enum Arm {
    SharesOnly,
    EaseNoRescale,
    EaseRescale,
}
impl Arm {
    fn label(&self) -> &'static str {
        match self {
            Arm::SharesOnly => "a:shares-only",
            Arm::EaseNoRescale => "b:ease-no-rescale",
            Arm::EaseRescale => "c:ease-rescale",
        }
    }
}

fn to_target(h: f32, spm: f32) -> Target {
    hash_rate_to_target(h.max(1.0) as f64, spm.max(0.001) as f64)
        .expect("hash_rate_to_target positive inputs")
}

struct Trace {
    // --- decline window (drop → recovery): the over-difficulty leg ---
    peak_over_e: f64,   // max e>0 (%) — over-difficulty / starvation
    over_area: f64,     // ∫ max(e,0) dt  (e-min)
    // --- recovery window (recovery → end): the under-difficulty leg ---
    // After recovery, true H jumps back up while Ĥ is still LOW (the hint eased
    // it), so e goes NEGATIVE (under-difficulty, safe side). The share-driven
    // controller must tighten back. This is where arm b's desynced estimator
    // could bite — measured here, not assumed.
    rec_worst_under_e: f64, // min e<0 (%) in recovery (depth of under-diff)
    rec_under_area: f64,    // ∫ |min(e,0)| dt (e-min) in recovery
    rec_settle_min: f64,    // minutes after recovery until |e| ≤ 3% and stays
    settled_e: f64,         // e (%) at very end (must converge across arms)
    post_ease_amp: f64,     // Q1: Σ|e| over the first ticks after the ease
}

/// One decline trial for one arm. Replicates the sim trial loop inline so the
/// pool's mid-run operating-point write can be injected; share sampling and
/// target update match trial.rs exactly (Poisson(λ), λ from true/belief ratio).
fn run_decline(arm: Arm, spm: f32, seed: u64) -> Trace {
    let clock = Arc::new(MockClock::new(0));
    let mut v = champion_composed(1.0, clock.clone() as Arc<dyn Clock>);

    // Schedule: mature at TRUE, step DOWN by DROP_FRAC at drop_at, then RECOVER
    // back to full TRUE at recover_at. The recovery is share-driven in every arm
    // (the hint is downward-only), but arm b carries a desynced estimator out of
    // the ease — the recovery window is where that could bite.
    let h_post = TRUE_HASHRATE * (1.0 - DROP_FRAC);
    let drop_at = DROP_AT_MIN * 60;
    let recover_at = (DROP_AT_MIN + recover_at_min()) * 60;
    let schedule = HashrateSchedule::new(vec![
        (0, TRUE_HASHRATE),
        (drop_at, h_post),
        (recover_at, TRUE_HASHRATE),
    ]);
    let total = recover_at + observe_min() * 60;

    let mut rng = XorShift64::new(seed);
    let mut belief = TRUE_HASHRATE; // operating point Ĥ (= channel nominal reg)
    let mut target = to_target(belief, spm);

    let mut last_t = 0u64;
    let mut tick_at = TICK;
    let mut hinted = false; // pool applies the downward hint once, at the drop

    // decline-window (drop_at < t ≤ recover_at)
    let mut peak_over = 0.0f64;
    let mut over_area = 0.0f64;
    // recovery-window (t > recover_at)
    let mut rec_worst_under = 0.0f64;
    let mut rec_under_area = 0.0f64;
    let mut rec_settle_tick: Option<u64> = None;
    let mut settled = 0.0f64;
    // Q1 DIAGNOSTIC (b-vs-c mechanism): the post-ease transient amplitude =
    // Σ|e| over the first POST_EASE_PROBE_TICKS after the hint fires. Hypothesis
    // (precision give-back): c resyncs the estimator, DISCARDING the EWMA's
    // banked ~1/√(r*τ) smoothing for nominal-tracking the operating-point ease
    // already provided → c should be briefly NOISIER (larger amplitude) than b,
    // which retains the smoothing. If amp_c > amp_b, Q1 closes: the rescale is a
    // strict precision give-back, which is why it doesn't earn its keep.
    let mut hint_tick: Option<u64> = None;
    let mut post_ease_amp = 0.0f64;

    while tick_at <= total {
        // --- sample shares for this interval (matches trial.rs) ---
        let mid = (last_t + tick_at) / 2;
        let true_h = schedule.at(mid) as f64;
        let est_h = belief as f64;
        let secs = (tick_at - last_t) as f64;
        let lambda = if est_h > 0.0 {
            (true_h / est_h) * (spm as f64) * (secs / 60.0)
        } else {
            0.0
        };
        let n = sample_poisson(&mut rng, lambda);
        v.add_shares(n);

        // --- POOL PRE-STEP: apply a downward hint once, at the drop tick ---
        // Models UpdateChannel(new_nominal = true post-drop H) arriving ~at the
        // drop. Eager-ease: act only DOWNWARD, only when the revision exceeds
        // the gate. This is the pool writing the operating-point register
        // before try_vardiff — exactly the no-trait-change move.
        if arm != Arm::SharesOnly && !hinted && tick_at > drop_at {
            let declared = h_post; // perfect telemetry (best case)
            if (declared as f64) < (belief as f64) * (1.0 - HINT_DROP_GATE as f64) {
                let old = belief;
                belief = declared;
                target = to_target(belief, spm);
                if arm == Arm::EaseRescale {
                    // Resync the estimator the way a real fire would, so the
                    // smoothed rate stays consistent with the new easier target.
                    v.estimator.on_fire(belief, old);
                }
                hinted = true;
                hint_tick = Some(tick_at);
            }
        }

        // --- advance clock, run the controller (share-driven leg) ---
        let belief_before = belief;
        clock.set(tick_at);
        let res = v.try_vardiff(belief, &target, spm);
        if let Ok(Some(new_h)) = res {
            belief = new_h;
            target = to_target(new_h, spm);
        }

        // --- record e for this tick (using belief BEFORE the decision) ---
        let h_true_tick = schedule.at(tick_at.saturating_sub(TICK / 2)) as f64;
        let e = (belief_before as f64 / h_true_tick).ln() * 100.0;
        // DECLINE window: drop → recovery. The over-difficulty (costly) leg.
        if tick_at > drop_at && tick_at <= recover_at {
            if e > peak_over {
                peak_over = e;
            }
            if e > 0.0 {
                over_area += e * (secs / 60.0);
            }
        }
        // Q1 probe: Σ|e| over the first POST_EASE_PROBE_TICKS after the ease.
        if let Some(ht) = hint_tick {
            if tick_at > ht && tick_at <= ht + POST_EASE_PROBE_TICKS * TICK {
                post_ease_amp += e.abs();
            }
        }
        // RECOVERY window: after H jumps back to full. The under-difficulty
        // (safe) leg — Ĥ is low, true H is high, e<0, controller tightens back.
        if tick_at > recover_at {
            if e < rec_worst_under {
                rec_worst_under = e;
            }
            if e < 0.0 {
                rec_under_area += (-e) * (secs / 60.0);
            }
            // first tick |e| ≤ 3% and stays (3% > the ~1-2% steady jitter band)
            if e.abs() <= 3.0 {
                if rec_settle_tick.is_none() {
                    rec_settle_tick = Some(tick_at);
                }
            } else {
                rec_settle_tick = None;
            }
            settled = e;
        }

        last_t = tick_at;
        tick_at += TICK;
    }

    let rec_settle_min = rec_settle_tick
        .map(|t| (t.saturating_sub(recover_at)) as f64 / 60.0)
        .unwrap_or(f64::NAN);

    Trace {
        peak_over_e: peak_over,
        over_area,
        rec_worst_under_e: rec_worst_under,
        rec_under_area,
        rec_settle_min,
        settled_e: settled,
        post_ease_amp,
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.retain(|x| !x.is_nan());
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

struct Row {
    spm: f32,
    arm: &'static str,
    peak_over: f64,
    over_area: f64,
    rec_worst_under: f64,
    rec_under_area: f64,
    rec_settle_min: f64,
    settled: f64,
    post_ease_amp: f64,
}

fn main() {
    let trials: usize = env::var("VARDIFF_DH_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let base_seed: u64 = env::var("VARDIFF_SWEEP_SEED")
        .ok()
        .and_then(|s| s.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()).or_else(|| s.parse().ok()))
        .unwrap_or(0xD0_1D_FACE);
    let n_threads: usize = env::var("VARDIFF_DH_THREADS")
        .ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)).max(1);

    // Slot rates (3=6spm, 4=30spm) + the sub-guard 2spm (worst decline-safety
    // regime, where the over-difficulty lag is largest) + a mid anchor.
    let spms = [2.0f32, 6.0, 12.0, 30.0];
    let arms = [Arm::SharesOnly, Arm::EaseNoRescale, Arm::EaseRescale];
    let jobs: Vec<(f32, Arm)> = spms.iter().flat_map(|&s| arms.iter().map(move |&a| (s, a))).collect();

    eprintln!(
        "downward-hint: {} cells × {} trials, {} threads. 50% drop at {}min, recover at +{}min, observe +{}min.",
        jobs.len(), trials, n_threads, DROP_AT_MIN, recover_at_min(), observe_min()
    );

    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Row>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..n_threads {
            scope.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                if j >= jobs.len() {
                    break;
                }
                let (spm, arm) = jobs[j];
                let (mut po, mut oa, mut rwu, mut rua, mut rsm, mut st, mut pea) =
                    (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
                for i in 0..trials {
                    let t = run_decline(arm, spm, base_seed.wrapping_add((j * 100003 + i) as u64));
                    po.push(t.peak_over_e);
                    oa.push(t.over_area);
                    rwu.push(t.rec_worst_under_e);
                    rua.push(t.rec_under_area);
                    rsm.push(t.rec_settle_min);
                    st.push(t.settled_e);
                    pea.push(t.post_ease_amp);
                }
                out.lock().unwrap().push(Row {
                    spm, arm: arm.label(),
                    peak_over: median(po), over_area: median(oa),
                    rec_worst_under: median(rwu), rec_under_area: median(rua),
                    rec_settle_min: median(rsm), settled: median(st),
                    post_ease_amp: median(pea),
                });
            });
        }
    });

    let mut rows = out.into_inner().unwrap();
    rows.sort_by(|a, b| (a.spm as u32, a.arm).partial_cmp(&(b.spm as u32, b.arm)).unwrap());

    println!("\n## Downward-hint fusion: DECLINE (50% drop) then RECOVERY (back to full). e=ln(Ĥ/H_true).");
    println!("Hint = pool writes operating-point Ĥ←declared at the drop (eager-ease, downward-only). Champion Ewma360/s1.5.");
    println!("Decline leg: e>0 over-difficulty (costly). Recovery leg: e<0 under-difficulty (safe) while Ĥ catches up.\n");
    println!("| spm | arm | DECLINE peak over-e% | decline over-area | REC worst under-e% | rec under-area | rec settle(min) | settled e% |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in &rows {
        println!(
            "| {} | {} | {:+.1} | {:.1} | {:+.1} | {:.1} | {:.0} | {:+.1} |",
            r.spm as u32, r.arm, r.peak_over, r.over_area,
            r.rec_worst_under, r.rec_under_area, r.rec_settle_min, r.settled
        );
    }

    println!("\n## RECOVERY comparison — does the downward hint slow/distort the upward leg?");
    println!("The hint is downward-only, so recovery is share-driven in ALL arms. Q: does arm b's desynced");
    println!("estimator (eased Ĥ without on_fire rescale) recover WORSE than shares-only (a) or rescale (c)?");
    println!("| spm | a rec-settle | b rec-settle | c rec-settle | a under-area | b under-area | c under-area |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for &spm in &spms {
        let get = |label: &str| rows.iter().find(|r| r.spm == spm && r.arm == label);
        if let (Some(a), Some(b), Some(c)) =
            (get("a:shares-only"), get("b:ease-no-rescale"), get("c:ease-rescale"))
        {
            println!(
                "| {} | {:.0}m | {:.0}m | {:.0}m | {:.1} | {:.1} | {:.1} |",
                spm as u32,
                a.rec_settle_min, b.rec_settle_min, c.rec_settle_min,
                a.rec_under_area, b.rec_under_area, c.rec_under_area
            );
        }
    }
    println!("\nKEY: arm 'a' (shares-only) recovery is the BASELINE the controller was selected against — it");
    println!("recovers from a 50% drop with no hint. If b's rec-settle and under-area ≈ a's, the hint leaves");
    println!("recovery UNTOUCHED (it only acted on the decline). If b ≫ a, the eased-but-desynced state has a");
    println!("recovery cost the over-difficulty saving must be weighed against.");

    println!("\n## Q1 mechanism — post-ease transient amplitude (Σ|e| over first {} ticks after the ease).", POST_EASE_PROBE_TICKS);
    println!("Precision-give-back hypothesis: c resyncs (discards EWMA banked smoothing) → c should be NOISIER");
    println!("than b (which retains it). If amp_c > amp_b, the rescale is a strict precision give-back → Q1 closes.");
    println!("| spm | b amp (no-rescale) | c amp (rescale) | c − b | verdict |");
    println!("| --- | --- | --- | --- | --- |");
    for &spm in &spms {
        let get = |label: &str| rows.iter().find(|r| r.spm == spm && r.arm == label);
        if let (Some(b), Some(c)) = (get("b:ease-no-rescale"), get("c:ease-rescale")) {
            let d = c.post_ease_amp - b.post_ease_amp;
            let verdict = if d > 1.0 { "c noisier (give-back ✓)" } else if d < -1.0 { "b noisier (✗ refutes)" } else { "≈ (inconclusive)" };
            println!(
                "| {} | {:.1} | {:.1} | {:+.1} | {} |",
                spm as u32, b.post_ease_amp, c.post_ease_amp, d, verdict
            );
        }
    }
}
