
use std::fmt::Debug;

pub trait ProducerIdentity: Clone + Debug + Ord + std::hash::Hash {
    type EventProducer: Clone + Debug;

    fn from_event_producer(producer: Self::EventProducer) -> Self;
}

pub trait StreamIdentity: Clone + Debug + Eq + std::hash::Hash {}

pub trait OperationIdentity: Clone + Debug + Eq {}

impl OperationIdentity for std::convert::Infallible {}

pub trait ProducerLocalOperation<P: ProducerIdentity>: OperationIdentity {
    fn issue(producer: &P, sequence: crate::ordering::Sequence) -> Self;
}

/// Identity authority for the reserved producer of stream-level records.
///
/// This does not extend `ProducerIdentity::EventProducer`: records are facts,
/// not graph emissions. The concrete identity owner selects the reserved value.
pub trait RecordProducerIdentity: ProducerIdentity {
    fn record_producer() -> Self;
    fn is_record_producer(&self) -> bool;
}
