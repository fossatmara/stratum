//! DEPLOYMENT-COUPLING, CORRECT GATE — the transient-ESCAPE gate, measured in the
//! CATCH-UP window and gated on STALL (not breach-duration). Bounds τ from ABOVE.
//!
//! ===========================================================================
//! WHY THIS EXISTS (the correction chain — read it, the gate was wrong twice and
//! the reasons matter). deploy-coupling.rs swept the SETTLED-e (asymptote) gate and
//! found it SLACK in the dense regime (every τ≤900 settles −6…−8% safe). Reading
//! "slack → only escape-vs-wobble cost left → wall 3 collapses into the refused
//! weighting (wall 4)" is WRONG: the asymptote gate is STRUCTURALLY BLIND to the
//! escape transient (it averages out the trough). So a transient gate is mandatory.
//!
//! FIRST transient attempt (this file, v1) measured the longest breach RUN (e>5%
//! sustained ≥ W) over the WHOLE d_start..trial_end window and FAILED the champion
//! everywhere but the vacuous δ_clock=0.1 corner. Two compounding errors, both the
//! depth-vs-duration confusion:
//!   ERROR 1 (window): the d_start..d_end span is the FORCED DECLINE — true H is
//!     still FALLING. Over-difficulty there is moving-target LAG, not spiral; the
//!     W-detection logic assumes a FIXED true value, so it misread chase-lag as
//!     "can't correct." FIX: measure only the CATCH-UP window d_end..trial_end,
//!     where true H is FIXED at the floor and "breach persists = can't correct" is
//!     a valid inference.
//!   ERROR 2 (quantity): even in the catch-up window, breach DURATION still
//!     confuses SLOW-but-correcting with STUCK. A sleepy window enters catch-up
//!     deeply lagged and grinds the over-difficulty down MONOTONICALLY but SLOWLY —
//!     a long breach run, yet self-correcting the whole time (NOT spiral). The
//!     champion's v1 peak escape-e was only +20% (e=0.20, below δ_clock=0.5 depth)
//!     — LONG, not DEEP. FIX: gate on STALL, not duration — spiral is over-
//!     difficulty that FAILS TO DECREASE (the e^(−e) collapse starves the shares
//!     needed to correct, so the breach stalls/deepens), NOT over-difficulty that's
//!     merely slow to clear. Slow-but-monotone = correcting = admissible (just
//!     sleepy); stuck-or-deepening = spiral = inadmissible.
//!
//! STALL, OPERATIONALIZED (progress, not rate — and NOT "slower than the floor").
//! "Slower than the floor-permitted correction rate" is the WRONG operationalization:
//! a long-τ EWMA corrects slower than the floor BY DESIGN (it averages more than the
//! floor-minimum to buy noise reduction — a chosen bias-variance point, e.g. the
//! champion's 360s window is ~9× the floor's ~40s τ_min at spm6/δ=0.5). Gating on
//! "slower than floor" would reject the champion for being deliberately sleepy — the
//! depth-vs-duration trap a third time. The spiral signature is the SIGN of progress,
//! not its rate: is the correction PROCEEDING (de/dt<0, the running-minimum keeps
//! improving) or STALLED (running-min stuck while breached)? A descending trajectory
//! — however slow — keeps ratcheting its running-min DOWN and never accumulates
//! stall; a stuck/deepening one stops improving its min and accumulates stall. So:
//!   STALL RUN = longest contiguous span of CATCH-UP ticks that are BREACHED (e>5%,
//!   δ_threshold inherited) AND make NO new running-minimum (no net downward
//!   progress). FAIL iff stall-run ≥ W(spm,k,δ_clock). A sleepy-but-monotone window
//!   PASSES (its min keeps dropping); a starved/spiraling window FAILS (min stuck).
//!   The floor-rate comparison is EMITTED AS A DIAGNOSTIC (champion table below), so
//!   the data SHOWS the champion is monotone-but-slower-than-floor — proving the
//!   rate operationalization would wrongly reject it and the progress one passes it.
//!
//! W (the sustain window; depth δ_clock sets the timescale, z² folds into k):
//!   W(spm) = k · e^(δ_clock) / (spm · δ_clock²)  [minutes]. δ_threshold=5% is the
//!   LEVEL (breach); δ_clock∈[0.1,0.9] swept is the spiral-DEPTH (timescale). Two-δ
//!   separation preserved. SWEPT: k∈{0.5,1,2,4}, δ_clock∈{0.1,0.3,0.5,0.7,0.9}.
//!   tick=60s=1min, so a stall run of n ticks = n minutes. At high spm W<1min
//!   (sub-tick) → gate degenerates to "one stalled breach tick fails" — conservative
//!   (the benign direction for a bound-from-above question).
//!
//! SCENARIO — DECLINE escape, bounds τ from ABOVE (NOT the jumpy-overshoot trough-
//! gate, which tested SHORT windows overshooting during a SPIKE, bounds from BELOW).
//! Same gate OBJECT (sustained spiral-risk over-difficulty during a transient),
//! OPPOSITE window-length regime + opposite scenario. The rig drives a DECLINE and
//! measures the post-decline CATCH-UP stall.
//!
//! CHAMPION BASELINE (calibration control, non-optional). Run τ=360 through the SAME
//! gate. The champion is floor-limited (slow) but NEVER stalled, so it MUST pass at
//! meaningful δ_clock — if it doesn't, the gate is still mis-specified, not the
//! window inadmissible. A FAIL is a finding only where the champion CLEARS the gate.
//!
//! THE DECISION (with the right gate):
//!   τ_deploy_combined(spm) = longest τ passing BOTH settled≤5% AND stall<W.
//!   (A) FLAT ≈360 across boundary regime ⇒ champion ~right at every rate;
//!       deployment rate-awareness ~no gain; p<1 is over-difficulty-axis only. STOP.
//!   (B) SLIDES beyond bars ⇒ real deployment gain; SIGN = which way to couple
//!       (shorter-at-high-rate = same as share-indexing p>0; longer = OPPOSITE,
//!       negative-p, share-indexing backwards for deployment).
//!   (C) stall gate ALSO slack at high rate among champ-PASS cells (sleepiest τ
//!       never stalls) ⇒ wall 3 genuinely collapses into wall 4, established with
//!       the CORRECT gate — the only honest route to the collapse.
//!
//! Usage: cargo run --release --bin deploy-coupling-transient
//! Env: VARDIFF_DCT_TRIALS (default 120 base, CI-scaled), VARDIFF_DCT_THREADS.
//! ===========================================================================
//!
//! ===========================================================================
//! RESULT — outcome (C), but REGIME-SPLIT (the guard boundary asserts itself one
//! last time). The verdict is read from DEPTH, not the stall metric: the stall
//! (duration) approach was ABANDONED because the synthetic controls FALSIFIED it,
//! and depth is the quantity the gate is actually about (spiral = over-difficulty
//! deep enough for e^(−e) to bite). Two findings, equal prominence:
//!
//! WHY THE STALL METRIC WAS ABANDONED (the control did its job — falsified the
//! metric BEFORE a verdict was read off it). Synthetic scorer controls (sim-
//! independent, in main()): a perfectly-clean MONOTONE shallow-slow descent
//! ("champ-shape clean") scored a 7-min DEBOUNCED stall — because its per-tick
//! descent (0.30%/min) is below the noise tolerance, and NO per-tick-tolerance
//! metric can separate slow-healthy from stalled when the true descent-per-tick is
//! sub-noise (they are identical one tick at a time). Pre-registered control
//! requirement (preserve a genuine stall AND pass a clean slow descent) → FAILED →
//! metric not calibrated → its τ_deploy verdict is NOT read. (Also: the RAW scorer,
//! tol=0, test `e ≤ anchor` counts EQUAL as progress, so "raw stall=0" means "never
//! STRICTLY rose" — which a flat/stalled trajectory ALSO satisfies; flat-held-high
//! scored raw stall=0/mono=1.00, the canonical stall reading as perfectly monotone.
//! So "monotone-correcting" from the raw scorer is NOT evidence of monotonicity.)
//!
//! THE DEPTH VERDICT (scorer-independent; e% = ln(belief/true)·100, units match
//! δ_clock: e%=30 ↔ δ_clock=0.3). Non-vacuous spiral depth is δ_clock ≥ 0.3 (≥30%);
//! the sleepiest representable window τ=900 is the worst case. CRUCIAL DISTINCTION:
//! e^(−e) integrates the excursion, so the spiral-relevant quantity is the SUSTAINED
//! depth (mean e over the breach), NOT the momentary PEAK — and sustained < peak
//! always, which is what makes a thin peak-margin sound.
//!   · DENSE regime (boundary, spm≥6): at τ=900 (sleepiest) the PEAK is +22–24%
//!     (spm6–12) — clearing the shallowest non-vacuous spiral floor (30%) by only
//!     ~6–8pp, THIN. But the SUSTAINED depth is +13%, clearing 30% by ~17pp —
//!     COMFORTABLE. And spm≥20 has NO BREACH at all even at τ=900 (settles −1…−3%).
//!     So the escape is sub-spiral even at maximum sleepiness — and it is the
//!     SUSTAINED depth (+13%, +17pp margin) that makes the thin peak-margin safe:
//!     the peak barely clears, the quantity that drives the spiral clears
//!     comfortably. ⇒ depth-gate has NOTHING to bind ⇒ deployment-τ NOT
//!     admissibility-bounded above; what bounds it is the shallow escape-COST vs
//!     wobble = the refused scalar (wall 4). **Wall 3 COLLAPSES into wall 4 in the
//!     dense regime — ESTABLISHED (sustained-depth-based). Terminus: the policy
//!     weighting (operator's, not the lab's).** Outcome (C), correct gate, as
//!     pre-registered ("at high rate so many shares even a slow window detects the
//!     drop fast enough").
//!   · SPARSE/GUARD regime (spm 2,4 — separate controller): PEAK +28–43% BRACKETS
//!     and at the deep end EXCEEDS δ_clock=0.3 (30%); but SUSTAINED is +17–20% —
//!     below 30%. So it is spiral-ADJACENT on the peak, sub-spiral on the sustained.
//!     **The collapse is NOT established here: the peak excursion means the gate
//!     COULD bind, so the coupling question may remain ADMISSIBILITY-MEASURABLE at
//!     low rate — UNDETERMINED (leaning sub-spiral on sustained, but the peak
//!     forbids the clean dense-regime call).** "Monotone-correcting therefore fine"
//!     is NOT available (it rests on the falsified raw scorer — see below); the
//!     robust statement is the depth, and the depth says UNDETERMINED. Resolving it
//!     needs the dedicated low-rate study, not this band-spanning sweep.
//!
//! THE HONEST "WHERE WE ARE" (the regime-split is the finding, not a caveat). The
//! deployment-coupling question COLLAPSES into the refused weighting AT HIGH RATE —
//! which is the regime where spiral was NEVER the danger — and remains OPEN and
//! potentially admissibility-measurable AT LOW RATE — which is exactly where spiral
//! always WAS the binding danger of the whole design. So the chain terminates at
//! POLICY for the safe (dense) regime, and at UNDETERMINED-would-need-a-low-rate-
//! specific-study for the dangerous (sparse) regime. The split is the SAME guard
//! boundary (spm_threshold=6) that fractured the τ* slope, the shares-optimum, and
//! every band-spanning measurement in this arc — flattening the FINAL verdict into
//! one statement would be the one place that boundary got ignored.
//!
//! WHY NO FOURTH METRIC (moot in BOTH regimes). Dense: depth already settles it
//! (sub-spiral), a working stall metric would only relabel shallow cost excursions.
//! Guard: the question there is not "is it stalling" but "is the escape deep enough
//! to gate," and depth already answers that — UNDETERMINED (brackets spiral depth);
//! no stall metric recovers determinacy. Only a DIFFERENT study (finer depth
//! resolution, or the low-rate transient characterized properly) could — that is
//! not the fourth metric, it is a separate low-rate study, left for whoever pursues
//! rate-awareness, because the low-rate corner is the ONLY regime with possible
//! measurable admissibility content.
//!
//! STATUS: deploy-coupling.rs (asymptote gate, slack at high rate — the WRONG gate,
//! asymptote-blind-to-transient) + this rig (transient depth gate). Verdict:
//! REGIME-SPLIT (C). Dense → collapse into wall 4 (established, policy terminus);
//! guard → undetermined (spiral-adjacent, gate may bind, separate low-rate study).
//! Stall (duration) metric abandoned (controls falsified it); DEPTH is the verdict.
//! ===========================================================================

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use channels_sv2::vardiff::composed::{
    AcceleratingPartialRetarget, AdaptiveSignPersist, Composed, EwmaEstimator,
    SignPersistenceCusumBoundary,
};
use channels_sv2::vardiff::MockClock;
use vardiff_sim::baseline::{Phase, Scenario, DEFAULT_BASELINE_SEED, TRUE_HASHRATE};
use vardiff_sim::grid::{AlgorithmSpec, VardiffBox};
use vardiff_sim::trial::{run_trial_observed, TrialConfig};

const BREACH_PCT: f64 = 5.0;        // δ_threshold, inherited level
const SETTLED_GATE_PCT: f64 = 5.0;  // asymptote gate, still required (both must pass)
const SENS: f64 = 1.5;
const Z2: f64 = 3.84;               // z² at 95% (1.96²) — diagnostic floor τ_min only
const SPMS_BOUNDARY: &[f32] = &[6.0, 8.0, 12.0, 20.0, 30.0];
const SPMS_GUARD: &[f32] = &[2.0, 4.0];
const TAUS: &[u64] = &[60, 90, 120, 150, 240, 300, 360, 480, 600, 720, 900];
const RATES_PPH: &[f32] = &[1.0, 2.0, 5.0, 10.0, 20.0, 40.0];
const KS: &[f64] = &[0.5, 1.0, 2.0, 4.0];
const DELTA_CLOCKS: &[f64] = &[0.1, 0.3, 0.5, 0.7, 0.9];

fn cfg(tau: u64, sens: f64) -> AlgorithmSpec {
    AlgorithmSpec::new(format!("Ewma{tau}/s{sens}"), move |clock| {
        VardiffBox(Box::new(Composed::new(
            EwmaEstimator::new(tau),
            AdaptiveSignPersist::sign_persist(
                SignPersistenceCusumBoundary::new(sens, 0.05, 8.0, 0.06, 0.6),
                6,
            ),
            AcceleratingPartialRetarget::new(0.2, 0.6, 0.05),
            1.0,
            clock,
        )))
    })
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() { return f64::NAN; }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// W(spm) in MINUTES: floor-limited spiral-onset sustain window. z² folded into k.
fn sustain_window_min(spm: f32, k: f64, delta_clock: f64) -> f64 {
    k * delta_clock.exp() / (spm as f64 * delta_clock * delta_clock)
}

/// Floor-limited detection time τ_min in MINUTES (DIAGNOSTIC ONLY — to show the
/// champion corrects slower than the floor by design). Resolve depth δ_clock at
/// rate r (shares/min): N ≥ z²/δ² shares ⇒ τ_min = z²/(r·δ²) min.
fn floor_taumin_min(spm: f32, delta_clock: f64) -> f64 {
    Z2 / (spm as f64 * delta_clock * delta_clock)
}

/// THE STALL SCORER (net-descent anchor rule, debounced at `tol`). A breached tick
/// counts as STALL unless the level has ratcheted DOWN by more than `tol` from the
/// current anchor (the anchor descends only on real, beyond-noise progress). This
/// is the rule that correctly FAILS a flat-held-high trajectory (anchor never
/// descends → all-stall) while PASSING any trajectory that descends faster than the
/// noise (anchor ratchets → stall resets). The symmetric run_min±tol rule was
/// REJECTED: it passes flat stalls (run_min locks to the flat level → never
/// exceeds run_min+tol → no stall), breaking the genuine-stall control.
/// Returns (longest_stall_run_min, monotone_frac = fraction of breached ticks that
/// made beyond-noise progress).
fn score_stall(etrace: &[f64], tol: f64) -> (f64, f64) {
    let (mut anchor, mut cur, mut maxrun) = (f64::INFINITY, 0.0f64, 0.0f64);
    let (mut breached, mut progressed) = (0.0f64, 0.0f64);
    for &e in etrace {
        if e > BREACH_PCT {
            breached += 1.0;
            if anchor.is_infinite() { anchor = e; }
            if e <= anchor - tol {            // beyond-noise net descent → progress
                anchor = e;
                progressed += 1.0;
                cur = 0.0;
            } else {                          // no real progress → stall
                cur += 1.0;
                if cur > maxrun { maxrun = cur; }
            }
        } else {
            anchor = f64::INFINITY;
            cur = 0.0;
        }
    }
    (maxrun, if breached > 0.0 { progressed / breached } else { 1.0 })
}

/// Per-trial catch-up measurements (window d_end..trial_end, true H FIXED at floor).
struct Catchup {
    settled_e: f64,
    catch_etrace: Vec<f64>, // per-tick e over the catch-up window (scored later, raw+debounced)
    noise_std: f64,         // EMPIRICAL tick-to-tick e jitter during the mature (converged) phase
    peak_e: f64,
    e_start: f64,
}

/// One (τ,rate,spm) trial → Catchup. VERBATIM decline profile from
/// tau-family-safety.rs; only the MEASUREMENT window + quantity differ.
fn trial(tau: u64, rate_pph: f32, spm: f32, seed: u64) -> Catchup {
    let a = cfg(tau, SENS);
    let mature = 60u64;
    let rate = rate_pph / 100.0 / 60.0;
    let target = 0.50f32;
    let observe = 120u64;
    let mut phases = vec![Phase::Hold { secs: mature * 60, h: TRUE_HASHRATE }];
    let mut dm = 0u64;
    for m in 0..300u64 {
        let frac = (rate * (m as f32 + 1.0)).min(target);
        phases.push(Phase::Hold { secs: 60, h: TRUE_HASHRATE * (1.0 - frac) });
        dm = m + 1;
        if frac >= target { break; }
    }
    let floor_h = TRUE_HASHRATE * (1.0 - (rate * dm as f32).min(target));
    phases.push(Phase::Hold { secs: observe * 60, h: floor_h });
    let scen = Scenario::Custom { name: "decline".into(), phases, initial_estimate: None };
    let (proto, sched) = scen.build(spm);
    let config = TrialConfig { tick_interval_secs: 60, ..proto };
    let clock = Arc::new(MockClock::new(0));
    let v = (a.factory)(clock.clone());
    let d_start = mature * 60;
    let d_end = (mature + dm) * 60;
    let trial_end = d_end + observe * 60;
    let t = run_trial_observed(v, clock, config, &sched, seed);

    let (mut settled, mut peak, mut e_start, mut started) = (0.0f64, f64::MIN, 0.0f64, false);
    let mut catch_etrace = Vec::new();
    // mature-phase e values (converged, true fixed) → EMPIRICAL noise floor.
    let mut mature_es: Vec<f64> = Vec::new();
    for tk in &t.ticks {
        let h_true = sched.at(tk.t_secs.saturating_sub(30)) as f64;
        let e = (tk.current_hashrate_before as f64 / h_true).ln() * 100.0;
        // mature phase: after warmup (skip first 20 min), before decline starts.
        if tk.t_secs > 20 * 60 && tk.t_secs <= d_start { mature_es.push(e); }
        if tk.t_secs > d_end && tk.t_secs <= trial_end {
            if !started { e_start = e; started = true; }
            if e > peak { peak = e; }
            settled = e;
            catch_etrace.push(e);
        }
    }
    // empirical noise = std of tick-to-tick differences of mature e (the jitter the
    // running-min must beat to count as real progress), /√2 to get per-sample σ.
    let noise_std = if mature_es.len() > 2 {
        let diffs: Vec<f64> = mature_es.windows(2).map(|w| w[1] - w[0]).collect();
        let m = diffs.iter().sum::<f64>() / diffs.len() as f64;
        let var = diffs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (diffs.len() as f64 - 1.0);
        (var / 2.0).sqrt()
    } else { f64::NAN };

    Catchup { settled_e: settled, catch_etrace, noise_std, peak_e: peak, e_start }
}

struct TauCell {
    tau: u64,
    worst_settled: f64,
    worst_stall_raw: f64,  // stall scored at tol=0 (the v2 metric — noise-blind)
    worst_stall_deb: f64,  // stall scored at tol=empirical noise (the debounced metric)
    peak_at_worst: f64,    // MOMENTARY max depth at the worst-stall severity
    sustained_at_worst: f64, // MEAN e over BREACHED catch-up ticks — the spiral-relevant
                             // depth (e^(−e) integrates over the excursion, not the peak)
    e_start_at_worst: f64,
    monotone_deb: f64,     // monotone_frac under the debounced scorer
    noise_at_worst: f64,   // empirical tol used (per-σ tick jitter) at the worst severity
}

/// gate uses the DEBOUNCED stall — the v2 raw metric is kept only for the
/// before/after calibration table.
fn tau_cells(spm: f32, base: usize, seed: u64) -> Vec<TauCell> {
    let ct = (base as f64 * (60.0 / spm as f64).max(1.0)).round() as usize;
    TAUS.iter().enumerate().map(|(ti, &tau)| {
        let (mut ws, mut wdeb) = (f64::MIN, f64::MIN);
        let (mut wraw, mut pk, mut sus, mut estart, mut mono, mut noi) =
            (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN);
        for (k, &r) in RATES_PPH.iter().enumerate() {
            let (mut settleds, mut raws, mut debs) =
                (Vec::with_capacity(ct), Vec::with_capacity(ct), Vec::with_capacity(ct));
            let (mut peaks, mut sustaineds, mut estarts, mut monos, mut noises) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for i in 0..ct {
                let c = trial(tau, r, spm,
                    seed.wrapping_add((ti as u64) << 32).wrapping_add((k as u64) << 16).wrapping_add(i as u64));
                let tol = if c.noise_std.is_finite() { c.noise_std } else { 0.0 };
                let (raw, _) = score_stall(&c.catch_etrace, 0.0);
                let (deb, mono_d) = score_stall(&c.catch_etrace, tol);
                // SUSTAINED breach depth = mean e over breached catch-up ticks (the
                // spiral-relevant quantity: e^(−e) integrates the excursion, not the
                // peak). NaN if no breach (excursion never crossed threshold).
                let breached: Vec<f64> = c.catch_etrace.iter().copied().filter(|&e| e > BREACH_PCT).collect();
                let sustained = if breached.is_empty() { f64::NAN }
                    else { breached.iter().sum::<f64>() / breached.len() as f64 };
                settleds.push(c.settled_e); raws.push(raw); debs.push(deb);
                peaks.push(c.peak_e); sustaineds.push(sustained);
                estarts.push(c.e_start); monos.push(mono_d); noises.push(tol);
            }
            let (se, deb) = (median(settleds), median(debs));
            if se > ws { ws = se; }
            if deb > wdeb {           // worst transient = longest DEBOUNCED stall
                wdeb = deb;
                wraw = median(raws);
                pk = median(peaks);
                let valid_sus: Vec<f64> = sustaineds.into_iter().filter(|x| x.is_finite()).collect();
                sus = if valid_sus.is_empty() { f64::NAN } else { median(valid_sus) };
                estart = median(estarts);
                mono = median(monos);
                noi = median(noises);
            }
        }
        TauCell { tau, worst_settled: ws, worst_stall_raw: wraw, worst_stall_deb: wdeb,
                  peak_at_worst: pk, sustained_at_worst: sus, e_start_at_worst: estart,
                  monotone_deb: mono, noise_at_worst: noi }
    }).collect()
}

fn tau_deploy_combined(cells: &[TauCell], spm: f32, k: f64, dc: f64) -> Option<u64> {
    let w = sustain_window_min(spm, k, dc);
    cells.iter()
        .filter(|c| c.worst_settled <= SETTLED_GATE_PCT && c.worst_stall_deb < w)
        .map(|c| c.tau)
        .max()
}

fn champ_pass(cells: &[TauCell], spm: f32, k: f64, dc: f64) -> Option<bool> {
    let w = sustain_window_min(spm, k, dc);
    cells.iter().find(|c| c.tau == 360).map(|c| c.worst_stall_deb < w)
}

fn main() {
    let base: usize = env::var("VARDIFF_DCT_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    let seed = DEFAULT_BASELINE_SEED ^ 0x7A_45_18;
    let nth: usize = env::var("VARDIFF_DCT_THREADS").ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)).max(1);

    let all_spms: Vec<f32> = SPMS_BOUNDARY.iter().chain(SPMS_GUARD).copied().collect();
    eprintln!("deploy-coupling-transient v3 (catch-up window, DEBOUNCED stall gate): {} spm × {} τ × {} sev, base {} trials, {} threads.",
        all_spms.len(), TAUS.len(), RATES_PPH.len(), base, nth);

    // --- SYNTHETIC SCORER CONTROLS (metric-level, sim-independent). Confirm the
    //     debounced scorer is CALIBRATED not TUNED: a flat-held-high trace must
    //     STALL (anchor never descends); a deep-fast-descent must PASS; a
    //     champion-shaped shallow-slow-descent-on-noise is the case in question.
    //     tol here = 2.0 (a representative noise σ); the point is the SHAPE response.
    println!("## SYNTHETIC SCORER CONTROLS (sim-independent — is the debounced stall metric calibrated or tuned?). tol=2.0.");
    let mk = |v: &[f64]| v.to_vec();
    let flat_high   = mk(&[20.0; 30]);                                  // held high → must STALL (full run)
    let deep_desc   = mk(&(0..30).map(|i| 25.0 - 1.5 * i as f64).collect::<Vec<_>>()); // fast descent → must PASS
    let champ_shape: Vec<f64> = (0..30).map(|i| (9.0 - 0.30 * i as f64).max(-3.0)).collect(); // shallow slow → in question
    let champ_noisy: Vec<f64> = champ_shape.iter().enumerate()        // + alternating ±2.5 jitter (sub-noise wiggle)
        .map(|(i, &e)| e + if i % 2 == 0 { 2.5 } else { -2.5 }).collect();
    for (name, tr, expect) in [
        ("flat-held-high (genuine stall)", &flat_high, "STALL (run≈30)"),
        ("deep-fast-descent (healthy)", &deep_desc, "PASS (run≈0)"),
        ("champ-shape clean (shallow slow)", &champ_shape, "PASS if descent>tol/step"),
        ("champ-shape + sub-noise jitter", &champ_noisy, "PASS iff debounce works"),
    ] {
        let (raw, mraw) = score_stall(tr, 0.0);
        let (deb, mdeb) = score_stall(tr, 2.0);
        println!("  {:38} raw stall={:>4.0} mono={:.2} | debounced stall={:>4.0} mono={:.2}  → expect {}",
            name, raw, mraw, deb, mdeb, expect);
    }
    println!("  (flat-high MUST stay stalled after debounce — else tol too big, rescuing everything. champ+jitter SHOULD collapse to ~deep-desc — that's the artifact fix.)\n");

    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(f32, Vec<TauCell>)>> = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..nth.min(all_spms.len()) {
            sc.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                if j >= all_spms.len() { break; }
                let spm = all_spms[j];
                let cells = tau_cells(spm, base, seed.wrapping_add((j as u64) << 8));
                out.lock().unwrap().push((spm, cells));
                eprintln!("  spm{} done", spm);
            });
        }
    });
    let raw = out.into_inner().unwrap();
    let cells_for = |spm: f32| -> &Vec<TauCell> { &raw.iter().find(|(s, _)| *s == spm).unwrap().1 };

    // --- CALIBRATION CONTROL FIRST: the champion catch-up diagnostic. The gate is
    //     only trustworthy if τ=360 passes at meaningful δ_clock. Show WHY it should:
    //     monotone (correcting, not stalled) but slower-than-floor (sleepy by design).
    println!("\n## CALIBRATION CONTROL — champion τ=360 stall RAW vs DEBOUNCED (the artifact test).");
    println!("PRE-REGISTERED PREDICTION (Reading A): if the raw low-monotone was sub-noise jitter, debouncing at the empirical noise σ");
    println!("collapses the champion's stall toward 0 and lifts monotone toward 1. If raw stall SURVIVES debounce, it's a real stall");
    println!("(Reading B) OR the descent is below tol (peak too shallow to register progress — a 3rd case: sub-noise SIGNAL, gate inapt).");
    println!("floor τ_min(δ.5) shown: champion halflife > this = corrects slower than floor BY DESIGN (why 'slower-than-floor' gate is wrong).\n");
    println!("| spm | PEAK-e% (momentary) | SUSTAINED-e% (mean over breach — spiral-relevant) | e_start% | noise σ (tol) | stall RAW | stall DEB | floor τ_min(δ.5) |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for &spm in SPMS_BOUNDARY.iter().chain(SPMS_GUARD) {
        let c = cells_for(spm).iter().find(|c| c.tau == 360).unwrap();
        println!("| {} | {:+.0} | {} | {:+.0} | {:.1} | {:.0} | {:.0} | {:.2} |",
            spm as u32, c.peak_at_worst,
            if c.sustained_at_worst.is_finite() { format!("{:+.0}", c.sustained_at_worst) } else { "no-breach".into() },
            c.e_start_at_worst, c.noise_at_worst,
            c.worst_stall_raw, c.worst_stall_deb, floor_taumin_min(spm, 0.5));
    }
    // τ=900 (SLEEPIEST representable window — worst case for the dense-regime sub-spiral claim):
    //   peak vs SUSTAINED depth, against the shallowest non-vacuous spiral depth (δ_clock=0.3 ↔ 30%).
    println!("\n### τ=900 (sleepiest window) — PEAK vs SUSTAINED depth vs spiral floor (δ_clock=0.3 ↔ 30%):");
    println!("| spm | peak-e% | SUSTAINED-e% | margin to 30% (sustained) |");
    println!("| --- | --- | --- | --- |");
    for &spm in SPMS_BOUNDARY.iter().chain(SPMS_GUARD) {
        let c = cells_for(spm).iter().find(|c| c.tau == 900).unwrap();
        let (pk, su) = (c.peak_at_worst, c.sustained_at_worst);
        println!("| {} | {:+.0} | {} | {} |", spm as u32, pk,
            if su.is_finite() { format!("{:+.0}", su) } else { "no-breach".into() },
            if su.is_finite() { format!("{:+.0}pp", 30.0 - su) } else { "n/a (no breach)".into() });
    }

    // --- reference-gate escape profile + τ_deploy ---
    println!("\n## TRANSIENT-ESCAPE (STALL) GATE — reference k=1, δ_clock=0.5 ⇒ W=6.59/spm min. FAIL iff worst stall_run ≥ W.");
    println!("| spm | W_ref(min) | τ=360 stall/peak | champ360 | longest stall-PASS τ | τ=900 stall/peak | τ_deploy_combined |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let mut boundary_deploys: Vec<(f32, Option<u64>)> = Vec::new();
    for &spm in SPMS_BOUNDARY.iter().chain(SPMS_GUARD) {
        let cells = cells_for(spm);
        let w = sustain_window_min(spm, 1.0, 0.5);
        let c360 = cells.iter().find(|c| c.tau == 360).unwrap();
        let c900 = cells.iter().find(|c| c.tau == 900).unwrap();
        let stall_pass = cells.iter().filter(|c| c.worst_stall_deb < w).map(|c| c.tau).max();
        let champ = champ_pass(cells, spm, 1.0, 0.5).unwrap();
        let td = tau_deploy_combined(cells, spm, 1.0, 0.5);
        if SPMS_BOUNDARY.contains(&spm) { boundary_deploys.push((spm, td)); }
        println!("| {} | {:.2} | {:.0}/{:+.0} | {} | {} | {:.0}/{:+.0} | {} |",
            spm as u32, w, c360.worst_stall_deb, c360.peak_at_worst,
            if champ {"PASS"} else {"**FAIL**"},
            stall_pass.map(|t| t.to_string()).unwrap_or_else(|| "NONE".into()),
            c900.worst_stall_deb, c900.peak_at_worst,
            td.map(|t| t.to_string()).unwrap_or_else(|| "NONE".into()));
    }

    let lo = boundary_deploys.first().and_then(|(_, t)| *t);
    let hi = boundary_deploys.last().and_then(|(_, t)| *t);
    println!("\n**Reference-gate verdict (spm6 vs spm30, k=1 δ=0.5):**");
    match (lo, hi) {
        (Some(l), Some(h)) if l == h =>
            println!("  τ_deploy_combined FLAT at {} ⇒ (A) champion-class ~right at every rate. CHECK sensitivity sweep before STOP.", l),
        (Some(l), Some(h)) if h > l =>
            println!("  SLIDES LONGER at high rate ({}→{}) ⇒ (B)-OPPOSITE to share-indexing (negative-p). Confirm across sweep.", l, h),
        (Some(l), Some(h)) =>
            println!("  SLIDES SHORTER at high rate ({}→{}) ⇒ (B)-SAME as share-indexing (p>0). Confirm across sweep, then p-pinning worth it.", l, h),
        _ => println!("  NONE admissible at reference — read sensitivity sweep (gate may still be strict at this k/δ)."),
    }

    // --- sensitivity, with champ-pass map (calibration control across the band) ---
    println!("\n## SENSITIVITY — τ_deploy_combined(spm6 → spm30) across k×δ_clock (★ = champion τ=360 FAILS stall gate ⇒ gate too strict there).");
    for &spm in &[6.0f32, 30.0f32] {
        println!("### spm{}", spm as u32);
        print!("| k \\ δ_clock |"); for &dc in DELTA_CLOCKS { print!(" {:.1} |", dc); } println!();
        print!("| --- |"); for _ in DELTA_CLOCKS { print!(" --- |"); } println!();
        let cells = cells_for(spm);
        for &k in KS {
            print!("| {:.1} |", k);
            for &dc in DELTA_CLOCKS {
                let td = tau_deploy_combined(cells, spm, k, dc);
                let champ = champ_pass(cells, spm, k, dc).unwrap();
                print!(" {}{} |", td.map(|t| t.to_string()).unwrap_or_else(|| "—".into()), if champ {""} else {"★"});
            }
            println!();
        }
        println!();
    }
    println!("READ: among champ-PASS (no ★) cells — τ_deploy(spm6)==τ_deploy(spm30) ⇒ (A) STOP; spm30>spm6 ⇒ (B)-opposite (sleepier");
    println!("at high rate, share-indexing backwards); spm30<spm6 ⇒ (B)-same (p>0, pursue p-pinning). If τ_deploy rails at 900 in");
    println!("champ-PASS cells (stall gate slack even at sleepiest τ) ⇒ wall 3 COLLAPSES into wall 4, with the CORRECT gate.");
}
