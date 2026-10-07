
use std::cmp::max;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{ProducerIdentity, Stamp};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Millis(u64);

impl Millis {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(millis: u64) -> Self {
        Self(millis)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Millis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}ms", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NonZeroMillis(std::num::NonZeroU64);

impl NonZeroMillis {
    pub fn new(millis: u64) -> Result<Self, ZeroMillisError> {
        std::num::NonZeroU64::new(millis)
            .map(Self)
            .ok_or(ZeroMillisError)
    }

    #[must_use]
    pub const fn get(self) -> Millis {
        Millis(self.0.get())
    }

    #[must_use]
    pub const fn non_zero(self) -> std::num::NonZeroU64 {
        self.0
    }
}

impl fmt::Display for NonZeroMillis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZeroMillisError;

impl fmt::Display for ZeroMillisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("0 is not a time interval")
    }
}

impl std::error::Error for ZeroMillisError {}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RecordedInstant(u64);

impl RecordedInstant {
    pub const ORIGIN: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    #[must_use]
    pub const fn millis(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn interval_since(self, earlier: Self) -> Option<Millis> {
        match self.0.checked_sub(earlier.0) {
            Some(elapsed) => Some(Millis(elapsed)),
            None => None,
        }
    }

    #[must_use]
    pub const fn checked_add(self, interval: Millis) -> Option<Self> {
        match self.0.checked_add(interval.0) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }

    #[must_use]
    pub const fn saturating_add(self, interval: Millis) -> Self {
        Self(self.0.saturating_add(interval.0))
    }
}

impl fmt::Display for RecordedInstant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "+{}ms", self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Tick(u64);

impl Tick {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn checked_add(self, ticks: Ticks) -> Option<Self> {
        match self.0.checked_add(ticks.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    pub const fn duration_since(self, earlier: Self) -> Option<Ticks> {
        match self.0.checked_sub(earlier.0) {
            Some(value) => Some(Ticks(value)),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Ticks(u64);

impl Ticks {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NonZeroTicks(Ticks);

impl NonZeroTicks {
    pub const fn new(value: u64) -> Result<Self, NonZeroTicksError> {
        if value == 0 {
            Err(NonZeroTicksError)
        } else {
            Ok(Self(Ticks(value)))
        }
    }

    #[must_use]
    pub const fn get(self) -> Ticks {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NonZeroTicksError;

impl fmt::Display for NonZeroTicksError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the tick interval must be at least 1")
    }
}

impl std::error::Error for NonZeroTicksError {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TicksPerSecond(u32);

impl TicksPerSecond {
    pub const MIN: u32 = 1_000;

    pub const fn new(value: u32) -> Result<Self, TicksPerSecondError> {
        if value < Self::MIN {
            Err(TicksPerSecondError { attempted: value })
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TicksPerSecondError {
    attempted: u32,
}

impl TicksPerSecondError {
    #[must_use]
    pub const fn attempted(self) -> u32 {
        self.attempted
    }
}

impl fmt::Display for TicksPerSecondError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "ticks_per_second must be at least {} (input: {})",
            TicksPerSecond::MIN,
            self.attempted
        )
    }
}

impl std::error::Error for TicksPerSecondError {}

pub trait TickSource: Send + Sync {
    fn current_tick(&self) -> Tick;
    fn wait_until(&self, tick: Tick) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    fn heartbeat_cadence(&self) -> Ticks;
    fn ticks_per_second(&self) -> u32;
    fn is_replay(&self) -> bool {
        false
    }
}

async fn await_advance(advanced: &tokio::sync::Notify, tick: Tick, current: impl Fn() -> Tick) {
    loop {
        let notified = advanced.notified();
        let mut notified = std::pin::pin!(notified);
        notified.as_mut().enable();
        if current() >= tick {
            return;
        }
        notified.await;
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TimeSourceKind {
    WallClockAnchored,
    LogDriven,
    Manual,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimeSourcePlan<P: ProducerIdentity> {
    Single(TimeSourceKind),
    Switched {
        head: TimeSourceKind,
        at: Stamp<P>,
        tail: TimeSourceKind,
    },
}

#[derive(Debug)]
struct ManualInner {
    current: Mutex<Tick>,
    advanced: tokio::sync::Notify,
}

#[derive(Debug)]
struct LogDrivenInner {
    cursor: Mutex<Tick>,
    advanced: tokio::sync::Notify,
}

#[derive(Clone, Debug)]
pub struct LogDrivenTimeSource {
    inner: Arc<LogDrivenInner>,
    terminal_tick: Tick,
    cadence: NonZeroTicks,
    resolution: TicksPerSecond,
}

impl LogDrivenTimeSource {
    #[must_use]
    pub fn new(terminal_tick: Tick, cadence: NonZeroTicks, resolution: TicksPerSecond) -> Self {
        Self {
            inner: Arc::new(LogDrivenInner {
                cursor: Mutex::new(Tick::ZERO),
                advanced: tokio::sync::Notify::new(),
            }),
            terminal_tick,
            cadence,
            resolution,
        }
    }

    #[must_use]
    pub const fn terminal_tick(&self) -> Tick {
        self.terminal_tick
    }

    pub fn advance_to(&self, tick: Tick) -> Result<(), LogDrivenTimeError> {
        {
            let mut cursor = recover_lock(self.inner.cursor.lock());
            if tick < *cursor {
                return Err(LogDrivenTimeError::WouldRegress {
                    current: *cursor,
                    attempted: tick,
                });
            }
            if tick > self.terminal_tick {
                return Err(LogDrivenTimeError::BeyondTerminal {
                    terminal: self.terminal_tick,
                    attempted: tick,
                });
            }
            *cursor = tick;
        }
        self.inner.advanced.notify_waiters();
        Ok(())
    }
}

impl TickSource for LogDrivenTimeSource {
    fn current_tick(&self) -> Tick {
        *recover_lock(self.inner.cursor.lock())
    }

    fn wait_until(&self, tick: Tick) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(await_advance(&self.inner.advanced, tick, || {
            TickSource::current_tick(self)
        }))
    }

    fn heartbeat_cadence(&self) -> Ticks {
        self.cadence.get()
    }

    fn ticks_per_second(&self) -> u32 {
        self.resolution.get()
    }

    fn is_replay(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogDrivenTimeError {
    WouldRegress { current: Tick, attempted: Tick },
    BeyondTerminal { terminal: Tick, attempted: Tick },
}

impl fmt::Display for LogDrivenTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldRegress { current, attempted } => write!(
                formatter,
                "the log cursor cannot decrease (current: {}, input: {})",
                current.get(),
                attempted.get()
            ),
            Self::BeyondTerminal {
                terminal,
                attempted,
            } => write!(
                formatter,
                "the log cursor cannot pass the horizon (horizon: {}, input: {})",
                terminal.get(),
                attempted.get()
            ),
        }
    }
}

impl std::error::Error for LogDrivenTimeError {}

#[derive(Clone, Debug)]
pub struct ManualTimeSource {
    inner: Arc<ManualInner>,
    cadence: NonZeroTicks,
    resolution: TicksPerSecond,
}

impl ManualTimeSource {
    #[must_use]
    pub fn new(initial: Tick, cadence: NonZeroTicks, resolution: TicksPerSecond) -> Self {
        Self {
            inner: Arc::new(ManualInner {
                current: Mutex::new(initial),
                advanced: tokio::sync::Notify::new(),
            }),
            cadence,
            resolution,
        }
    }

    pub fn advance_to(&self, tick: Tick) -> Result<(), ManualTimeError> {
        {
            let mut current = recover_lock(self.inner.current.lock());
            if tick < *current {
                return Err(ManualTimeError::WouldRegress {
                    current: *current,
                    attempted: tick,
                });
            }
            *current = tick;
        }
        self.inner.advanced.notify_waiters();
        Ok(())
    }
}

impl TickSource for ManualTimeSource {
    fn current_tick(&self) -> Tick {
        *recover_lock(self.inner.current.lock())
    }

    fn wait_until(&self, tick: Tick) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(await_advance(&self.inner.advanced, tick, || {
            TickSource::current_tick(self)
        }))
    }

    fn heartbeat_cadence(&self) -> Ticks {
        self.cadence.get()
    }

    fn ticks_per_second(&self) -> u32 {
        self.resolution.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManualTimeError {
    WouldRegress { current: Tick, attempted: Tick },
}

impl fmt::Display for ManualTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldRegress { current, attempted } => write!(
                formatter,
                "manual time cannot go backwards (current: {}, input: {})",
                current.get(),
                attempted.get()
            ),
        }
    }
}

impl std::error::Error for ManualTimeError {}

#[derive(Debug)]
struct WallClockInner {
    anchor: Instant,
    start: Tick,
    maximum_returned: AtomicU64,
}

#[derive(Clone, Debug)]
pub struct WallClockTimeSource {
    inner: Arc<WallClockInner>,
    cadence: NonZeroTicks,
    resolution: TicksPerSecond,
}

impl WallClockTimeSource {
    #[must_use]
    pub fn new(cadence: NonZeroTicks, resolution: TicksPerSecond) -> Self {
        Self {
            inner: Arc::new(WallClockInner {
                anchor: Instant::now(),
                start: Tick::ZERO,
                maximum_returned: AtomicU64::new(0),
            }),
            cadence,
            resolution,
        }
    }

    /// Resume the same recorded Tick axis with a fresh host monotonic anchor.
    #[must_use]
    pub fn starting_at(mut self, start: Tick) -> Self {
        self.inner = Arc::new(WallClockInner {
            anchor: Instant::now(),
            start,
            maximum_returned: AtomicU64::new(start.get()),
        });
        self
    }

    fn elapsed_tick(&self) -> u64 {
        let elapsed = self.inner.anchor.elapsed().as_nanos();
        let ticks = elapsed * u128::from(self.resolution.0) / 1_000_000_000;
        self.inner
            .start
            .get()
            .saturating_add(ticks.min(u64::MAX as u128) as u64)
    }

    fn duration_for_ticks(&self, ticks: u64) -> Duration {
        let numerator = u128::from(ticks) * 1_000_000_000;
        let denominator = u128::from(self.resolution.0);
        let nanos = numerator.div_ceil(denominator);
        let seconds = nanos / 1_000_000_000;
        if seconds > u64::MAX as u128 {
            return Duration::MAX;
        }
        Duration::new(seconds as u64, (nanos % 1_000_000_000) as u32)
    }
}

impl TickSource for WallClockTimeSource {
    fn current_tick(&self) -> Tick {
        let candidate = self.elapsed_tick();
        let previous = self
            .inner
            .maximum_returned
            .fetch_max(candidate, Ordering::AcqRel);
        Tick(max(previous, candidate))
    }

    fn wait_until(&self, tick: Tick) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            loop {
                let current = TickSource::current_tick(self);
                if current >= tick {
                    return;
                }
                let remaining = tick.get() - current.get();
                tokio::time::sleep(self.duration_for_ticks(remaining)).await;
            }
        })
    }

    fn heartbeat_cadence(&self) -> Ticks {
        self.cadence.get()
    }

    fn ticks_per_second(&self) -> u32 {
        self.resolution.get()
    }
}

pub type WallAnchored = WallClockTimeSource;

pub type LogDriven = LogDrivenTimeSource;

fn recover_lock<'a, T>(
    result: Result<MutexGuard<'a, T>, PoisonError<MutexGuard<'a, T>>>,
) -> MutexGuard<'a, T> {
    result.unwrap_or_else(PoisonError::into_inner)
}
