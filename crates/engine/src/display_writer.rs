
use circular_core::{
    BuiltinObservationName, EncodedPayload, NonZeroMillis, PayloadVersionTag, RecordedInstant,
    Stamp,
};
use circular_plan::{ActorId, NamedActorId};
use circular_store::{DisplayRecord, Record, RecordOrigin, StoreSchema};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DisplayCoordinate {
    name: BuiltinObservationName,
    subject: NamedActorId,
    bucket: NonZeroMillis,
    window: u64,
}

type DisplayAxis = (BuiltinObservationName, NamedActorId, NonZeroMillis);

fn display_axis(coordinate: &DisplayCoordinate) -> DisplayAxis {
    (
        coordinate.name(),
        coordinate.subject().clone(),
        coordinate.bucket(),
    )
}

/// Consecutive display-window state owned by the provider that appends display records.
///
/// A standing run settles many times, and each settle carries only the arrivals since the
/// previous one, so folding a window inside one settle silently restarts its tally at every
/// settle.  This accumulator lives beside the
/// provider's [`DisplayCatalog`] and carries the current window across those completion
/// boundaries.
///
/// The key is the full display coordinate `(observation, subject, bucket, window)`.  Advancing to
/// a new window retires the previous window for the same `(observation, subject, bucket)` axis.
/// A defensive capacity also bounds state when authored actors or registered buckets churn during
/// a long-lived run.
#[derive(Debug)]
pub struct DisplayWindowAccumulator {
    active: BTreeMap<DisplayCoordinate, DisplayWindowState>,
    touch: u64,
    capacity: usize,
}

#[derive(Clone, Copy, Debug)]
struct DisplayWindowState {
    count: i64,
    touched_at: u64,
}

impl Default for DisplayWindowAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayWindowAccumulator {
    /// Enough active axes for a large product graph while remaining an absolute memory bound.
    pub const DEFAULT_CAPACITY: usize = 4_096;

    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Construct a bounded accumulator.  A zero request still retains one active coordinate.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            active: BTreeMap::new(),
            touch: 0,
            capacity: capacity.max(1),
        }
    }

    /// Add `amount` to a coordinate and return the accumulated value for its current window.
    pub fn advance(&mut self, coordinate: &DisplayCoordinate, amount: i64) -> i64 {
        self.touch = self.touch.saturating_add(1);
        let touched_at = self.touch;

        let carried = self
            .active
            .iter()
            .find(|(existing, _)| {
                existing.name() == coordinate.name()
                    && existing.subject() == coordinate.subject()
                    && existing.bucket() == coordinate.bucket()
            })
            .map_or(0, |(_, state)| state.count);
        self.active.retain(|existing, _| {
            existing == coordinate
                || existing.name() != coordinate.name()
                || existing.subject() != coordinate.subject()
                || existing.bucket() != coordinate.bucket()
        });

        if !self.active.contains_key(coordinate) && self.active.len() >= self.capacity {
            let oldest = self
                .active
                .iter()
                .min_by_key(|(key, state)| (state.touched_at, (*key).clone()))
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.active.remove(&oldest);
            }
        }

        let state = self
            .active
            .entry(coordinate.clone())
            .or_insert(DisplayWindowState {
                count: carried,
                touched_at,
            });
        state.count = state.count.saturating_add(amount);
        state.touched_at = touched_at;
        state.count
    }

    /// Number of currently retained coordinate windows (diagnostics and bound tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.active.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

impl DisplayCoordinate {
    #[must_use]
    pub fn at(
        name: BuiltinObservationName,
        subject: NamedActorId,
        bucket: NonZeroMillis,
        observed_at: RecordedInstant,
    ) -> Self {
        let width = bucket.get().get();
        Self {
            name,
            subject,
            bucket,
            window: observed_at.millis() / width,
        }
    }

    #[must_use]
    pub const fn window(&self) -> u64 {
        self.window
    }

    #[must_use]
    pub const fn name(&self) -> BuiltinObservationName {
        self.name
    }

    #[must_use]
    pub const fn bucket(&self) -> NonZeroMillis {
        self.bucket
    }

    #[must_use]
    pub const fn subject(&self) -> &NamedActorId {
        &self.subject
    }
}

pub struct DisplayCatalog<K> {
    minted: BTreeMap<DisplayAxis, K>,
    mint: Box<dyn FnMut(&DisplayCoordinate) -> K + Send>,
}

impl<K> core::fmt::Debug for DisplayCatalog<K> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DisplayCatalog")
            .field("minted", &self.minted.len())
            .finish_non_exhaustive()
    }
}

impl<K: Clone> DisplayCatalog<K> {
    pub fn deriving(mint: impl FnMut(&DisplayCoordinate) -> K + Send + 'static) -> Self {
        Self {
            minted: BTreeMap::new(),
            mint: Box::new(mint),
        }
    }

    pub fn key_for(&mut self, coordinate: &DisplayCoordinate) -> K {
        let axis = display_axis(coordinate);
        if let Some(existing) = self.minted.get(&axis) {
            return existing.clone();
        }
        let issued = (self.mint)(coordinate);
        self.minted.insert(axis, issued.clone());
        issued
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.minted.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.minted.is_empty()
    }
}

pub fn project_display<S, V>(
    at: Stamp<ActorId>,
    key: S::DisplayKey,
    value: &V,
    encode: impl FnOnce(&V) -> Vec<u8>,
) -> Record<S>
where
    S: StoreSchema<Producer = ActorId, DisplayPayload = EncodedPayload>,
{
    Record::Display(DisplayRecord::new(
        at,
        RecordOrigin::Stream,
        key,
        EncodedPayload::new(PayloadVersionTag::FIRST, &encode(value)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::{Name, ScopeId};

    fn coordinate(subject: &str, bucket_ms: u64, observed_at_ms: u64) -> DisplayCoordinate {
        DisplayCoordinate::at(
            BuiltinObservationName::Tally,
            NamedActorId::new(ScopeId::root(), Name::from_normalized(subject)),
            NonZeroMillis::new(bucket_ms).expect("positive bucket"),
            RecordedInstant::from_millis(observed_at_ms),
        )
    }

    #[test]
    fn accumulator_carries_a_run_tally_across_provider_settles_and_windows() {
        let mut accumulator = DisplayWindowAccumulator::new();
        let first_window = coordinate("counter", 1_000, 100);
        assert_eq!(accumulator.advance(&first_window, 1), 1);
        assert_eq!(accumulator.advance(&first_window, 1), 2);

        let next_window = coordinate("counter", 1_000, 1_100);
        assert_eq!(accumulator.advance(&next_window, 1), 3);
        assert_eq!(accumulator.len(), 1, "the retired window is not retained");
    }

    #[test]
    fn accumulator_keeps_actors_and_registered_buckets_independent() {
        let mut accumulator = DisplayWindowAccumulator::new();
        let left = coordinate("left", 1_000, 100);
        let right = coordinate("right", 1_000, 100);
        let dense = coordinate("left", 250, 100);

        assert_eq!(accumulator.advance(&left, 1), 1);
        assert_eq!(accumulator.advance(&right, 1), 1);
        assert_eq!(accumulator.advance(&dense, 1), 1);
        assert_eq!(accumulator.advance(&left, 1), 2);
        assert_eq!(accumulator.advance(&right, 1), 2);
        assert_eq!(accumulator.advance(&dense, 1), 2);
        assert_eq!(accumulator.len(), 3);
    }

    #[test]
    fn accumulator_evicts_the_least_recent_axis_at_its_bound() {
        let mut accumulator = DisplayWindowAccumulator::with_capacity(2);
        let first = coordinate("first", 1_000, 100);
        let second = coordinate("second", 1_000, 100);
        let third = coordinate("third", 1_000, 100);

        assert_eq!(accumulator.advance(&first, 1), 1);
        assert_eq!(accumulator.advance(&second, 1), 1);
        assert_eq!(accumulator.advance(&first, 1), 2, "first becomes recent");
        assert_eq!(accumulator.advance(&third, 1), 1, "second is evicted");
        assert_eq!(accumulator.len(), 2);
        assert_eq!(accumulator.advance(&second, 1), 1, "evicted axes restart");
        assert_eq!(accumulator.len(), 2);
    }

    #[test]
    fn catalog_reuses_one_key_across_windows_of_the_same_axis() {
        let mut catalog = DisplayCatalog::deriving(DisplayCoordinate::window);
        let first_window = coordinate("counter", 1_000, 100);
        let next_window = coordinate("counter", 1_000, 1_100);
        assert_ne!(first_window.window(), next_window.window());

        assert_eq!(catalog.key_for(&first_window), 0);
        assert_eq!(catalog.key_for(&next_window), 0);
        assert_eq!(catalog.len(), 1);
    }
}
