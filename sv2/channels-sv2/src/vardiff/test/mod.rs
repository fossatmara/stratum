//! Contains a generic test implementation that is agnostic to the Vardiff implementation,
//! providing methods to verify the correctness of any specific implementation.

mod classic;

use super::Vardiff;
use crate::target::hash_rate_to_target;

pub const TEST_INITIAL_HASHRATE: f32 = 1000.0;
pub const TEST_SHARES_PER_MINUTE: f32 = 10.0;
pub const TEST_MIN_ALLOWED_HASHRATE: f32 = 10.0;

// Helper function to simulate a number of shares being found over a given duration.
pub fn simulate_shares_and_wait<V: Vardiff>(
    vardiff: &mut V,
    num_shares: u32,
    wait_duration_secs: u64,
) {
    for _ in 0..num_shares {
        vardiff.increment_shares_since_last_update();
    }

    // Rather than waiting for wait_duration,
    // we are performing time magic and going
    // back in time.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - wait_duration_secs;

    vardiff.set_timestamp_of_last_update(now);
}

// A backwards clock step (e.g. an NTP correction) leaves the recorded timestamp in the
// future. `try_vardiff` must decline to adjust rather than panic on the elapsed-time
// subtraction, and it must re-anchor the window: the `delta_time <= 15` early return runs
// before `reset_counter()`, so leaving the future timestamp in place would stall vardiff
// for the entire length of the clock step, not just one round.
pub fn test_backwards_clock_step_reanchors_window<V: Vardiff>(vardiff: &mut V) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Pretend the last update happened an hour in the future.
    vardiff.set_timestamp_of_last_update(now + 3600);

    let target =
        hash_rate_to_target(TEST_INITIAL_HASHRATE.into(), TEST_SHARES_PER_MINUTE.into()).unwrap();

    assert!(matches!(
        vardiff.try_vardiff(TEST_INITIAL_HASHRATE, &target, TEST_SHARES_PER_MINUTE),
        Ok(None)
    ));

    assert!(
        vardiff.last_update_timestamp() <= now + 1,
        "window not re-anchored (timestamp still {}, now {}): vardiff would stall for the \
         full length of the clock step",
        vardiff.last_update_timestamp(),
        now
    );
}
