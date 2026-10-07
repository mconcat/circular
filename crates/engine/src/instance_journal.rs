
use circular_core::{BuiltinObservationName, EncodedPayload, PayloadVersionTag, Value};
use circular_runtime::{InstanceAuthority, InstanceDisposition, InstanceKey, ScopeId};
use circular_store::{
    ClassKey, IncarnationId, ObservationBucket, ObservationFact, ObservationItemKey,
    ObservationKey, ObservationRecord, OpaqueId, ProductStore, Record, RecordOrigin,
};
use std::collections::BTreeMap;
use std::fmt;

const MINTED_TAG: i64 = 1;
const RETIRED_TAG: i64 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceTransitionRecordError {
    ScopeNotPublishable,
    CarrierTooLarge,
    NotActorOwned,
    NotLifecycle,
    PayloadNotCanonical,
    CarrierNotArray,
    CarrierArity,
    UnknownTransitionKind,
    MalformedScope,
    MalformedKey,
    MalformedKeyPath,
}

impl InstanceTransitionRecordError {
    #[must_use]
    pub const fn detail(&self) -> &'static str {
        match self {
            Self::ScopeNotPublishable => "instance-set scope is not publishable",
            Self::CarrierTooLarge => "instance transition exceeds codec ceilings",
            Self::NotActorOwned => "instance transition is not owned by an actor incarnation",
            Self::NotLifecycle => "instance transition is not a lifecycle occurrence",
            Self::PayloadNotCanonical => "instance transition payload is not canonical",
            Self::CarrierNotArray => "instance transition carrier is not [kind, scope, key]",
            Self::CarrierArity => "instance transition carrier does not have three fields",
            Self::UnknownTransitionKind => "instance transition kind is unknown",
            Self::MalformedScope => "instance-set scope is malformed",
            Self::MalformedKey => "instance key is malformed",
            Self::MalformedKeyPath => "container key path is malformed",
        }
    }
}

impl fmt::Display for InstanceTransitionRecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.detail())
    }
}

impl std::error::Error for InstanceTransitionRecordError {}

fn disposition_tag(disposition: InstanceDisposition) -> Option<i64> {
    match disposition {
        InstanceDisposition::Minted => Some(MINTED_TAG),
        InstanceDisposition::Retired => Some(RETIRED_TAG),
        InstanceDisposition::AlreadyLive
        | InstanceDisposition::NotLive
        | InstanceDisposition::RejectedCapacity { .. } => None,
    }
}

fn transition_value(
    disposition: InstanceDisposition,
    scope: &ScopeId,
    key: &InstanceKey,
) -> Result<Option<Value>, InstanceTransitionRecordError> {
    let Some(tag) = disposition_tag(disposition) else {
        return Ok(None);
    };
    let scope = circular_store::scope_value(scope)
        .map_err(|_| InstanceTransitionRecordError::ScopeNotPublishable)?;
    Ok(Some(Value::array([
        Value::int(tag),
        scope,
        circular_store::instance_key_value(key),
    ])))
}

pub fn instance_transition_record(
    disposition: InstanceDisposition,
    scope: ScopeId,
    key: InstanceKey,
    at: circular_core::Stamp<circular_plan::ActorId>,
    producer_incarnation: u64,
) -> Result<Option<ObservationRecord<ProductStore>>, InstanceTransitionRecordError> {
    let Some(value) = transition_value(disposition, &scope, &key)? else {
        return Ok(None);
    };
    let payload = circular_core::encode(
        &value,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
    )
    .map_err(|_| InstanceTransitionRecordError::CarrierTooLarge)?;
    Ok(Some(ObservationRecord::lifecycle(
        at.clone(),
        ObservationBucket::from_millis(at.physical_time().get()),
        RecordOrigin::Actor(IncarnationId::new(producer_incarnation)),
        ObservationItemKey::new(BuiltinObservationName::InstanceTransition, OpaqueId::new(0)),
        EncodedPayload::new(PayloadVersionTag::FIRST, &payload),
    )))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecodedTransition {
    pub(crate) disposition: InstanceDisposition,
    scope: ScopeId,
    pub(crate) key: InstanceKey,
    value: Value,
}

pub(crate) fn decode_transition(
    record: &Record<ProductStore>,
) -> Result<Option<DecodedTransition>, InstanceTransitionRecordError> {
    let Record::Observation(observation) = record else {
        return Ok(None);
    };
    let ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) = observation.header().key()
    else {
        return Ok(None);
    };
    if item.kind() != &BuiltinObservationName::InstanceTransition {
        return Ok(None);
    }
    if !matches!(observation.header().origin(), RecordOrigin::Actor(_)) {
        return Err(InstanceTransitionRecordError::NotActorOwned);
    }
    let ObservationFact::Lifecycle(payload) = observation.fact() else {
        return Err(InstanceTransitionRecordError::NotLifecycle);
    };
    let value = circular_core::decode(
        payload.body(),
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
    )
    .map_err(|_| InstanceTransitionRecordError::PayloadNotCanonical)?;
    let Value::Array(fields) = &value else {
        return Err(InstanceTransitionRecordError::CarrierNotArray);
    };
    if fields.len() != 3 {
        return Err(InstanceTransitionRecordError::CarrierArity);
    }
    let disposition = match fields[0].as_int() {
        Some(MINTED_TAG) => InstanceDisposition::Minted,
        Some(RETIRED_TAG) => InstanceDisposition::Retired,
        _ => {
            return Err(InstanceTransitionRecordError::UnknownTransitionKind);
        }
    };
    let scope = circular_store::scope_from_value(&fields[1])
        .map_err(|_| InstanceTransitionRecordError::MalformedScope)?;
    let key = circular_store::instance_key_from_value(&fields[2])
        .map_err(|_| InstanceTransitionRecordError::MalformedKey)?;
    Ok(Some(DecodedTransition {
        disposition,
        scope,
        key,
        value,
    }))
}

pub fn instance_transition_value(
    record: &Record<ProductStore>,
) -> Result<Option<Value>, InstanceTransitionRecordError> {
    Ok(decode_transition(record)?.map(|transition| transition.value))
}

/// The concrete cell scope of a recorded mint. Retirement does not erase a mint.
/// Callers retain record order and deduplicate remints of the same scope.
pub fn minted_instance_scope(
    record: &Record<ProductStore>,
) -> Result<Option<ScopeId>, InstanceTransitionRecordError> {
    let Some(transition) = decode_transition(record)? else {
        return Ok(None);
    };
    if transition.disposition != InstanceDisposition::Minted {
        return Ok(None);
    }
    let mut segments = transition.scope.segments().to_vec();
    let Some(circular_plan::ScopeSeg::Child(of)) = segments.pop() else {
        return Err(InstanceTransitionRecordError::MalformedScope);
    };
    segments.push(circular_plan::ScopeSeg::Instance {
        of,
        key: transition.key,
    });
    ScopeId::from_segments(segments)
        .map(Some)
        .map_err(|_| InstanceTransitionRecordError::MalformedScope)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InstanceLifecycle {
    live: BTreeMap<InstanceKey, (circular_core::Stamp<circular_plan::ActorId>, u64)>,
    declared: Option<(u64, circular_core::RevisionEpochId)>,
    configurations:
        BTreeMap<circular_core::RevisionEpochId, Option<circular_actors::config::PayloadPath>>,
    active: Option<(
        circular_core::RevisionEpochId,
        Option<circular_actors::config::PayloadPath>,
    )>,
    incarnation_began: Option<circular_core::RevisionEpochId>,
    incarnation: Option<u64>,
}

impl InstanceLifecycle {
    pub(crate) fn read(
        &mut self,
        record: &Record<ProductStore>,
    ) -> Result<(), InstanceTransitionRecordError> {
        if let Some((_, incarnation)) = crate::incarnation_transition::standing_incarnation(record)
            .map_err(|_| InstanceTransitionRecordError::PayloadNotCanonical)?
        {
            if self
                .declared
                .is_some_and(|(_, began)| record.header().at().revision() >= began)
            {
                self.incarnation(incarnation);
            }
        }
        if let Some(transition) = decode_transition(record)? {
            let RecordOrigin::Actor(incarnation) = record.header().origin() else {
                unreachable!("validated transition owner")
            };
            self.transition(
                transition.disposition,
                transition.key,
                record.header().at().clone(),
                incarnation.get(),
            );
        }
        Ok(())
    }

    pub(crate) fn declare(
        &mut self,
        revision: circular_core::RevisionEpochId,
        declaration: Option<&circular_plan::ActorDecl>,
    ) -> Result<(), InstanceTransitionRecordError> {
        let declaration = declaration.filter(|declaration| {
            *declaration.domain().actor_type() == circular_core::ActorType::Replicator
        });
        let Some(declaration) = declaration else {
            self.declared = None;
            self.live.clear();
            return Ok(());
        };
        let generation = declaration.authored_generation();
        let began = self
            .declared
            .filter(|(before, _)| *before == generation)
            .map_or(revision, |(_, began)| began);
        if self.declared.is_none_or(|(before, _)| before != generation) {
            self.incarnation = None;
        }
        self.declared = Some((generation, began));
        let at = declaration
            .domain()
            .config()
            .record()
            .entries()
            .iter()
            .find(|(name, _)| {
                name.as_str() == circular_actors::replicator_actor::ReplicatorRouting::AT
            })
            .map(|(_, value)| {
                let value = value
                    .to_wire_value()
                    .map_err(|_| InstanceTransitionRecordError::MalformedKeyPath)?;
                circular_actors::config::payload_path_from_value(&value)
                    .map_err(|_| InstanceTransitionRecordError::MalformedKeyPath)
            })
            .transpose()?;
        if self
            .configurations
            .last_key_value()
            .is_none_or(|(_, before)| before != &at)
        {
            self.configurations.insert(revision, at.clone());
        }
        if self.active.is_none() {
            self.active = Some((revision, at));
        }
        self.discard_ended();
        Ok(())
    }

    pub(crate) fn arrive(&mut self, revision: circular_core::RevisionEpochId) {
        let Some((&configured, at)) = self.configurations.range(..=revision).next_back() else {
            return;
        };
        if self.active.as_ref().is_some_and(|(_, before)| before != at) {
            self.incarnation_began = Some(revision);
            self.incarnation = Some(revision.get());
        }
        self.active = Some((revision, at.clone()));
        self.configurations.retain(|at, _| *at >= configured);
        self.discard_ended();
    }

    pub(crate) fn incarnation(&mut self, incarnation: u64) {
        self.incarnation = Some(incarnation);
        self.discard_ended();
    }

    fn discard_ended(&mut self) {
        let ended: Vec<_> = self
            .live
            .iter()
            .filter(|(_, (at, owner))| !self.within_life(at, *owner))
            .map(|(key, _)| key.clone())
            .collect();
        for key in ended {
            self.live.remove(&key);
        }
    }

    pub(crate) fn transition(
        &mut self,
        disposition: InstanceDisposition,
        key: InstanceKey,
        at: circular_core::Stamp<circular_plan::ActorId>,
        incarnation: u64,
    ) {
        match disposition {
            InstanceDisposition::Minted => {
                if self.within_life(&at, incarnation) {
                    self.live.insert(key, (at, incarnation));
                }
            }
            InstanceDisposition::Retired => {
                if self.live.get(&key).is_some_and(|(minted, owner)| {
                    *owner == incarnation && minted.revision() <= at.revision()
                }) {
                    self.live.remove(&key);
                }
            }
            _ => {}
        }
    }

    fn within_life(
        &self,
        minted: &circular_core::Stamp<circular_plan::ActorId>,
        owner: u64,
    ) -> bool {
        self.declared
            .is_some_and(|(_, began)| minted.revision() >= began)
            && self
                .incarnation_began
                .is_none_or(|began| minted.revision() >= began)
            && self
                .incarnation
                .is_none_or(|incarnation| incarnation == owner)
    }

    pub(crate) fn minted(
        &self,
    ) -> impl Iterator<Item = (&InstanceKey, &circular_core::Stamp<circular_plan::ActorId>)> {
        self.live
            .iter()
            .filter(|(_, (at, owner))| self.within_life(at, *owner))
            .map(|(key, (at, _))| (key, at))
    }

    pub(crate) fn get(
        &self,
        key: &InstanceKey,
    ) -> Option<&circular_core::Stamp<circular_plan::ActorId>> {
        self.live
            .get(key)
            .filter(|(at, owner)| self.within_life(at, *owner))
            .map(|(at, _)| at)
    }
}

#[derive(Clone, Debug, Default)]
pub struct InstanceLifecycleReplay {
    sets: BTreeMap<ScopeId, InstanceLifecycle>,
}

impl InstanceLifecycleReplay {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn declarations(
        &mut self,
        revision: circular_core::RevisionEpochId,
        declarations: &crate::run_graph::RunGraph,
    ) -> Result<(), InstanceTransitionRecordError> {
        let declared: BTreeMap<_, _> = declarations
            .actors()
            .iter()
            .map(|(actor, declaration)| {
                (
                    actor
                        .scope()
                        .append_segment(circular_plan::ScopeSeg::Child(actor.name().clone()))
                        .expect("a declared container scope"),
                    declaration,
                )
            })
            .collect();
        for (scope, life) in &mut self.sets {
            life.declare(revision, declared.get(scope).copied())?;
        }
        for (scope, declaration) in declared {
            if *declaration.domain().actor_type() == circular_core::ActorType::Replicator
                && !self.sets.contains_key(&scope)
            {
                self.sets
                    .entry(scope)
                    .or_default()
                    .declare(revision, Some(declaration))?;
            }
        }
        Ok(())
    }

    pub fn read(
        &mut self,
        record: &Record<ProductStore>,
    ) -> Result<bool, InstanceTransitionRecordError> {
        if let Record::Boundary(boundary) = record
            && let circular_store::BoundaryFact::Arrival {
                inlet: Some(inlet),
                body,
                ..
            } = boundary.fact()
            && inlet.as_str() == circular_actors::LIFECYCLE_PORT_NAME
        {
            let Some(payload) = body.payload() else {
                return Ok(false);
            };
            let value = circular_core::decode(
                payload.body(),
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
            )
            .map_err(|_| InstanceTransitionRecordError::PayloadNotCanonical)?;
            if !crate::kernel::turn::lifecycle(inlet, &value)
                .map_err(|_| InstanceTransitionRecordError::PayloadNotCanonical)?
                .is_some_and(|life| life.declares())
            {
                return Ok(false);
            }
            if let circular_plan::ActorId::Scoped {
                scope,
                local: circular_plan::LocalKey::Named(name),
            } = record.header().at().producer()
            {
                let scope = scope
                    .append_segment(circular_plan::ScopeSeg::Child(name.clone()))
                    .map_err(|_| InstanceTransitionRecordError::MalformedScope)?;
                if let Some(life) = self.sets.get_mut(&scope) {
                    life.arrive(record.header().at().revision());
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if let Some((actor, incarnation)) =
            crate::incarnation_transition::standing_incarnation(record)
                .map_err(|_| InstanceTransitionRecordError::PayloadNotCanonical)?
        {
            let scope = actor
                .scope()
                .append_segment(circular_plan::ScopeSeg::Child(actor.name().clone()))
                .map_err(|_| InstanceTransitionRecordError::MalformedScope)?;
            if let Some(life) = self.sets.get_mut(&scope) {
                if life
                    .declared
                    .is_some_and(|(_, began)| record.header().at().revision() >= began)
                {
                    life.incarnation(incarnation);
                    return Ok(true);
                }
            }
        }
        let Some(transition) = decode_transition(record)? else {
            return Ok(false);
        };
        let RecordOrigin::Actor(incarnation) = record.header().origin() else {
            unreachable!("validated transition owner")
        };
        self.sets.entry(transition.scope).or_default().transition(
            transition.disposition,
            transition.key,
            record.header().at().clone(),
            incarnation.get(),
        );
        Ok(true)
    }

    pub fn live(&self, scope: &ScopeId) -> impl Iterator<Item = &InstanceKey> {
        self.minted(scope).map(|(key, _)| key)
    }

    pub fn minted(
        &self,
        scope: &ScopeId,
    ) -> impl Iterator<Item = (&InstanceKey, &circular_core::Stamp<circular_plan::ActorId>)> {
        self.sets
            .get(scope)
            .into_iter()
            .flat_map(InstanceLifecycle::minted)
    }

    pub(crate) fn owned(&self, actor: &circular_plan::NamedActorId) -> InstanceLifecycle {
        let scope = actor
            .scope()
            .append_segment(circular_plan::ScopeSeg::Child(actor.name().clone()))
            .expect("a container scope");
        self.sets.get(&scope).cloned().unwrap_or_default()
    }

    #[must_use]
    pub fn count(&self, scope: &ScopeId) -> usize {
        self.minted(scope).count()
    }

    pub fn scopes(&self) -> impl Iterator<Item = &ScopeId> {
        self.sets.keys()
    }

    #[cfg(test)]
    fn fold(
        &mut self,
        disposition: InstanceDisposition,
        scope: ScopeId,
        key: InstanceKey,
        at: circular_core::Stamp<circular_plan::ActorId>,
    ) {
        self.sets
            .entry(scope)
            .or_default()
            .transition(disposition, key, at, 0);
    }
}

#[must_use]
pub fn instance_set_scope<Seal>(authority: &InstanceAuthority<Seal>) -> ScopeId {
    authority.instance_set_scope()
}

pub type InstanceCoordinate = (ScopeId, InstanceKey);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring_assembly::projection::AuthoredProjection;
    use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
    use circular_core::{Sequence, Stamp, Tick};
    use circular_plan::{Name, PipelineActorDecl, ScopeRole, ScopeSeg, admit_template};
    use circular_runtime::{InstanceGovernor, InstanceIntent, InstanceRegistry};
    use circular_store::{
        AppendBatch, AppendResult, Bound, MemoryStore, ObservationScope, PagePolicy, Query,
        ScanStart, Store,
    };
    use std::num::NonZeroUsize;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Seal {}

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn key(value: &str) -> InstanceKey {
        InstanceKey::Scalar(circular_runtime::InstanceScalar::normalized_text(value))
    }

    fn scope() -> ScopeId {
        ScopeId::from_segments(vec![
            ScopeSeg::Child(name("fleet")),
            ScopeSeg::Child(name("cell")),
        ])
        .expect("two segments")
    }

    fn fleet_plan() -> AuthoredProjection {
        fleet_generation(0)
    }

    fn fleet_generation(generation: u64) -> AuthoredProjection {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .enter_scope(name("fleet"), PipelineActorDecl::default())
            .expect("fleet scope");
        for template in ["cell", "shard"] {
            builder
                .enter_scope_with_container(
                    name(template),
                    circular_plan::ContainerActorDecl::replicator(
                        circular_plan::Config::default(),
                        circular_plan::ActorFlags::default(),
                        generation,
                    ),
                )
                .expect("template scope");
            builder.exit_scope().expect("leave template");
        }
        builder.exit_scope().expect("leave fleet");
        builder.finish().expect("fleet plan")
    }

    fn fleet_scope() -> ScopeId {
        ScopeId::from_segments(vec![ScopeSeg::Child(name("fleet"))]).expect("fleet scope")
    }

    fn authority(plan: &AuthoredProjection, template: &str) -> InstanceAuthority<Seal> {
        InstanceAuthority::granted(
            &admit_template(
                &crate::authoring_assembly::projection::scope_roles(plan),
                &fleet_scope(),
                &name(template),
            )
            .expect("admitted template"),
        )
    }

    fn stamp(sequence: u64) -> Stamp<circular_plan::ActorId> {
        Stamp::from_event_producer(
            Tick::new(sequence),
            circular_plan::NamedActorId::new(ScopeId::root(), name("fleet")),
            Sequence::new(sequence).expect("positive sequence"),
            circular_core::RevisionEpochId::new(1).expect("first revision"),
        )
    }

    fn step(
        registry: &mut InstanceRegistry<Seal>,
        authority: &InstanceAuthority<Seal>,
        intent: InstanceIntent,
        sequence: u64,
    ) -> (InstanceDisposition, Option<Record<ProductStore>>) {
        let key = intent.key().clone();
        let disposition = registry.apply(authority, intent);
        let record = instance_transition_record(
            disposition,
            instance_set_scope(authority),
            key,
            stamp(sequence),
            4,
        )
        .expect("canonical lifecycle carrier")
        .map(Record::Observation);
        (disposition, record)
    }

    #[test]
    fn lifecycle_occurrences_reproduce_the_live_set_at_every_prefix() {
        let plan = fleet_plan();
        let cells = authority(&plan, "cell");
        let mut registry = InstanceRegistry::<Seal>::new();
        let mut journal = Vec::new();
        let script = [
            InstanceIntent::Instantiate { key: key("a") },
            InstanceIntent::Instantiate { key: key("a") },
            InstanceIntent::Instantiate { key: key("b") },
            InstanceIntent::Retire { key: key("ghost") },
            InstanceIntent::Retire { key: key("a") },
            InstanceIntent::Instantiate { key: key("a") },
            InstanceIntent::Instantiate { key: key("c") },
            InstanceIntent::Retire { key: key("b") },
        ];

        for (index, intent) in script.into_iter().enumerate() {
            let (_, record) = step(
                &mut registry,
                &cells,
                intent,
                u64::try_from(index).expect("small index") + 1,
            );
            if let Some(record) = record {
                journal.push(record);
            }

            let mut replay = InstanceLifecycleReplay::new();
            replay
                .declarations(
                    stamp(1).revision(),
                    &crate::run_graph::flatten(&plan).unwrap(),
                )
                .unwrap();
            for record in &journal {
                assert!(replay.read(record).expect("valid lifecycle occurrence"));
            }
            assert_eq!(
                replay
                    .live(&instance_set_scope(&cells))
                    .cloned()
                    .collect::<Vec<_>>(),
                registry.live(&cells).cloned().collect::<Vec<_>>(),
                "prefix {index} diverged"
            );
        }
        assert_eq!(
            journal.len(),
            6,
            "idempotent and no-op intents write nothing"
        );
    }

    #[test]
    fn two_templates_under_one_container_keep_distinct_lifecycle_coordinates() {
        let plan = fleet_plan();
        let cells = authority(&plan, "cell");
        let shards = authority(&plan, "shard");
        assert_ne!(instance_set_scope(&cells), instance_set_scope(&shards));

        let mut registry = InstanceRegistry::<Seal>::new();
        let mut replay = InstanceLifecycleReplay::new();
        replay
            .declarations(
                stamp(1).revision(),
                &crate::run_graph::flatten(&plan).unwrap(),
            )
            .unwrap();
        for (authority, sequence) in [(&cells, 1), (&shards, 2)] {
            let (_, record) = step(
                &mut registry,
                authority,
                InstanceIntent::Instantiate { key: key("same") },
                sequence,
            );
            assert!(
                replay
                    .read(&record.expect("minted occurrence"))
                    .expect("valid occurrence")
            );
        }
        assert_eq!(replay.count(&instance_set_scope(&cells)), 1);
        assert_eq!(replay.count(&instance_set_scope(&shards)), 1);
        assert_eq!(replay.scopes().count(), 2);

        let (_, record) = step(
            &mut registry,
            &cells,
            InstanceIntent::Retire { key: key("same") },
            3,
        );
        assert!(
            replay
                .read(&record.expect("retired occurrence"))
                .expect("valid occurrence")
        );
        assert_eq!(replay.count(&instance_set_scope(&cells)), 0);
        assert_eq!(replay.count(&instance_set_scope(&shards)), 1);
    }

    #[test]
    fn instance_lifecycle_actor_events_round_trip_through_store_and_replay() {
        let events = [
            (InstanceDisposition::Minted, key("a"), 1),
            (InstanceDisposition::Minted, key("b"), 2),
            (InstanceDisposition::Retired, key("a"), 3),
        ]
        .into_iter()
        .map(|(disposition, key, sequence)| {
            Record::Observation(
                instance_transition_record(disposition, scope(), key, stamp(sequence), 4)
                    .expect("carrier")
                    .expect("set-changing transition"),
            )
        })
        .collect::<Vec<_>>();

        for event in &events {
            assert_eq!(
                event.header().at().producer(),
                &circular_plan::ActorId::from(circular_plan::NamedActorId::new(
                    ScopeId::root(),
                    name("fleet"),
                )),
                "instantiator is the producer"
            );
            assert!(matches!(
                event.header().origin(),
                RecordOrigin::Actor(found) if found.get() == 4
            ));
            assert!(circular_store::reencodes_identically(event));
        }

        let page = NonZeroUsize::new(16).expect("positive");
        let mut store =
            MemoryStore::<ProductStore>::new(PagePolicy::new(page, page).expect("valid policy"));
        assert!(matches!(
            store.append(AppendBatch::try_new(events).expect("non-empty")),
            AppendResult::Committed(_)
        ));
        store.seal_all();
        let recorded = store
            .query(&Query::ObservationScan {
                scope: ObservationScope::Kind(BuiltinObservationName::InstanceTransition),
                buckets: None,
                start: ScanStart::Beginning,
                upto: Bound::EndOfSealed,
            })
            .expect("transition read");
        assert_eq!(recorded.records().len(), 3);

        let mut replay = InstanceLifecycleReplay::new();
        replay
            .declarations(
                stamp(1).revision(),
                &crate::run_graph::flatten(&fleet_plan()).unwrap(),
            )
            .unwrap();
        for record in recorded.records() {
            assert!(replay.read(record).expect("valid actor event"));
        }
        assert_eq!(
            replay.live(&scope()).cloned().collect::<Vec<_>>(),
            vec![key("b")]
        );
    }

    #[test]
    fn ended_declaration_without_retired_never_restores_a_cell() {
        let mut replay = InstanceLifecycleReplay::new();
        let declared = crate::run_graph::flatten(&fleet_plan()).unwrap();
        replay.declarations(stamp(1).revision(), &declared).unwrap();
        replay.fold(InstanceDisposition::Minted, scope(), key("a"), stamp(1));
        replay.fold(InstanceDisposition::Minted, scope(), key("b"), stamp(2));
        assert_eq!(
            replay.live(&scope()).cloned().collect::<Vec<_>>(),
            vec![key("a"), key("b")]
        );

        let empty = AuthoredProjectionBuilder::new().finish().unwrap();
        replay
            .declarations(
                circular_core::RevisionEpochId::new(2).unwrap(),
                &crate::run_graph::flatten(&empty).unwrap(),
            )
            .unwrap();
        assert_eq!(replay.count(&scope()), 0);
        replay
            .declarations(circular_core::RevisionEpochId::new(3).unwrap(), &declared)
            .unwrap();
        assert_eq!(replay.minted(&scope()).count(), 0);
    }

    #[test]
    fn same_epoch_redeclaration_rejects_late_old_mints_without_retired() {
        let mut replay = InstanceLifecycleReplay::new();
        replay
            .declarations(
                stamp(1).revision(),
                &crate::run_graph::flatten(&fleet_plan()).unwrap(),
            )
            .unwrap();
        replay.fold(InstanceDisposition::Minted, scope(), key("a"), stamp(1));
        replay
            .declarations(
                circular_core::RevisionEpochId::new(2).unwrap(),
                &crate::run_graph::flatten(&fleet_generation(1)).unwrap(),
            )
            .unwrap();
        replay.fold(InstanceDisposition::Minted, scope(), key("b"), stamp(2));
        assert_eq!(replay.count(&scope()), 0);
        let minted = Stamp::from_event_producer(
            Tick::new(3),
            circular_plan::NamedActorId::new(fleet_scope(), name("cell")),
            Sequence::new(3).unwrap(),
            circular_core::RevisionEpochId::new(2).unwrap(),
        );
        replay.fold(InstanceDisposition::Minted, scope(), key("a"), minted);
        assert_eq!(
            replay.live(&scope()).cloned().collect::<Vec<_>>(),
            vec![key("a")]
        );
        replay
            .declarations(
                circular_core::RevisionEpochId::new(3).unwrap(),
                &crate::run_graph::flatten(&fleet_generation(1)).unwrap(),
            )
            .unwrap();
        assert_eq!(replay.count(&scope()), 1);
    }

    #[test]
    fn no_op_dispositions_do_not_create_lifecycle_facts() {
        for disposition in [
            InstanceDisposition::AlreadyLive,
            InstanceDisposition::NotLive,
            InstanceDisposition::RejectedCapacity { max: 2 },
        ] {
            assert!(
                instance_transition_record(disposition, scope(), key("a"), stamp(1), 0)
                    .expect("no carrier error")
                    .is_none()
            );
        }
    }

    #[test]
    fn a_run_owned_transition_is_rejected_by_the_closed_error_channel() {
        let at = stamp(1);
        let valid = instance_transition_record(
            InstanceDisposition::Minted,
            scope(),
            key("a"),
            at.clone(),
            4,
        )
        .expect("carrier")
        .expect("set-changing transition");
        let ObservationFact::Lifecycle(payload) = valid.fact() else {
            unreachable!("constructor makes a lifecycle occurrence")
        };
        let forged = Record::Observation(ObservationRecord::lifecycle(
            at,
            ObservationBucket::from_millis(1),
            RecordOrigin::Stream,
            ObservationItemKey::new(BuiltinObservationName::InstanceTransition, OpaqueId::new(0)),
            payload.clone(),
        ));
        assert_eq!(
            instance_transition_value(&forged),
            Err(InstanceTransitionRecordError::NotActorOwned)
        );
    }
}
