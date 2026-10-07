use circular_runtime::{Effect, EffectFailure, OutcomePayload};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrySchedule(Arc<[u64]>);

impl RetrySchedule {
    #[must_use]
    pub fn creator_default() -> Self {
        Self(Arc::from(circular_actors::retry_config::CREATOR_DEFAULT_MS))
    }

    #[must_use]
    pub fn none() -> Self {
        Self(Arc::from([]))
    }

    #[must_use]
    pub fn from_checked_ms(waits: impl Into<Arc<[u64]>>) -> Self {
        Self(waits.into())
    }

    #[must_use]
    pub fn ms(&self) -> &[u64] {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn wait_before(&self, retry: u32) -> Option<u64> {
        let index = usize::try_from(retry.checked_sub(1)?).ok()?;
        self.0.get(index).copied()
    }
}

#[must_use]
pub(crate) fn may_retry(effect: &Effect) -> bool {
    matches!(effect, Effect::Http { .. } | Effect::Notify { .. })
}

#[must_use]
pub(crate) fn transient(
    effect: &Effect,
    result: &Result<OutcomePayload, EffectFailure>,
) -> Option<Option<u64>> {
    if !may_retry(effect) {
        return None;
    }
    match result {
        Ok(OutcomePayload::HttpResponse(response)) if matches!(response.status(), 429 | 503) => {
            Some(response.retry_after_seconds())
        }
        Err(EffectFailure::TransportUnreached | EffectFailure::RemoteDeferred) => Some(None),
        _ => None,
    }
}

#[derive(Debug)]
pub struct ReceiverBackoff {
    schedule: RetrySchedule,
    refused_in_a_row: std::sync::atomic::AtomicUsize,
}

impl Default for ReceiverBackoff {
    fn default() -> Self {
        Self::new(RetrySchedule::creator_default())
    }
}

impl ReceiverBackoff {
    #[must_use]
    pub fn new(schedule: RetrySchedule) -> Self {
        Self {
            schedule,
            refused_in_a_row: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn refused(&self) -> u64 {
        let order = self
            .refused_in_a_row
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let waits = self.schedule.ms();
        match waits.len() {
            0 => 1,
            len => waits[order.min(len - 1)].div_ceil(1000).max(1),
        }
    }

    pub fn accepted(&self) {
        self.refused_in_a_row
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

