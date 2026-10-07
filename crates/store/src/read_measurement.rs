//! Opt-in thread-local timing for the large-prefix benchmark, never a product switch.
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};
#[derive(Clone, Default, Debug)]
pub struct Timings {
    pub sha: Duration,
    pub rows: u64,
    pub bytes: u64,
    /// In-memory record rows opened while the measurement is active.
    pub record_rows: u64,
}
thread_local! { static CURRENT: RefCell<Option<Timings>> = const { RefCell::new(None) }; }
pub fn start() {
    CURRENT.with(|v| *v.borrow_mut() = Some(Timings::default()));
}
pub fn finish() -> Timings {
    CURRENT.with(|v| v.borrow_mut().take().unwrap_or_default())
}
pub(crate) fn sha(start: Instant, bytes: usize) {
    CURRENT.with(|v| {
        if let Some(t) = v.borrow_mut().as_mut() {
            t.sha += start.elapsed();
            t.rows += 1;
            t.bytes += bytes as u64;
        }
    });
}

/// Counts row access at the shared stored-row boundary. Folded indexes do not
/// open rows; a query that scans a prefix must cross this boundary.
pub(crate) fn record_row() {
    CURRENT.with(|v| {
        if let Some(t) = v.borrow_mut().as_mut() {
            t.record_rows += 1;
        }
    });
}
