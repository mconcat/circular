use crate::query_positions::QueryPositions;
use crate::record::{
    BoundaryKey, Class, ClassKey, ObservationKey, Record, StoreSchema, StructureKey,
};
use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(crate) struct SurfaceIndex<S: StoreSchema> {
    manifest: bool,
    structure_positions: Vec<usize>,
    arrival_positions: HashMap<S::ActorId, QueryPositions>,
    emission_positions: HashMap<S::Producer, QueryPositions>,
    display_positions: HashMap<(S::Producer, S::DisplayKey), QueryPositions>,
    lifecycle_positions: HashMap<S::ObservationKindKey, QueryPositions>,
}

impl<S: StoreSchema> Default for SurfaceIndex<S> {
    fn default() -> Self {
        Self {
            manifest: false,
            structure_positions: Vec::new(),
            arrival_positions: HashMap::new(),
            emission_positions: HashMap::new(),
            display_positions: HashMap::new(),
            lifecycle_positions: HashMap::new(),
        }
    }
}

fn append_sealed_batches<K: Eq + std::hash::Hash>(
    index: &mut HashMap<K, QueryPositions>,
    batches: HashMap<K, Vec<(u64, usize)>>,
) {
    for (key, batch) in batches {
        index.entry(key).or_default().append(batch);
    }
}

impl<S: StoreSchema> SurfaceIndex<S> {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn observe_appended(&mut self, index: usize, record: &Record<S>) {
        if let ClassKey::Structure(StructureKey::Manifest) = record.header().key() {
            self.manifest = true;
        }
        if record.header().class() == Class::Structure {
            self.structure_positions.push(index);
        }
    }

    pub(crate) fn observe_sealed<'r>(
        &mut self,
        rows: impl IntoIterator<Item = (usize, Cow<'r, Record<S>>)>,
    ) where
        S: 'r,
    {
        let mut arrivals = HashMap::<_, Vec<(u64, usize)>>::new();
        let mut emissions = HashMap::<_, Vec<(u64, usize)>>::new();
        let mut lifecycles = HashMap::<_, Vec<(u64, usize)>>::new();
        let mut displays = HashMap::<_, Vec<(u64, usize)>>::new();
        for (position, record) in rows {
            match &*record {
                Record::Boundary(row) => match (row.header().key(), row.fact()) {
                    (
                        ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }),
                        crate::BoundaryFact::Arrival { arrival_index, .. },
                    ) => arrivals
                        .entry(actor.clone())
                        .or_default()
                        .push((arrival_index.get(), position)),
                    (
                        ClassKey::Boundary(BoundaryKey::EmissionBody { producer, .. }),
                        crate::BoundaryFact::EmissionBody { cause, .. },
                    ) => emissions
                        .entry(producer.clone())
                        .or_default()
                        .push((cause.get(), position)),
                    _ => {}
                },
                Record::Observation(row)
                    if matches!(row.fact(), crate::ObservationFact::Lifecycle(_)) =>
                {
                    if let ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) =
                        row.header().key()
                    {
                        lifecycles
                            .entry(item.kind().clone())
                            .or_default()
                            .push((position as u64, position));
                    }
                }
                Record::Display(row) => {
                    let ClassKey::Display { key, .. } = row.header().key() else {
                        unreachable!("display key")
                    };
                    displays
                        .entry((row.header().at().producer().clone(), key.clone()))
                        .or_default()
                        .push((row.header().at().sequence().get(), position));
                }
                _ => {}
            }
        }
        append_sealed_batches(&mut self.arrival_positions, arrivals);
        append_sealed_batches(&mut self.emission_positions, emissions);
        append_sealed_batches(&mut self.lifecycle_positions, lifecycles);
        append_sealed_batches(&mut self.display_positions, displays);
    }

    pub(crate) fn note_manifest(&mut self) {
        self.manifest = true;
    }

    pub(crate) fn has_manifest(&self) -> bool {
        self.manifest
    }

    pub(crate) fn structure_count_before(&self, end: usize) -> usize {
        self.structure_positions.partition_point(|at| *at < end)
    }

    pub(crate) fn structure_positions_before(
        &self,
        end: usize,
    ) -> impl Iterator<Item = usize> + '_ {
        self.structure_positions
            .iter()
            .copied()
            .take_while(move |position| *position < end)
    }

    /// Sealed per-actor arrival bounds. The ordinal is an exclusive prefix bound.
    pub(crate) fn arrival_coordinate_bounds(&self) -> impl Iterator<Item = (&S::ActorId, u64)> {
        self.arrival_positions
            .iter()
            .map(|(actor, positions)| (actor, positions.end()))
    }

    pub(crate) fn arrival_positions_since(
        &self,
        actor: &S::ActorId,
        from: u64,
    ) -> impl Iterator<Item = usize> + '_ {
        self.arrival_positions
            .get(actor)
            .into_iter()
            .flat_map(move |positions| positions.since(from))
    }

    pub(crate) fn emission_positions_between(
        &self,
        producer: &S::Producer,
        from: u64,
        end: u64,
    ) -> impl Iterator<Item = usize> + '_ {
        self.emission_positions
            .get(producer)
            .into_iter()
            .flat_map(move |positions| positions.coordinates_since(from))
            .take_while(move |(cause, _)| *cause < end)
            .map(|(_, position)| position)
    }

    pub(crate) fn emission_first_cause(&self, producer: &S::Producer) -> Option<u64> {
        self.emission_positions
            .get(producer)
            .and_then(QueryPositions::first)
    }

    pub(crate) fn arrival_first_ordinal(&self, actor: &S::ActorId) -> Option<u64> {
        self.arrival_positions
            .get(actor)
            .and_then(QueryPositions::first)
    }

    pub(crate) fn display_coordinate_bounds(&self) -> impl Iterator<Item = (&S::Producer, u64)> {
        self.display_positions
            .iter()
            .map(|((actor, _), positions)| (actor, positions.end()))
    }

    pub(crate) fn display_positions_since(
        &self,
        actor: &S::Producer,
        from: u64,
    ) -> impl Iterator<Item = usize> + '_ {
        let actor = actor.clone();
        self.display_positions
            .iter()
            .filter(move |((producer, _), _)| producer == &actor)
            .flat_map(move |(_, positions)| positions.since(from))
    }

    pub(crate) fn recorded_display_keys(&self) -> impl Iterator<Item = &S::DisplayKey> {
        self.display_positions.keys().map(|(_, key)| key)
    }

    pub(crate) fn lifecycle_positions_of(
        &self,
        kind: &S::ObservationKindKey,
    ) -> impl Iterator<Item = usize> + '_ {
        self.lifecycle_positions
            .get(kind)
            .into_iter()
            .flat_map(|positions| positions.since(0))
    }
}
