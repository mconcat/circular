
use circular_core::RecordedInstant;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn stepped(step_ms: u64) -> impl Fn() -> RecordedInstant + Send + Sync {
    let next = AtomicU64::new(0);
    move || {
        let millis = next.fetch_add(step_ms, Ordering::Relaxed) + step_ms;
        RecordedInstant::from_millis(millis)
    }
}
