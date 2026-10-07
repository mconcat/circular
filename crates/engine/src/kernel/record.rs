
use circular_core::{ArrivalIndex, Hlc, RevisionEpochId, Sequence, Stamp, StampIssuer, Tick};
use circular_plan::{ActorId, NamedActorId};
use circular_store::{BoundaryFact, BoundaryKey, ClassKey, ProductStore, Record};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) enum IssueError {
    Exhausted,
    NotEventProducer,
    Stamp(circular_core::StampIssueError),
}

pub(crate) fn owner(record: &Record<ProductStore>) -> &ActorId {
    match record.header().key() {
        ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }) => actor,
        _ => record.header().at().producer(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LifeStart {
    index: ArrivalIndex,
    at: Stamp<ActorId>,
}

impl LifeStart {
    fn of(record: &Record<ProductStore>) -> Option<(&ActorId, Self)> {
        let (Record::Boundary(boundary), ClassKey::Boundary(BoundaryKey::Arrival { actor, .. })) =
            (record, record.header().key())
        else {
            return None;
        };
        let BoundaryFact::Arrival {
            inlet: Some(inlet),
            body,
            arrival_index,
            ..
        } = boundary.fact()
        else {
            return None;
        };
        if !super::actor::is_lifecycle(inlet) {
            return None;
        }
        let value = circular_core::decode(
            body.payload()?.body(),
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        )
        .ok()?;
        matches!(
            super::turn::lifecycle(inlet, &value),
            Ok(Some(super::actor::Life::Activate))
        )
        .then(|| {
            (
                actor,
                Self {
                    index: *arrival_index,
                    at: record.header().at().clone(),
                },
            )
        })
    }

    pub(crate) const fn index(&self) -> ArrivalIndex {
        self.index
    }

    pub(crate) fn holds(&self, at: &Stamp<ActorId>) -> bool {
        at.hlc() >= self.at.hlc()
    }
}

#[derive(Default)]
pub(crate) struct LifeStarts(BTreeMap<ActorId, LifeStart>);

impl LifeStarts {
    pub(crate) fn retain(&mut self, record: &Record<ProductStore>) {
        let Some((actor, start)) = LifeStart::of(record) else {
            return;
        };
        match self.0.get_mut(actor) {
            Some(known) if known.index >= start.index => {}
            Some(known) => *known = start,
            None => {
                self.0.insert(actor.clone(), start);
            }
        }
    }

    pub(crate) fn of<'a>(records: impl IntoIterator<Item = &'a Record<ProductStore>>) -> Self {
        let mut starts = Self::default();
        for record in records {
            starts.retain(record);
        }
        starts
    }

    pub(crate) fn get(&self, actor: &ActorId) -> Option<&LifeStart> {
        self.0.get(actor)
    }

    pub(crate) fn holds(&self, actor: &ActorId, at: &Stamp<ActorId>) -> bool {
        self.get(actor).is_none_or(|start| start.holds(at))
    }

    pub(crate) fn emitted<'a>(
        &self,
        records: impl IntoIterator<Item = &'a Record<ProductStore>>,
    ) -> BTreeMap<ActorId, Emitted> {
        let mut emitted = BTreeMap::<ActorId, Emitted>::new();
        for record in records {
            let Record::Boundary(boundary) = record else {
                continue;
            };
            if !matches!(boundary.fact(), BoundaryFact::EmissionBody { .. }) {
                continue;
            }
            let at = record.header().at();
            if self.holds(at.producer(), at) {
                continue;
            }
            emitted.entry(at.producer().clone()).or_default().retain(at);
        }
        emitted
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Emitted {
    last: Option<Sequence>,
    own: Option<Hlc>,
}

impl Emitted {
    fn retain(&mut self, at: &Stamp<ActorId>) {
        self.last = self.last.max(Some(at.sequence()));
        self.own = self.own.max(Some(at.hlc()));
    }
}

pub(crate) struct Issuer {
    actor: ActorId,
    stamps: StampIssuer<ActorId>,
    emissions: StampIssuer<ActorId>,
    next: ArrivalIndex,
}

impl Issuer {
    pub(crate) fn new(actor: impl Into<ActorId>) -> Self {
        Self {
            actor: actor.into(),
            stamps: StampIssuer::new(),
            emissions: StampIssuer::new(),
            next: ArrivalIndex::FIRST,
        }
    }

    pub(crate) fn retain(&mut self, record: &Record<ProductStore>) -> Result<(), String> {
        let at = record.header().at();
        if let Record::Boundary(boundary) = record {
            match boundary.fact() {
                BoundaryFact::Arrival { arrival_index, .. } => {
                    self.observe(at);
                    self.next = self
                        .next
                        .max(arrival_index.next().ok_or("arrival index exhausted")?);
                }
                BoundaryFact::EmissionBody { .. } => {
                    self.resume_emissions(Some(at.sequence()), Some(at.hlc()));
                }
                _ => {}
            }
        }
        if matches!(self.actor, ActorId::System(_)) || matches!(record, Record::Observation(_)) {
            self.stamps
                .retain_issued(Some(at.hlc()), &self.actor, Some(at.sequence()));
        }
        Ok(())
    }

    pub(crate) fn replaying(mut self, before: &Emitted) -> Self {
        self.emissions = StampIssuer::new();
        self.resume_emissions(before.last, before.own);
        self
    }

    pub(crate) fn begin_life(&mut self) {
        self.stamps
            .retain_issued(self.emissions.own(), &self.actor, None);
    }

    fn named(&self) -> Result<NamedActorId, IssueError> {
        match &self.actor {
            ActorId::Scoped {
                scope,
                local: circular_plan::LocalKey::Named(name),
            } => Ok(NamedActorId::new(scope.clone(), name.clone())),
            _ => Err(IssueError::NotEventProducer),
        }
    }

    pub(crate) const fn next_index(&self) -> ArrivalIndex {
        self.next
    }

    pub(crate) fn arrival<'a>(
        &mut self,
        sample: Tick,
        parents: impl IntoIterator<Item = &'a Stamp<ActorId>>,
        revision: RevisionEpochId,
    ) -> Result<(ArrivalIndex, Stamp<ActorId>), IssueError> {
        let index = self.next;
        let sequence = Sequence::new(index.get()).map_err(|_| IssueError::Exhausted)?;
        let at = self
            .stamps
            .issue_arrival(sample, self.named()?, sequence, parents, revision)
            .map_err(IssueError::Stamp)?;
        Ok((index, at))
    }

    pub(crate) fn recorded(&mut self, index: ArrivalIndex) -> Result<(), IssueError> {
        self.next = index.next().ok_or(IssueError::Exhausted)?;
        Ok(())
    }

    pub(crate) fn observe(&mut self, recorded: &Stamp<ActorId>) {
        self.stamps.retain_recorded(recorded, false);
    }

    pub(crate) fn resume_at(&mut self, next: ArrivalIndex) {
        self.next = next;
    }

    pub(crate) fn emissions(&self) -> (Option<Sequence>, Option<Hlc>) {
        (
            self.emissions.last_sequence(&self.actor),
            self.emissions.own(),
        )
    }

    pub(crate) fn resume_emissions(&mut self, last: Option<Sequence>, own: Option<Hlc>) {
        self.emissions.retain_issued(own, &self.actor, last);
    }

    pub(crate) fn observation(
        &mut self,
        now: Tick,
        revision: RevisionEpochId,
    ) -> Result<Stamp<ActorId>, IssueError> {
        let producer = self.actor.clone();
        let sequence = match self.stamps.last_sequence(&producer) {
            Some(last) => last
                .get()
                .checked_add(1)
                .and_then(|next| Sequence::new(next).ok())
                .ok_or(IssueError::Exhausted)?,
            None => Sequence::FIRST,
        };
        match &self.actor {
            ActorId::System(_) => self
                .stamps
                .issue_record(now, producer, sequence, revision)
                .ok_or(IssueError::NotEventProducer)?,
            _ => self
                .stamps
                .issue(now, self.named()?, sequence, [], revision),
        }
        .map_err(IssueError::Stamp)
    }

    pub(crate) fn emission_revision(
        &self,
        input: &Stamp<ActorId>,
        cause: &Stamp<ActorId>,
        current: RevisionEpochId,
    ) -> RevisionEpochId {
        circular_core::inherited_revision([cause, input], current)
    }

    pub(crate) fn emission(
        &mut self,
        input: &Stamp<ActorId>,
        cause: &Stamp<ActorId>,
        revision: RevisionEpochId,
    ) -> Result<Stamp<ActorId>, IssueError> {
        let producer = self.actor.clone();
        let sequence = match self.emissions.last_sequence(&producer) {
            Some(last) => last
                .get()
                .checked_add(1)
                .and_then(|next| Sequence::new(next).ok())
                .ok_or(IssueError::Exhausted)?,
            None => Sequence::FIRST,
        };
        self.emissions
            .issue(
                input.physical_time(),
                self.named()?,
                sequence,
                [cause, input],
                revision,
            )
            .map_err(IssueError::Stamp)
    }
}

