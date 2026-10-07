use crate::config::{NonZeroIntervalList, SlotKind};
use circular_core::Value;

pub const RETRY_FIELD: &str = "retry_delays";

pub const CREATOR_DEFAULT_MS: [u64; 9] = [
    1_000, 1_000, 1_000, 5_000, 5_000, 5_000, 15_000, 15_000, 15_000,
];

pub fn schedule(value: &Value) -> Result<Box<[u64]>, &'static str> {
    NonZeroIntervalList
        .decode(value)
        .map(|waits| waits.iter().map(|wait| wait.non_zero().get()).collect())
        .map_err(|_| "retry_delays must be an array of whole milliseconds, each at least 1")
}

pub fn declared(config: &Value) -> Result<Option<Box<[u64]>>, &'static str> {
    config
        .as_object()
        .and_then(|root| root.get(RETRY_FIELD))
        .map(schedule)
        .transpose()
}

