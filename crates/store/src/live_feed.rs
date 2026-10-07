
use crate::record::{
    BoundaryFact, BoundaryKey, BoundaryRecord, ClassKey, DisplayRecord, ObservationFact,
    ObservationRecord, Record, StoreSchema,
};

#[derive(Debug, Eq, PartialEq)]
pub enum LiveFrame<'surface, S: StoreSchema> {
    Display {
        key: &'surface S::DisplayKey,
        record: &'surface DisplayRecord<S>,
    },
    ActorEvent {
        actor: &'surface S::ActorId,
        record: &'surface BoundaryRecord<S>,
    },
    ActorEmission {
        producer: &'surface S::Producer,
        record: &'surface BoundaryRecord<S>,
    },
    Failure {
        record: &'surface ObservationRecord<S>,
    },
}

impl<'surface, S: StoreSchema> LiveFrame<'surface, S> {
    #[must_use]
    pub fn of(record: &'surface Record<S>) -> Option<Self> {
        match record {
            Record::Display(display) => match display.header().key() {
                crate::record::ClassKey::Display { key, .. } => Some(Self::Display {
                    key,
                    record: display,
                }),
                _ => None,
            },
            Record::Observation(observation) => match observation.fact() {
                ObservationFact::DeadLetter(_) => Some(Self::Failure {
                    record: observation,
                }),
                ObservationFact::Lifecycle(_)
                | ObservationFact::Diagnostic(_)
                | ObservationFact::Accounting(_)
                | ObservationFact::ReplaySessionTransition(_)
                | ObservationFact::Restart(_)
                | ObservationFact::Checkpoint(_) => None,
            },
            Record::Boundary(boundary) => match (boundary.header().key(), boundary.fact()) {
                (
                    ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }),
                    BoundaryFact::Arrival { .. },
                ) => Some(Self::ActorEvent {
                    actor,
                    record: boundary,
                }),
                (
                    ClassKey::Boundary(BoundaryKey::EmissionBody { producer, .. }),
                    BoundaryFact::EmissionBody { .. },
                ) => Some(Self::ActorEmission {
                    producer,
                    record: boundary,
                }),
                _ => None,
            },
            Record::Structure(_) => None,
        }
    }
}
