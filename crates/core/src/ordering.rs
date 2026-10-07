
use crate::identity::ProducerIdentity;
use crate::time::Tick;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Sequence(u64);

impl Sequence {
    pub const FIRST: Self = Self(0);

    pub const fn new(value: u64) -> Result<Self, SequenceError> {
        if value == u64::MAX {
            Err(SequenceError::ReservedMaximum)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    const fn next(self) -> Result<Self, SequenceError> {
        match self.0.checked_add(1) {
            Some(value) if value != u64::MAX => Ok(Self(value)),
            _ => Err(SequenceError::Exhausted),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceError {
    ReservedMaximum,
    Exhausted,
}

impl fmt::Display for SequenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedMaximum => formatter
                .write_str("the maximum sequence integer is reserved as the ordering bound"),
            Self::Exhausted => formatter.write_str("the producer's sequence domain is exhausted"),
        }
    }
}

impl std::error::Error for SequenceError {}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArrivalIndex(u64);

impl ArrivalIndex {
    pub const FIRST: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Display for ArrivalIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProducerSequencer {
    last: Option<Sequence>,
}

impl ProducerSequencer {
    #[must_use]
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Resume the producer's recorded emission sequence without issuing a value.
    pub fn retain_recorded(&mut self, sequence: Sequence) {
        self.last = Some(self.last.map_or(sequence, |last| last.max(sequence)));
    }

    pub fn next_sequence(&mut self) -> Result<Sequence, SequenceError> {
        let next = match self.last {
            Some(previous) => previous.next()?,
            None => Sequence::FIRST,
        };
        self.last = Some(next);
        Ok(next)
    }

    #[must_use]
    pub const fn last(&self) -> Option<Sequence> {
        self.last
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LogicalCounter(u64);

impl LogicalCounter {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Hlc {
    l: Tick,
    c: LogicalCounter,
}

impl Hlc {
    #[must_use]
    pub const fn new(l: Tick, c: LogicalCounter) -> Self {
        Self { l, c }
    }

    #[must_use]
    pub const fn from_physical(l: Tick) -> Self {
        Self::new(l, LogicalCounter::ZERO)
    }

    #[must_use]
    pub const fn l(self) -> Tick {
        self.l
    }

    #[must_use]
    pub const fn c(self) -> LogicalCounter {
        self.c
    }
}

/// Monotonic identity of an accepted graph revision inside one standing run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RevisionEpochId(u64);

impl RevisionEpochId {
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Stamp<P: ProducerIdentity> {
    hlc: Hlc,
    producer: P,
    sequence: Sequence,
    revision: RevisionEpochId,
}

impl<P: ProducerIdentity> Stamp<P> {
    #[must_use]
    pub fn from_event_producer(
        l: Tick,
        producer: P::EventProducer,
        sequence: Sequence,
        revision: RevisionEpochId,
    ) -> Self {
        Self::from_event_producer_at(Hlc::from_physical(l), producer, sequence, revision)
    }

    #[must_use]
    pub fn from_event_producer_at(
        hlc: Hlc,
        producer: P::EventProducer,
        sequence: Sequence,
        revision: RevisionEpochId,
    ) -> Self {
        Self::new(hlc, P::from_event_producer(producer), sequence, revision)
    }

    const fn new(hlc: Hlc, producer: P, sequence: Sequence, revision: RevisionEpochId) -> Self {
        Self {
            hlc,
            producer,
            sequence,
            revision,
        }
    }

    #[must_use]
    pub const fn physical_time(&self) -> Tick {
        self.hlc.l
    }

    #[must_use]
    pub const fn hlc(&self) -> Hlc {
        self.hlc
    }

    #[must_use]
    pub const fn producer(&self) -> &P {
        &self.producer
    }

    #[must_use]
    pub const fn sequence(&self) -> Sequence {
        self.sequence
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionEpochId {
        self.revision
    }

    #[must_use]
    pub fn timeline_cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.hlc, &self.producer, self.sequence, self.revision).cmp(&(
            other.hlc,
            &other.producer,
            other.sequence,
            other.revision,
        ))
    }
}

/// Record-only construction boundary. Event construction still requires EventProducer.
impl<P: crate::identity::RecordProducerIdentity> Stamp<P> {
    /// Record identities remain separate from event-producing identities.
    pub fn from_system_record_producer_at(
        hlc: Hlc,
        producer: P,
        sequence: Sequence,
        revision: RevisionEpochId,
    ) -> Option<Self> {
        producer
            .is_record_producer()
            .then(|| Self::new(hlc, producer, sequence, revision))
    }

    #[must_use]
    pub fn from_record_producer_at(
        hlc: Hlc,
        sequence: Sequence,
        revision: RevisionEpochId,
    ) -> Self {
        Self::new(hlc, P::record_producer(), sequence, revision)
    }
}

impl<P: ProducerIdentity> Ord for Stamp<P> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.timeline_cmp(other)
    }
}

impl<P: ProducerIdentity> PartialOrd for Stamp<P> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[must_use]
pub fn inherited_revision<'a, P: ProducerIdentity + 'a>(
    causes: impl IntoIterator<Item = &'a Stamp<P>>,
    current: RevisionEpochId,
) -> RevisionEpochId {
    causes
        .into_iter()
        .map(Stamp::revision)
        .max()
        .unwrap_or(current)
}

#[derive(Clone, Debug)]
pub struct StampIssuer<P: ProducerIdentity> {
    own: Option<Hlc>,
    observed: Option<Hlc>,
    last_sequence: BTreeMap<P, Sequence>,
}

impl<P: ProducerIdentity> Default for StampIssuer<P> {
    fn default() -> Self {
        Self {
            own: None,
            observed: None,
            last_sequence: BTreeMap::new(),
        }
    }
}

impl<P: ProducerIdentity> StampIssuer<P> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn own(&self) -> Option<Hlc> {
        self.own
    }

    #[must_use]
    pub fn last_sequence(&self, producer: &P) -> Option<Sequence> {
        self.last_sequence.get(producer).copied()
    }

    pub fn retain_issued(&mut self, own: Option<Hlc>, producer: &P, last: Option<Sequence>) {
        if let Some(own) = own {
            self.own = Some(self.own.map_or(own, |current| current.max(own)));
        }
        if let Some(last) = last {
            self.last_sequence
                .entry(producer.clone())
                .and_modify(|current| *current = (*current).max(last))
                .or_insert(last);
        }
    }

    /// Retain coordinates already issued by this actor during durable replay.
    /// Arrival ordinals and emission sequences are distinct; only emissions
    /// advance the per-producer emission sequence check.
    pub fn retain_recorded(&mut self, stamp: &Stamp<P>, emission: bool) {
        self.own = Some(self.own.map_or(stamp.hlc(), |own| own.max(stamp.hlc())));
        if emission {
            self.last_sequence
                .entry(stamp.producer().clone())
                .and_modify(|last| *last = (*last).max(stamp.sequence()))
                .or_insert(stamp.sequence());
        }
    }

    pub fn issue<'a>(
        &mut self,
        sample: Tick,
        producer: P::EventProducer,
        sequence: Sequence,
        causes: impl IntoIterator<Item = &'a Stamp<P>>,
        current: RevisionEpochId,
    ) -> Result<Stamp<P>, StampIssueError>
    where
        P: 'a,
    {
        let producer = P::from_event_producer(producer);
        self.issue_sequenced(sample, producer, sequence, causes, current)
    }

    /// Record-only producers keep their own observation sequence without
    /// widening the event-producer domain. An invalid record producer is absent.
    pub fn issue_record(
        &mut self,
        sample: Tick,
        producer: P,
        sequence: Sequence,
        current: RevisionEpochId,
    ) -> Option<Result<Stamp<P>, StampIssueError>>
    where
        P: crate::identity::RecordProducerIdentity,
    {
        producer
            .is_record_producer()
            .then(|| self.issue_sequenced(sample, producer, sequence, [], current))
    }

    fn issue_sequenced<'a>(
        &mut self,
        sample: Tick,
        producer: P,
        sequence: Sequence,
        causes: impl IntoIterator<Item = &'a Stamp<P>>,
        current: RevisionEpochId,
    ) -> Result<Stamp<P>, StampIssueError>
    where
        P: 'a,
    {
        match self.last_sequence.get(&producer) {
            Some(previous) if sequence <= *previous => {
                return Err(StampIssueError::SequenceDidNotIncrease {
                    previous: *previous,
                    attempted: sequence,
                });
            }
            _ => {}
        }
        let stamp = self.issue_coordinate(sample, producer.clone(), sequence, causes, current)?;
        self.last_sequence.insert(producer, sequence);
        Ok(stamp)
    }

    pub fn issue_arrival<'a>(
        &mut self,
        sample: Tick,
        producer: P::EventProducer,
        sequence: Sequence,
        causes: impl IntoIterator<Item = &'a Stamp<P>>,
        current: RevisionEpochId,
    ) -> Result<Stamp<P>, StampIssueError>
    where
        P: 'a,
    {
        self.issue_coordinate(
            sample,
            P::from_event_producer(producer),
            sequence,
            causes,
            current,
        )
    }

    fn issue_coordinate<'a>(
        &mut self,
        sample: Tick,
        producer: P,
        sequence: Sequence,
        causes: impl IntoIterator<Item = &'a Stamp<P>>,
        current: RevisionEpochId,
    ) -> Result<Stamp<P>, StampIssueError>
    where
        P: 'a,
    {
        let mut observed = self.observed;
        let revision = inherited_revision(
            causes.into_iter().inspect(|cause| {
                let coordinate = cause.hlc();
                observed = Some(observed.map_or(coordinate, |seen| seen.max(coordinate)));
            }),
            current,
        );
        let maximum_local_l = self
            .own
            .map(Hlc::l)
            .into_iter()
            .chain(observed.map(Hlc::l))
            .max();
        let l = maximum_local_l.map_or(sample, |maximum| sample.max(maximum));
        let c = if maximum_local_l.is_none_or(|maximum| sample > maximum) {
            LogicalCounter::ZERO
        } else {
            let maximum_c = self
                .own
                .filter(|coordinate| coordinate.l() == l)
                .map(Hlc::c)
                .into_iter()
                .chain(
                    observed
                        .filter(|coordinate| coordinate.l() == l)
                        .map(Hlc::c),
                )
                .max()
                .unwrap_or(LogicalCounter::ZERO);
            maximum_c
                .next()
                .ok_or(StampIssueError::LogicalCounterExhausted { at: l })?
        };
        let hlc = Hlc::new(l, c);
        self.own = Some(hlc);
        self.observed = observed;
        Ok(Stamp::new(hlc, producer, sequence, revision))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StampIssueError {
    LogicalCounterExhausted {
        at: Tick,
    },
    SequenceDidNotIncrease {
        previous: Sequence,
        attempted: Sequence,
    },
}

impl fmt::Display for StampIssueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LogicalCounterExhausted { at } => write!(
                formatter,
                "HLC logical counter exhausted for the same physical sample (l: {})",
                at.get()
            ),
            Self::SequenceDidNotIncrease {
                previous,
                attempted,
            } => write!(
                formatter,
                "the producer's sequence must strictly increase (previous: {}, input: {})",
                previous.get(),
                attempted.get()
            ),
        }
    }
}

impl std::error::Error for StampIssueError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct TestProducer(u8);

    impl ProducerIdentity for TestProducer {
        type EventProducer = Self;

        fn from_event_producer(producer: Self::EventProducer) -> Self {
            producer
        }
    }

    #[test]
    fn revision_is_current_on_entry_and_maximum_of_causes_on_inheritance() {
        let first = RevisionEpochId::new(1).unwrap();
        let second = RevisionEpochId::new(2).unwrap();
        let current = RevisionEpochId::new(9).unwrap();
        let causes: [Stamp<TestProducer>; 2] = [
            Stamp::from_event_producer(Tick::ZERO, TestProducer(2), Sequence::FIRST, first),
            Stamp::from_event_producer(Tick::ZERO, TestProducer(3), Sequence::FIRST, second),
        ];
        let mut issuer = StampIssuer::new();
        let entry = issuer
            .issue(Tick::ZERO, TestProducer(1), Sequence::FIRST, [], current)
            .unwrap();
        assert_eq!(entry.revision(), current);
        let inherited = issuer
            .issue(
                Tick::ZERO,
                TestProducer(1),
                Sequence::new(1).unwrap(),
                &causes,
                current,
            )
            .unwrap();
        assert_eq!(
            inherited.revision(),
            second,
            "current does not override older causes"
        );
        let arrival = issuer
            .issue_arrival(
                Tick::ZERO,
                TestProducer(1),
                Sequence::FIRST,
                &causes,
                current,
            )
            .unwrap();
        assert_eq!(arrival.revision(), second);
    }

    #[test]
    fn revision_is_the_fifth_timeline_key_and_part_of_identity() {
        let stamp = |time, sequence, revision| {
            Stamp::from_event_producer(
                Tick::new(time),
                TestProducer(1),
                Sequence::new(sequence).unwrap(),
                RevisionEpochId::new(revision).unwrap(),
            )
        };
        let first: Stamp<TestProducer> = stamp(0, 0, 1);
        let second = stamp(0, 0, 2);
        assert_ne!(first, second);
        assert_eq!(first.timeline_cmp(&second), std::cmp::Ordering::Less);
        assert_eq!(first.cmp(&second), std::cmp::Ordering::Less);
        assert!(stamp(0, 0, 9) < stamp(0, 1, 1));
        assert!(stamp(0, 1, 9) < stamp(1, 0, 1));
        assert_eq!(std::collections::BTreeSet::from([first, second]).len(), 2);
    }

    #[test]
    fn hlc_issue_known_answer_covers_same_sample_and_causal_promotion() {
        let producer = TestProducer(1);
        let mut issuer = StampIssuer::<TestProducer>::new();
        let first = issuer
            .issue(
                Tick::new(10),
                producer,
                Sequence::FIRST,
                std::iter::empty(),
                RevisionEpochId::new(1).expect("first revision"),
            )
            .unwrap();
        assert_eq!(first.hlc(), Hlc::new(Tick::new(10), LogicalCounter::ZERO));

        let second = issuer
            .issue(
                Tick::new(10),
                producer,
                Sequence::new(1).unwrap(),
                std::iter::empty(),
                RevisionEpochId::new(1).expect("first revision"),
            )
            .unwrap();
        assert_eq!(
            second.hlc(),
            Hlc::new(Tick::new(10), LogicalCounter::new(1))
        );

        let cause = Stamp::from_event_producer_at(
            Hlc::new(Tick::new(10), LogicalCounter::new(7)),
            TestProducer(2),
            Sequence::FIRST,
            RevisionEpochId::new(1).expect("first revision"),
        );
        let promoted = issuer
            .issue(
                Tick::new(10),
                producer,
                Sequence::new(2).unwrap(),
                std::iter::once(&cause),
                RevisionEpochId::new(1).expect("first revision"),
            )
            .unwrap();
        assert_eq!(
            promoted.hlc(),
            Hlc::new(Tick::new(10), LogicalCounter::new(8)),
            "promotes by exactly +1 from the largest c among causes with the same l",
        );

        let physical_advance = issuer
            .issue(
                Tick::new(11),
                producer,
                Sequence::new(3).unwrap(),
                std::iter::empty(),
                RevisionEpochId::new(1).expect("first revision"),
            )
            .unwrap();
        assert_eq!(
            physical_advance.hlc(),
            Hlc::new(Tick::new(11), LogicalCounter::ZERO),
            "when the physical sample l advances, c resets to 0",
        );
    }

    #[test]
    fn hlc_issue_lifts_a_regressed_sample_above_a_remote_cause() {
        let cause = Stamp::from_event_producer_at(
            Hlc::new(Tick::new(12), LogicalCounter::new(4)),
            TestProducer(2),
            Sequence::FIRST,
            RevisionEpochId::new(1).expect("first revision"),
        );
        let issued = StampIssuer::<TestProducer>::new()
            .issue(
                Tick::new(10),
                TestProducer(1),
                Sequence::FIRST,
                std::iter::once(&cause),
                RevisionEpochId::new(1).expect("first revision"),
            )
            .unwrap();
        assert_eq!(
            issued.hlc(),
            Hlc::new(Tick::new(12), LogicalCounter::new(5))
        );
        assert!(cause < issued);
    }

    #[test]
    fn arrival_sequence_uses_the_same_hlc_without_consuming_the_emission_column() {
        let producer = TestProducer(1);
        let sender = Stamp::from_event_producer_at(
            Hlc::new(Tick::new(10), LogicalCounter::new(4)),
            TestProducer(2),
            Sequence::new(9).unwrap(),
            RevisionEpochId::new(1).expect("first revision"),
        );
        let mut issuer = StampIssuer::<TestProducer>::new();

        let arrival = issuer
            .issue_arrival(
                Tick::new(10),
                producer,
                Sequence::FIRST,
                std::iter::once(&sender),
                RevisionEpochId::new(1).expect("first revision"),
            )
            .unwrap();
        let emission = issuer
            .issue(
                Tick::new(10),
                producer,
                Sequence::FIRST,
                [&arrival],
                RevisionEpochId::new(1).expect("first revision"),
            )
            .unwrap();

        assert_eq!(arrival.sequence(), Sequence::FIRST);
        assert_eq!(emission.sequence(), Sequence::FIRST);
        assert_eq!(arrival.hlc().c(), LogicalCounter::new(5));
        assert_eq!(emission.hlc().c(), LogicalCounter::new(6));
        assert!(arrival < emission);
    }
}
