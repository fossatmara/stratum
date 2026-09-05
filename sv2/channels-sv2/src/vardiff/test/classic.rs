/// Classic implementation test suite
use crate::vardiff::test::{
    simulate_shares_and_wait, TEST_INITIAL_HASHRATE, TEST_MIN_ALLOWED_HASHRATE,
    TEST_SHARES_PER_MINUTE,
};
use crate::{target::hash_rate_to_target, vardiff::VardiffError, VardiffState};

use super::{
    test_backwards_clock_step_reanchors_window, test_increment_and_reset_shares,
    test_try_vardiff_low_hashrate_decrease_target,
    test_try_vardiff_stable_hashrate_minimal_change_or_no_change,
    test_try_vardiff_with_less_spm_than_expected, test_try_vardiff_with_shares_30_to_60s,
    test_try_vardiff_with_shares_less_than_30, test_try_vardiff_with_shares_more_than_60s, Vardiff,
};

fn new_test_vardiff_state() -> Result<VardiffState, VardiffError> {
    VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
}

#[test]
fn test_initialization_and_getters() {
    let vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");

    assert_eq!(vardiff.min_allowed_hashrate(), TEST_MIN_ALLOWED_HASHRATE);
    assert_eq!(vardiff.shares_since_last_update(), 0);
}

#[test]
fn test_increment_and_reset_shares_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_increment_and_reset_shares(&mut vardiff)
}

#[test]
fn test_backwards_clock_step_reanchors_window_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_backwards_clock_step_reanchors_window(&mut vardiff);
}

#[test]
fn test_try_vardiff_stable_hashrate_minimal_change_or_no_change_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_stable_hashrate_minimal_change_or_no_change(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_low_hashrate_decrease_target_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_low_hashrate_decrease_target(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_with_shares_less_than_30_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_shares_less_than_30(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_with_shares_30_to_60s_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_shares_30_to_60s(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_with_shares_more_than_60s_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_shares_more_than_60s(&mut vardiff);
}

#[test]
fn test_try_vardiff_with_less_spm_than_expected_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_less_spm_than_expected(&mut vardiff);
}

#[test]
fn test_try_vardiff_hashrate_clamps_to_minimum() {
    let hashrate = TEST_MIN_ALLOWED_HASHRATE * 1.5;
    let target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
        .unwrap()
        .into();

    let mut vardiff = VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
        .expect("Failed to create VardiffState");

    // Long enough that a zero estimate is worth acting on. A 16-second window was enough under
    // the fixed ladder, whose top rung fired on any 100% deviation regardless of window length;
    // the threshold now asks for roughly 570% at that length, and rightly refuses.
    let simulation_duration_secs = 300;
    simulate_shares_and_wait(&mut vardiff, 0, simulation_duration_secs);

    let result = vardiff
        .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
        .expect("try_vardiff failed");
    assert!(result.is_some(), "Hashrate should update");
    let new_hashrate = result.unwrap();

    assert_eq!(
        new_hashrate, TEST_MIN_ALLOWED_HASHRATE,
        "Hashrate should be clamped to minimum"
    );
    assert_eq!(
        new_hashrate, TEST_MIN_ALLOWED_HASHRATE,
        "Stored hashrate should be clamped"
    );
    assert_eq!(vardiff.shares_since_last_update(), 0);
}

// Every unusable prior hashrate must fall back to the configured floor. `hashrate` is
// the divisor for the delta percentage and the base every special-case cap scales, so
// without the fallback: `0.0` pins to the floor forever (`0.0 * 10.0` is absorbing),
// a value below the floor pins to it for the round, `NaN` makes every `should_update`
// arm compare false, and `inf` makes the percentage `inf / inf == NaN` with the same
// effect. A miner picks this value via `nominal_hash_rate` at channel open.
#[test]
fn test_try_vardiff_unusable_prior_hashrate_falls_back_to_floor() {
    for bad in [0.0_f32, 0.01, -5.0, f32::NAN, f32::INFINITY] {
        let mut vardiff = VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
            .expect("Failed to create VardiffState");

        let target = hash_rate_to_target(1000.0_f64, TEST_SHARES_PER_MINUTE.into()).unwrap();

        // 1000 shares in 16s => 3750 shares/min, far above TEST_SHARES_PER_MINUTE.
        simulate_shares_and_wait(&mut vardiff, 1000, 16);

        match vardiff.try_vardiff(bad, &target, TEST_SHARES_PER_MINUTE) {
            Ok(Some(v)) => assert!(
                v.is_finite() && v > TEST_MIN_ALLOWED_HASHRATE * 1.5,
                "prior hashrate {bad} pinned difficulty to the floor: {v}"
            ),
            other => panic!("prior hashrate {bad} silently disabled vardiff: {other:?}"),
        }
    }
}

// `min_allowed_hashrate` is a `pub` field, so the validation in `new_with_min` can be
// bypassed by direct assignment; `try_vardiff` must sanitize the floor itself before
// relying on it as the fallback baseline and the output clamp.
#[test]
fn test_try_vardiff_sanitizes_unusable_floor() {
    for bad_floor in [0.0_f32, -5.0, f32::NAN, f32::INFINITY] {
        let mut vardiff = VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
            .expect("Failed to create VardiffState");
        vardiff.min_allowed_hashrate = bad_floor;

        let target = hash_rate_to_target(1000.0_f64, TEST_SHARES_PER_MINUTE.into()).unwrap();

        // 1000 shares in 16s => 3750 shares/min, far above TEST_SHARES_PER_MINUTE.
        simulate_shares_and_wait(&mut vardiff, 1000, 16);

        // prior hashrate 0.0 forces the fallback onto the (unusable) floor
        match vardiff.try_vardiff(0.0, &target, TEST_SHARES_PER_MINUTE) {
            Ok(Some(v)) => assert!(
                v.is_finite() && v > 0.0,
                "floor {bad_floor} produced unusable hashrate {v}"
            ),
            other => panic!("floor {bad_floor} silently disabled vardiff: {other:?}"),
        }
    }
}

// The floor itself is the fallback baseline used by `try_vardiff`, so a non-positive or
// non-finite `min_allowed_hashrate` would reintroduce the division by zero.
#[test]
fn test_new_with_min_rejects_unusable_floor() {
    for bad in [0.0_f32, -5.0, f32::NAN, f32::INFINITY] {
        let vardiff = VardiffState::new_with_min(bad).expect("Failed to create VardiffState");
        let min = vardiff.min_allowed_hashrate();
        assert!(
            min.is_finite() && min > 0.0,
            "min_allowed_hashrate {min} from input {bad} is unusable as a baseline"
        );
    }
}

// A fully-silent channel must not be eased all the way to the floor. Unbounded, each silent
// evaluation eases again with nothing limiting how many times it runs, so displacement compounds
// without limit. A returning miner is then served a difficulty that much too easy and floods shares
// until vardiff climbs back.
#[test]
fn test_silent_channel_ease_displacement_is_bounded() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    let mut hashrate = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    // Forty consecutive silent evaluations. Unbounded, the ~4 that carry
    // TEST_INITIAL_HASHRATE down to TEST_MIN_ALLOWED_HASHRATE are spent in the first few.
    for _ in 0..40 {
        simulate_shares_and_wait(&mut vardiff, 0, 61);
        if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
            hashrate = updated;
        }
    }

    // Displacement must settle at the `9×` that MAX_SILENT_DISPLACEMENT sets. Written as a literal
    // rather than derived from that constant deliberately: a derived expectation moves with the
    // constant and would assert nothing, and this way the test also compiles and fails against a
    // tree without the bound at all.
    let expected = TEST_INITIAL_HASHRATE / 9.0;
    assert!(
        (hashrate / expected - 1.0).abs() < 1e-3,
        "silent channel settled at {hashrate} H/s, not the {expected} H/s that a 9× bound \
         allows (floor is {}) — a returning miner would be served a difficulty that much too easy",
        vardiff.min_allowed_hashrate()
    );
}

// The bound is on *consecutive* silence, so any share activity must restore the full budget.
// Otherwise a long-lived channel that goes briefly quiet many times would eventually exhaust it
// and stop easing for a genuine decline.
#[test]
fn test_share_activity_rearms_the_silence_bound() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    let mut hashrate = TEST_INITIAL_HASHRATE;
    let mut target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    // 1. Sustained silence settles at the bound: 9x below the belief that last had evidence.
    for _ in 0..40 {
        simulate_shares_and_wait(&mut vardiff, 0, 61);
        if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
            hashrate = updated;
            target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
        }
    }
    let first_bound = TEST_INITIAL_HASHRATE / 9.0;
    assert!(
        (hashrate / first_bound - 1.0).abs() < 1e-3,
        "silence should settle at {first_bound} H/s, got {hashrate}"
    );

    // 2. Evidence arrives at the lowered belief, re-anchoring the bound to it.
    simulate_shares_and_wait(&mut vardiff, TEST_SHARES_PER_MINUTE as u32, 61);
    if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
        hashrate = updated;
        target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
            .expect("valid target");
    }
    let rearmed_from = hashrate;

    // 3. Silence may now descend past the previous bound, because the bound is relative to the
    //    last evidenced belief rather than an absolute floor. Without the re-anchor the channel
    //    would be stuck at `first_bound` forever, unable to follow a miner that really had slowed.
    for _ in 0..40 {
        simulate_shares_and_wait(&mut vardiff, 0, 61);
        if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
            hashrate = updated;
            target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
        }
    }
    assert!(
        hashrate < first_bound,
        "activity did not re-arm the bound: still at {hashrate} H/s, no lower than the earlier \
         bound of {first_bound} H/s despite evidence at {rearmed_from} H/s"
    );
    assert!(
        hashrate >= rearmed_from / 9.0 * (1.0 - 1e-3),
        "descent went past the re-armed bound: {hashrate} H/s is below {} H/s",
        rearmed_from / 9.0
    );
}

/// The estimator must forget on the clock, by `e^(−Δt/tau)`, which is the property the cumulative
/// mean lacked and the reason it entrenched.
///
/// Asserted at the mechanism rather than end-to-end because the shared harness cannot express a
/// growing evaluation window: `simulate_shares_and_wait` re-anchors the window on every call, so a
/// sequence of non-retargeting evaluations — the situation in which the old estimator entrenches —
/// is not reachable through it. An end-to-end entrenchment test needs a harness able to advance the
/// evaluation clock independently of the retarget clock.
///
/// The intervals below are chosen to stay under the decision boundary, so no retarget intervenes.
/// They are shorter than they once were because the boundary is now sized from the evidence: at
/// this share rate a 180-second gap clears it, where the old fixed ladder would not have fired.
/// That matters: on a retarget `rescale_ewma` re-expresses the rate against the new difficulty, so
/// the stored rate returns to roughly its pre-decay value — correct, but it hides the decay from
/// an observer reading the rate afterwards.
#[test]
fn test_estimator_forgets_on_the_clock() {
    let hashrate = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    let seed = |vardiff: &mut VardiffState| {
        simulate_shares_and_wait(vardiff, TEST_SHARES_PER_MINUTE as u32, 60);
        let fired = vardiff
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        assert!(fired.is_none(), "on-target seeding should not retarget");
    };

    // Seeding takes the first observation whole rather than blending it against a zero prior.
    let mut single = new_test_vardiff_state().expect("Failed to create VardiffState");
    seed(&mut single);
    let seeded = single.ewma_rate();
    assert!(
        (seeded - TEST_SHARES_PER_MINUTE as f64).abs() < 1e-2,
        "seeding should take the first observation whole, got {seeded}"
    );

    // A quarter of a time constant of silence discards `1 − e^(−1/4)` of the stored rate.
    simulate_shares_and_wait(&mut single, 0, 90);
    let fired = single
        .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
        .expect("try_vardiff failed");
    assert!(fired.is_none(), "a 90s gap should stay under the boundary");
    let expected = seeded * (-0.25f64).exp();
    let decayed = single.ewma_rate();
    assert!(
        (decayed / expected - 1.0).abs() < 1e-3,
        "after a quarter of a tau the rate should be {expected}, got {decayed}"
    );

    // Decay tracks elapsed time, not the number of evaluations: two eighth-tau gaps must land
    // where one quarter-tau gap did. A per-evaluation constant would forget twice as much here, which
    // is how an irregular cadence silently changes a filter's memory.
    let mut split = new_test_vardiff_state().expect("Failed to create VardiffState");
    seed(&mut split);
    for _ in 0..2 {
        simulate_shares_and_wait(&mut split, 0, 45);
        let fired = split
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        assert!(fired.is_none(), "a 45s gap should stay under the boundary");
    }
    assert!(
        (split.ewma_rate() / decayed - 1.0).abs() < 1e-3,
        "two eighth-tau gaps ({}) should decay the same as one quarter-tau ({decayed})",
        split.ewma_rate()
    );
}

/// A decline is detected in the same few evaluations whatever the channel's age.
///
/// The same scenario across the series: an hour of on-target delivery over sixty non-retargeting
/// evaluations, then the miner drops to a tenth of its rate. The fixed ladder over a cumulative
/// window took **thirteen** evaluations to react. The clock-decayed average took **two**, because
/// evidence ages whether or not the controller fires. Sizing the threshold from the evidence takes
/// it to **one**: after an hour the window holds enough observation that a tenfold shortfall clears
/// the bar immediately, where the ladder still demanded a fixed 15%.
///
/// Both figures are asserted exactly rather than as bounds. Reaction time is the quantity these
/// patches trade against each other, so a change to it should appear as a change to this line.
///
/// Needs a controllable clock: `simulate_shares_and_wait` re-anchors the window on every call, so
/// it can express "the window is an hour long" but not "an hour arrived as sixty separate
/// evaluations" — and the second is what grows the window.
///
/// [`MockClock`]: crate::vardiff::clock::MockClock
#[test]
fn test_decline_detected_regardless_of_window_age() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    let clock = Arc::new(MockClock::new(1_000_000));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("Failed to create VardiffState");
    let hashrate = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    // An hour of delivery exactly on target. Nothing retargets, so the window reaches an hour.
    for evaluation in 1..=60 {
        for _ in 0..TEST_SHARES_PER_MINUTE as u32 {
            vardiff.increment_shares_since_last_update();
        }
        clock.advance(60);
        let outcome = vardiff
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        assert!(
            outcome.is_none(),
            "on-target delivery should not retarget (evaluation {evaluation})"
        );
    }

    // The miner collapses to a tenth of its rate and stays there.
    let mut evaluations_to_react = 0;
    loop {
        evaluations_to_react += 1;
        vardiff.increment_shares_since_last_update();
        clock.advance(60);
        let outcome = vardiff
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        if outcome.is_some() || evaluations_to_react > 60 {
            break;
        }
    }

    assert!(
        evaluations_to_react <= 2,
        "an hour-old window must not delay detection; took {evaluations_to_react} evaluations \
         (the fixed ladder over a cumulative window took 13)"
    );
    assert_eq!(
        evaluations_to_react, 1,
        "reaction time is 1 evaluation; a change here is a change in reaction time"
    );
}
