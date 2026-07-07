//! Read-only controller telemetry, for visualization.
//!
//! Controllers publish their current gains keyed by the channel\'s user
//! identity; a simulator embedding the pool reads them to plot when the
//! gain-scheduling (e.g. Q-learning) changes kp/ki/kd. Process-global like
//! [`super::sim_clock`]; production pools bear only a HashMap insert per
//! gain change.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock, Mutex,
    },
};

static GAINS: LazyLock<Mutex<HashMap<String, (u64, f64, f64, f64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static IDS: AtomicU64 = AtomicU64::new(1);

/// Unique id per controller instance, so a replaced controller (miner
/// reconnect creates a new channel before the old one drops) cannot clear
/// its successor\'s entry.
pub fn next_instance_id() -> u64 {
    IDS.fetch_add(1, Ordering::Relaxed)
}

/// Publishes the current gains for a user identity.
pub fn publish_gains(key: &str, instance: u64, kp: f64, ki: f64, kd: f64) {
    GAINS
        .lock()
        .expect("gain telemetry lock")
        .insert(key.to_string(), (instance, kp, ki, kd));
}

/// Current gains for a user identity, if a controller has published them.
pub fn gains(key: &str) -> Option<(f64, f64, f64)> {
    GAINS
        .lock()
        .expect("gain telemetry lock")
        .get(key)
        .map(|&(_, kp, ki, kd)| (kp, ki, kd))
}

/// Clears an identity\'s entry, but only if it still belongs to `instance`.
pub fn clear_gains(key: &str, instance: u64) {
    let mut gains = GAINS.lock().expect("gain telemetry lock");
    if gains.get(key).map(|&(i, ..)| i) == Some(instance) {
        gains.remove(key);
    }
}
