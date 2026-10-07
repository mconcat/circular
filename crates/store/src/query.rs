
use crate::record::{ObservationBucket, Record, RecordOrigin, StoreSchema};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceSequence(u64);

impl SurfaceSequence {
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        Self(index as u64 + 1)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Bound {
    UpTo(SurfaceSequence),
    Before(SurfaceSequence),
    EndOfSealed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BucketRange {
    from: ObservationBucket,
    through: ObservationBucket,
}

impl BucketRange {
    pub fn try_new(
        from: ObservationBucket,
        through: ObservationBucket,
    ) -> Result<Self, BucketRangeError> {
        if from.millis() > through.millis() {
            return Err(BucketRangeError::Inverted);
        }
        Ok(Self { from, through })
    }

    #[must_use]
    pub const fn from(&self) -> ObservationBucket {
        self.from
    }

    #[must_use]
    pub const fn through(&self) -> ObservationBucket {
        self.through
    }

    #[must_use]
    pub const fn contains(&self, bucket: ObservationBucket) -> bool {
        bucket.millis() >= self.from.millis() && bucket.millis() <= self.through.millis()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BucketRangeError {
    Inverted,
}

impl fmt::Display for BucketRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inverted => formatter.write_str("bucket window starts after its end"),
        }
    }
}

impl std::error::Error for BucketRangeError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationScope<S: StoreSchema> {
    All,
    Stream,
    Origin(RecordOrigin<S>),
    Kind(S::ObservationKindKey),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedBound {
    UpTo(SurfaceSequence),
    Before(SurfaceSequence),
    Sealed(Option<SurfaceSequence>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum QueryFingerprint<S: StoreSchema> {
    Boundary {
        upto: ResolvedBound,
    },
    Display {
        key: S::DisplayKey,
        upto: ResolvedBound,
    },
    Observation {
        scope: ObservationScope<S>,
        buckets: Option<BucketRange>,
        upto: ResolvedBound,
    },
    Structure {
        upto: ResolvedBound,
    },
    Arrival {
        actor: S::ActorId,
        upto: ResolvedBound,
    },
    ExternalArrivalScan {
        upto: ResolvedBound,
    },
}

#[derive(Clone, Debug)]
pub struct Cursor<S: StoreSchema> {
    pub(crate) store_generation: Arc<()>,
    pub(crate) query_fingerprint: QueryFingerprint<S>,
    pub(crate) position: usize,
}

impl<S: StoreSchema> PartialEq for Cursor<S> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.store_generation, &other.store_generation)
            && self.query_fingerprint == other.query_fingerprint
            && self.position == other.position
    }
}

impl<S: StoreSchema> Eq for Cursor<S> {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanStart<S: StoreSchema> {
    Beginning,
    After(Cursor<S>),
    Tail { count: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Query<S: StoreSchema> {
    BoundaryScan {
        start: ScanStart<S>,
        upto: Bound,
    },
    StructureOf {
        start: ScanStart<S>,
        upto: Bound,
    },
    DisplayRange {
        key: S::DisplayKey,
        start: ScanStart<S>,
        upto: Bound,
    },
    ObservationScan {
        scope: ObservationScope<S>,
        buckets: Option<BucketRange>,
        start: ScanStart<S>,
        upto: Bound,
    },
    ArrivalScan {
        actor: S::ActorId,
        start: ScanStart<S>,
        upto: Bound,
    },
    ExternalArrivalScan {
        start: ScanStart<S>,
        upto: Bound,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<S: StoreSchema> {
    records: Box<[Record<S>]>,
    next: Option<Cursor<S>>,
}

impl<S: StoreSchema> Page<S> {
    pub(crate) fn new(records: Vec<Record<S>>, next: Option<Cursor<S>>) -> Self {
        Self {
            records: records.into_boxed_slice(),
            next,
        }
    }

    #[must_use]
    pub fn terminal(records: Vec<Record<S>>) -> Self {
        Self::new(records, None)
    }

    #[must_use]
    pub fn records(&self) -> &[Record<S>] {
        &self.records
    }

    #[must_use]
    pub const fn next(&self) -> Option<&Cursor<S>> {
        self.next.as_ref()
    }

    #[must_use]
    pub fn into_next(self) -> Option<Cursor<S>> {
        self.next
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CursorRejection {
    DifferentStore,
    DifferentQuery,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError<S: StoreSchema> {
    Cursor(CursorRejection),
    MissingManifest,
    UnrecordedDisplayKey {
        requested: S::DisplayKey,
        recorded: Box<[S::DisplayKey]>,
    },
    PageSizeTooLarge {
        requested: usize,
        maximum: usize,
    },
    UnsupportedInSlice,
}

impl<S: StoreSchema> fmt::Display for StoreError<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cursor(_) => formatter.write_str("cursor is invalid for the current query"),
            Self::MissingManifest => {
                formatter.write_str("manifest required for Boundary query is missing")
            }
            Self::UnrecordedDisplayKey { recorded, .. } => write!(
                formatter,
                "display tick is not recorded; {} ticks are recorded",
                recorded.len()
            ),
            Self::PageSizeTooLarge { requested, maximum } => write!(
                formatter,
                "requested page size {requested} exceeds deployment limit {maximum}"
            ),
            Self::UnsupportedInSlice => {
                formatter.write_str("this query is outside the current implementation slice")
            }
        }
    }
}

impl<S: StoreSchema> std::error::Error for StoreError<S> {}
