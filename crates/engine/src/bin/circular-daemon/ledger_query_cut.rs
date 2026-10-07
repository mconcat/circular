use super::*;
use circular_core::ArrivalIndex;
use circular_protocol::replay_payload::LogCut as WireCut;

struct QueryCut {
    from: BTreeMap<ActorId, u64>,
    through: BTreeMap<ActorId, u64>,
    cut: Value,
    folded_from: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadBound {
    extent: usize,
    arrivals: Arc<BTreeMap<ActorId, u64>>,
}

impl ReadBound {
    #[cfg(test)]
    pub(crate) fn at(end: usize) -> Self {
        Self {
            extent: end,
            arrivals: Arc::default(),
        }
    }

    pub(crate) const fn end(&self) -> usize {
        self.extent
    }

    pub(crate) const fn admits(&self, position: usize) -> bool {
        position < self.extent
    }

    pub(crate) fn arrival_end(&self, actor: &ActorId) -> u64 {
        self.arrivals.get(actor).copied().unwrap_or(0)
    }

    pub(crate) fn admits_arrival(&self, actor: &ActorId, ordinal: u64) -> bool {
        ordinal < self.arrival_end(actor)
    }

    pub(crate) fn within(&self, horizon: &Self) -> Self {
        let arrivals = self
            .arrivals
            .iter()
            .map(|(actor, end)| (actor.clone(), (*end).min(horizon.arrival_end(actor))))
            .collect();
        Self {
            extent: self.extent.min(horizon.extent),
            arrivals: Arc::new(arrivals),
        }
    }

    pub(crate) fn precedes_somewhere(&self, other: &Self) -> bool {
        self.extent < other.extent
            || other
                .arrivals
                .iter()
                .any(|(actor, end)| self.arrival_end(actor) < *end)
    }
}

fn ordinals_before(
    first_at: impl Fn(u64) -> Result<Option<usize>, String>,
    end: u64,
    limit: usize,
) -> Result<u64, String> {
    let (mut low, mut high) = (0_u64, end);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if first_at(middle - 1)?.is_some_and(|position| position < limit) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Ok(low)
}

impl ServerRead {
    pub(crate) fn read_bound(&self, cut: &circular_runtime::LogCut) -> Result<ReadBound, String> {
        let bounds: BTreeMap<ActorId, u64> = self
            .store
            .arrival_coordinate_bounds()?
            .into_iter()
            .collect();
        for (actor, index) in cut.components() {
            match bounds.get(actor) {
                Some(end) if index.get() <= *end => {}
                None if index.get() == 0 => {}
                _ => return Err("the cut is beyond this stream's recorded arrivals".to_owned()),
            }
        }
        let arrivals = bounds
            .keys()
            .map(|actor| {
                let at = cut.get(actor).map_or(0, circular_core::ArrivalIndex::get);
                (actor.clone(), at)
            })
            .collect();
        self.bound_of(arrivals)
    }

    fn bound_of(&self, arrivals: BTreeMap<ActorId, u64>) -> Result<ReadBound, String> {
        let mut extent = self.store.end();
        for (actor, from) in &arrivals {
            if let Some(position) = self.store.first_arrival_position(actor, *from)? {
                extent = extent.min(position);
            }
        }
        Ok(ReadBound {
            extent,
            arrivals: Arc::new(arrivals),
        })
    }

    pub(crate) fn record_horizon(&self) -> Result<ReadBound, String> {
        Ok(ReadBound {
            extent: self.store.end(),
            arrivals: Arc::new(
                self.store
                    .arrival_coordinate_bounds()?
                    .into_iter()
                    .collect(),
            ),
        })
    }

    pub(crate) fn bounded_cut(
        &self,
        bound: &ReadBound,
    ) -> Result<circular_runtime::LogCut, String> {
        Ok(circular_runtime::LogCut::new(
            self.store
                .arrival_coordinate_bounds()?
                .into_iter()
                .map(|(actor, end)| {
                    let at = end.min(bound.arrival_end(&actor));
                    (actor, ArrivalIndex::new(at))
                })
                .collect::<Vec<_>>(),
        ))
    }

    fn arrival_instant(
        &self,
        actor: &ActorId,
        ordinal: u64,
    ) -> Result<Option<(usize, RecordedInstant)>, String> {
        let Some(position) = self.store.first_arrival_position(actor, ordinal)? else {
            return Ok(None);
        };
        let row = self.store.record_row(position)?;
        Ok(crate::daemon::record_rows::arrival_envelope(&row)?
            .map(|arrival| (position, arrival.observed_at)))
    }

    pub(crate) fn next_arrival_after(
        &self,
        bound: &ReadBound,
        horizon: &ReadBound,
    ) -> Result<Option<(usize, RecordedInstant)>, String> {
        let mut next: Option<(usize, RecordedInstant)> = None;
        for (actor, end) in horizon.arrivals.iter() {
            let at = bound.arrival_end(actor);
            if at >= *end {
                continue;
            }
            if let Some(found) = self.arrival_instant(actor, at)?
                && next.is_none_or(|(_, instant)| found.1 < instant)
            {
                next = Some(found);
            }
        }
        Ok(next)
    }

    pub(crate) fn advance_bound(
        &self,
        bound: &ReadBound,
        horizon: &ReadBound,
        until: RecordedInstant,
    ) -> Result<ReadBound, String> {
        let mut arrivals = (*bound.arrivals).clone();
        for (actor, end) in horizon.arrivals.iter() {
            let mut at = bound.arrival_end(actor);
            while at < *end {
                match self.arrival_instant(actor, at)? {
                    Some((_, instant)) if instant <= until => at += 1,
                    _ => break,
                }
            }
            arrivals.insert(actor.clone(), at);
        }
        let advanced = self.bound_of(arrivals)?;
        Ok(ReadBound {
            extent: advanced.extent.max(bound.extent).min(horizon.extent),
            arrivals: advanced.arrivals,
        })
    }

    fn arrival_positions_within(
        &self,
        actor: &ActorId,
        from: u64,
        bound: Option<&ReadBound>,
    ) -> Result<Vec<usize>, String> {
        let mut positions = self.store.arrival_positions_since(actor, from)?;
        if let Some(bound) = bound {
            let admitted = bound.arrival_end(actor).saturating_sub(from);
            positions.truncate(usize::try_from(admitted).unwrap_or(usize::MAX));
        }
        Ok(positions)
    }

    pub(crate) fn arrival_revision_within(
        &self,
        bound: &ReadBound,
    ) -> Result<Option<RevisionEpochId>, String> {
        let mut latest = None;
        for (actor, end) in bound.arrivals.iter() {
            if *end == 0 {
                continue;
            }
            for position in self.arrival_positions_within(actor, 0, Some(bound))? {
                let row = self.store.record_row(position)?;
                let revision = crate::daemon::record_rows::with_envelope(&row, |envelope| {
                    match envelope.origin {
                        circular_store::EnvelopeOrigin::Stamped { revision, .. } => Some(revision),
                        circular_store::EnvelopeOrigin::OperationCoordinate { .. } => None,
                    }
                })?
                .flatten()
                .unwrap_or_else(|| row.record().header().at().revision());
                latest = latest.max(Some(revision));
            }
        }
        Ok(latest)
    }

    fn query_cut(&self, since: Option<&WireCut>, display: bool) -> Result<QueryCut, String> {
        self.query_cut_within(since, display, None)
    }

    fn query_cut_within(
        &self,
        since: Option<&WireCut>,
        display: bool,
        bound: Option<&ReadBound>,
    ) -> Result<QueryCut, String> {
        let mut through: BTreeMap<_, _> = self
            .stood_actors
            .iter()
            .map(|actor| (actor.as_actor_id(), 0))
            .collect();
        if display {
            for (actor, index) in self.store.display_coordinate_bounds()? {
                through
                    .entry(actor)
                    .and_modify(|end| *end = (*end).max(index))
                    .or_insert(index);
            }
        } else {
            through.extend(self.store.arrival_coordinate_bounds()?);
        }
        if let Some(bound) = bound {
            for (actor, end) in &mut through {
                *end = if display {
                    self.store
                        .display_positions_since(actor, 0)?
                        .into_iter()
                        .filter(|position| bound.admits(*position))
                        .count() as u64
                } else {
                    (*end).min(bound.arrival_end(actor))
                };
            }
        }
        let mut from = BTreeMap::new();
        let mut invalid = false;
        if let Some(since) = since {
            for component in &since.components {
                let actor =
                    circular_runtime::product_identity::named_actor_from_wire(&component.actor)
                        .map_err(|error| format!("query cut actor: {error:?}"))?
                        .as_actor_id();
                if through.get(&actor).is_none_or(|end| component.index > *end) {
                    invalid = true;
                }
                from.insert(actor, component.index);
            }
        }
        if invalid {
            from.clear();
        }
        let cut = crate::daemon::replay::replay_wire_cut(&circular_runtime::LogCut::new(
            through
                .iter()
                .map(|(actor, index)| (actor.clone(), ArrivalIndex::new(*index))),
        ))?
        .to_value()
        .map_err(|error| format!("query cut: {error:?}"))?;
        Ok(QueryCut {
            from,
            through,
            cut,
            folded_from: invalid.then(|| Value::Array(Vec::new())),
        })
    }

    pub(crate) fn rollup_within(
        &self,
        since: Option<&WireCut>,
        bound: Option<&ReadBound>,
    ) -> Result<circular_protocol::declaration_payload::QueryPage, String> {
        let cut = self.query_cut_within(since, true, bound)?;

        let mut positions = Vec::new();
        for actor in cut.through.keys() {
            positions.extend(
                self.store
                    .display_positions_since(actor, cut.from.get(actor).copied().unwrap_or(0))?
                    .into_iter()
                    .filter(|position| bound.is_none_or(|bound| bound.admits(*position))),
            );
        }
        let recorded_keys = self.store.recorded_display_keys()?;
        let key_order: std::collections::HashMap<_, _> = recorded_keys
            .iter()
            .enumerate()
            .map(|(index, key)| (key, index))
            .collect();
        let mut entries = Vec::new();
        for position in positions {
            let record = self.store.row(position)?;
            let Record::Display(display) = &*record else {
                unreachable!("display position")
            };
            let ClassKey::Display { key, .. } = display.header().key() else {
                unreachable!("display key")
            };
            let Some(order) = key_order.get(key) else {
                continue;
            };
            let body = circular_core::decode(
                display.payload().body(),
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
            )
            .map_err(|error| format!("failed to decode display payload: {error:?}"))?;
            let subject = key.args();
            let actor = circular_store::named_actor_value(subject)
                .map_err(|error| format!("display subject: {error:?}"))?;
            let value = display_frame_value(key, display.header().at(), actor, body)
                .ok_or("display key exceeds the published frame carrier")?;
            let cell = subject
                .scope()
                .segments()
                .iter()
                .any(|segment| matches!(segment, circular_runtime::ScopeSeg::Instance { .. }));
            entries.push((*order, position, cell, value));
        }
        entries.sort_by_key(|(key, position, _, _)| (*key, *position));
        let (mut rollup, mut cells) = (Vec::new(), Vec::new());
        for (_, _, cell, value) in entries {
            if cell {
                cells.push(value);
            } else {
                rollup.push(value);
            }
        }
        Ok(circular_protocol::declaration_payload::QueryPage {
            anchor: Value::Null,
            items: vec![Value::array([Value::Array(rollup), Value::Array(cells)])],
            terminal: circular_protocol::declaration_payload::Terminal::Complete,
            reached: None,
            cut: Some(cut.cut),
            folded_from: cut.folded_from,
        })
    }
}

use super::projection::{ProjectionRecord, ProjectionSnapshot, ProjectionTail};

struct ArrivalPageTail {
    store: circular_store::JournalView,
    positions: std::vec::IntoIter<(usize, Option<Role>)>,
}
impl ProjectionTail for ArrivalPageTail {
    fn fill(&mut self, rows: &mut Vec<ProjectionRecord>, limit: usize) -> Result<bool, String> {
        while rows.len() < limit {
            let Some((position, role)) = self.positions.next() else {
                return Ok(true);
            };
            let record = self.store.row(position)?;
            if let Some(role) = role {
                let Record::Boundary(boundary) = &*record else {
                    unreachable!("arrival position")
                };
                rows.push(ProjectionRecord::new(boundary, role)?);
            } else if let Some(body) = actor_event_value(&record) {
                rows.push(ProjectionRecord::recorded(
                    record_witness(record.header())?,
                    body,
                ));
            }
        }
        Ok(false)
    }
}

impl ServerRead {
    pub(crate) fn actor_events_within(
        &self,
        since: Option<&WireCut>,
        bound: Option<&ReadBound>,
    ) -> Result<ProjectionSnapshot, String> {
        let cut = self.query_cut_within(since, false, bound)?;
        let mut positions = Vec::new();
        for (actor, through) in &cut.through {
            let from = cut.from.get(actor).copied().unwrap_or(0);
            positions.extend(self.arrival_positions_within(actor, from, bound)?);
            positions.extend(
                self.store
                    .emission_positions_between(actor, from, *through)?,
            );
        }
        positions.sort_unstable();
        let mut snapshot = ProjectionSnapshot::recorded(
            Value::UInt(self.anchor_mark()?.get()),
            Vec::new(),
            Vec::new(),
        );
        snapshot.records_tail = Some(Box::new(ArrivalPageTail {
            store: self.store.clone(),
            positions: positions
                .into_iter()
                .map(|position| (position, None))
                .collect::<Vec<_>>()
                .into_iter(),
        }));
        snapshot.cut = Some(cut.cut);
        snapshot.folded_from = cut.folded_from;
        Ok(snapshot)
    }

    #[cfg(test)]
    pub(crate) fn arrivals_since(
        &self,
        mount: &str,
        since: Option<&WireCut>,
    ) -> Result<ProjectionSnapshot, String> {
        self.arrivals_within(mount, since, None)
    }

    #[cfg(test)]
    pub(crate) fn arrivals_at_mount(&self, mount: &str) -> Result<Vec<Value>, String> {
        self.arrivals_since(mount, None)?
            .filled()
            .records
            .iter()
            .map(|record| record.value())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn arrival_digests_at_mount(&self, mount: &str) -> Result<Vec<Value>, String> {
        Ok(self
            .arrivals_at_mount(mount)?
            .iter()
            .map(arrival_digest)
            .collect())
    }

    pub(crate) fn arrivals_within(
        &self,
        mount: &str,
        since: Option<&WireCut>,
        upto: Option<&ReadBound>,
    ) -> Result<ProjectionSnapshot, String> {
        let cut = self.query_cut_within(since, false, upto)?;
        let name = ExportName::new(circular_plan::Name::from_normalized(mount));
        let export = self
            .plan
            .exports()
            .get(&name)
            .ok_or_else(|| format!("this plan's export section has no `{mount}`"))?;
        let mut selected = Vec::new();
        let mut has_role = false;
        for role in [Role::Request, Role::Result, Role::Progress, Role::Error] {
            let Some(bound) = export.roles().get(&role) else {
                continue;
            };
            has_role = true;
            let actors = if role == Role::Request {
                request_mount_actors(&self.plan, bound.actor(), bound.port())
            } else {
                observation_mount_actor(&self.plan, bound.actor(), bound.port())
                    .map(|actor| vec![actor])
            }
            .map_err(|_| "failed to resolve the boundary leaf validated during activation")?;
            for actor in actors {
                for actor in self.recorded_observation_actors(actor)? {
                    let actor = actor.as_actor_id();
                    let mut column = Vec::new();
                    for position in self.arrival_positions_within(
                        &actor,
                        cut.from.get(&actor).copied().unwrap_or(0),
                        upto,
                    )? {
                        let record = self.store.row(position)?;
                        let effect_first = matches!(record.header().key(), ClassKey::Boundary(BoundaryKey::Arrival { origin, .. })
                            if matches!(origin.as_ref(), circular_store::ArrivalKey::EffectOutcome { .. }));
                        column.push((!effect_first, position));
                    }
                    column.sort_unstable();
                    selected.extend(
                        column
                            .into_iter()
                            .map(|(_, position)| (position, Some(role))),
                    );
                }
            }
        }
        if !has_role {
            return Err(format!("export `{mount}` has no roles"));
        }
        let revisions = self.revision_positions()?;
        let (revision, _) = revisions.last().ok_or("journal has no GraphRevision")?;
        let anchor = Value::object([
            ("lifecycle", self.lifecycle_anchor()),
            ("revision_epoch", Value::UInt(revision.get())),
            ("cut", cut.cut.clone()),
        ])
        .map_err(|error| format!("arrival anchor: {error:?}"))?;
        let mut delivered = BTreeMap::new();
        let mut emitted = 0_usize;
        let mut rows = Vec::with_capacity(selected.len());
        for (position, role) in &selected {
            let record = self.store.row(*position)?;
            if let Record::Boundary(row) = &*record
                && let ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }) = row.header().key()
                && let BoundaryFact::Arrival {
                    arrival_index,
                    inlet,
                    ..
                } = row.fact()
            {
                if !inlet
                    .as_ref()
                    .is_some_and(|port| port.as_str() == circular_actors::LIFECYCLE_PORT_NAME)
                {
                    emitted += 1;
                    rows.push((*position, *role));
                }
                delivered
                    .entry((actor.clone(), arrival_index.get()))
                    .or_insert(emitted);
            }
        }
        let mut prefixes = BTreeMap::new();
        for (actor, end) in &cut.through {
            let base = cut.from.get(actor).copied().unwrap_or(0);
            let mut required = vec![0];
            for index in base..*end {
                let Some(position) = delivered.get(&(actor.clone(), index)) else {
                    break;
                };
                required.push((*required.last().expect("zero prefix")).max(*position));
            }
            prefixes.insert(actor.clone(), (base, required));
        }
        let reached = self
            .query_timeline(&cut)?
            .into_iter()
            .filter_map(|(_, point)| {
                let required =
                    point
                        .cut()
                        .components()
                        .try_fold(0, |maximum, (actor, index)| {
                            let (base, prefix) = prefixes.get(actor)?;
                            if index.get() <= *base {
                                return Some(maximum);
                            }
                            prefix
                                .get(usize::try_from(index.get() - base).ok()?)
                                .map(|position| maximum.max(*position))
                        })?;
                Some(
                    crate::daemon::replay::replay_wire_cut(point.cut()).and_then(|wire| {
                        let value = wire
                            .to_value()
                            .map_err(|error| format!("reached cut: {error:?}"))?;
                        Value::object([
                            ("revision_epoch", Value::UInt(point.revision().get())),
                            ("cut", value),
                        ])
                        .map(|value| (required, value))
                        .map_err(|error| format!("reached target: {error:?}"))
                    }),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut snapshot = ProjectionSnapshot::recorded(anchor, Vec::new(), reached);
        snapshot.records_tail = Some(Box::new(ArrivalPageTail {
            store: self.store.clone(),
            positions: rows.into_iter(),
        }));
        snapshot.cut = Some(cut.cut);
        snapshot.folded_from = cut.folded_from;
        Ok(snapshot)
    }
}

struct TimelineEpoch {
    revision: RevisionEpochId,
    activation: Option<(usize, RecordedInstant, circular_runtime::LogCut)>,
    counted: BTreeMap<ActorId, u64>,
    groups: Vec<(RecordedInstant, usize, Vec<(ActorId, u64)>)>,
}

struct TimelinePoints {
    epochs: std::vec::IntoIter<TimelineEpoch>,
    current: Option<TimelineEpoch>,
    group: usize,
}

impl TimelinePoints {
    fn new(epochs: Vec<TimelineEpoch>) -> Self {
        Self {
            epochs: epochs.into_iter(),
            current: None,
            group: 0,
        }
    }
}

impl Iterator for TimelinePoints {
    type Item = (usize, ServerReplayCheckpoint);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let Some(epoch) = self.current.as_mut() else {
                let mut epoch = self.epochs.next()?;
                self.group = 0;
                let activation = epoch.activation.take();
                let revision = epoch.revision;
                self.current = Some(epoch);
                if let Some((position, at, cut)) = activation {
                    return Some((position, ServerReplayCheckpoint { revision, at, cut }));
                }
                continue;
            };
            let Some((at, position, advanced)) = epoch.groups.get(self.group) else {
                self.current = None;
                continue;
            };
            self.group += 1;
            for (actor, n) in advanced {
                epoch.counted.insert(actor.clone(), *n);
            }
            return Some((
                *position,
                ServerReplayCheckpoint {
                    revision: epoch.revision,
                    at: *at,
                    cut: circular_runtime::LogCut::new(
                        epoch
                            .counted
                            .iter()
                            .map(|(actor, n)| (actor.clone(), ArrivalIndex::new(*n))),
                    ),
                },
            ));
        }
    }
}

struct TimelineTail {
    store: circular_store::JournalView,
    points: TimelinePoints,
    stream: u64,
}
impl ProjectionTail for TimelineTail {
    fn fill(&mut self, rows: &mut Vec<ProjectionRecord>, limit: usize) -> Result<bool, String> {
        while rows.len() < limit {
            let Some((position, point)) = self.points.next() else {
                return Ok(true);
            };
            rows.push(ProjectionRecord::checkpoint(
                crate::daemon::record_rows::witness(&self.store.record_row(position)?)?,
                point,
                self.stream,
            ));
        }
        Ok(false)
    }
}

impl ServerRead {
    fn query_timeline(
        &self,
        cut: &QueryCut,
    ) -> Result<Vec<(usize, ServerReplayCheckpoint)>, String> {
        Ok(TimelinePoints::new(self.timeline_epochs(cut)?).collect())
    }

    pub(super) fn revision_positions(&self) -> Result<Vec<(RevisionEpochId, usize)>, String> {
        let mut revisions = Vec::new();
        for position in self.store.sealed_structure_positions()? {
            let record = self.store.row(position)?;
            if let Record::Structure(row) = &*record
                && matches!(row.fact(), StructureFact::GraphRevision(_))
            {
                revisions.push((row.header().at().revision(), position));
            }
        }
        Ok(revisions)
    }

    fn epoch_positions(
        revisions: &[(RevisionEpochId, usize)],
        index: usize,
    ) -> (usize, Option<usize>) {
        (
            revisions[index].1,
            revisions.get(index + 1).map(|(_, position)| *position),
        )
    }

    fn arrivals_before(
        &self,
        actor: &ActorId,
        recorded: u64,
        position: usize,
    ) -> Result<u64, String> {
        ordinals_before(
            |ordinal| self.store.first_arrival_position(actor, ordinal),
            recorded,
            position,
        )
    }

    pub(crate) fn replay_checkpoint_at(
        &self,
        revision: RevisionEpochId,
        cut: circular_runtime::LogCut,
    ) -> Result<circular_runtime::LogCut, String> {
        let revisions = self.revision_positions()?;
        let index = revisions
            .iter()
            .position(|(recorded, _)| *recorded == revision)
            .ok_or_else(|| format!("unknown revision epoch {}", revision.get()))?;
        let (opened, closed) = Self::epoch_positions(&revisions, index);
        let recorded: BTreeMap<ActorId, u64> = self
            .store
            .arrival_coordinate_bounds()?
            .into_iter()
            .collect();
        for (actor, at) in cut.components() {
            if at.get() > recorded.get(actor).copied().unwrap_or(0) {
                return Err(format!(
                    "revision replay target is invalid: actor `{actor}` at {} is beyond this stream's recorded arrivals",
                    at.get()
                ));
            }
        }
        for (actor, &end) in &recorded {
            let at = cut.get(actor).map_or(0, ArrivalIndex::get);
            let start = self.arrivals_before(actor, end, opened)?;
            let finish = match closed {
                Some(closed) => self.arrivals_before(actor, end, closed)?,
                None => end,
            };
            if at < start || at > finish {
                return Err(format!(
                    "revision replay target is invalid: actor `{actor}` at {at} is outside revision {}, which spans {start} to {finish}",
                    revision.get()
                ));
            }
        }
        Ok(cut)
    }

    fn timeline_epochs(&self, cut: &QueryCut) -> Result<Vec<TimelineEpoch>, String> {
        let mut arrivals: BTreeMap<ActorId, Vec<(u64, RecordedInstant, usize)>> = BTreeMap::new();
        for actor in cut.through.keys() {
            let ActorId::Scoped {
                local: circular_plan::LocalKey::Named(_),
                ..
            } = actor
            else {
                continue;
            };
            let mut column = Vec::new();
            for position in self
                .store
                .arrival_positions_since(actor, cut.from.get(actor).copied().unwrap_or(0))?
            {
                if let Some(arrival) =
                    crate::daemon::record_rows::arrival_envelope(&self.store.record_row(position)?)?
                {
                    column.push((arrival.index, arrival.observed_at, position));
                }
            }
            arrivals.insert(actor.clone(), column);
        }
        let revisions = self.revision_positions()?;
        let mut planned = Vec::new();
        let instants = self.revision_instants(&revisions)?;
        for (epoch_index, (revision, _)) in revisions.iter().enumerate() {
            let (opened, closed) = Self::epoch_positions(&revisions, epoch_index);
            let mut counted = BTreeMap::new();
            let mut ordered = Vec::new();
            let mut activation_cut = BTreeMap::new();
            for (actor, column) in &arrivals {
                let recorded = cut.through[actor];
                let start = self.arrivals_before(actor, recorded, opened)?;
                let end = match closed {
                    Some(closed) => self.arrivals_before(actor, recorded, closed)?,
                    None => recorded,
                };
                if start > 0 {
                    activation_cut.insert(actor.clone(), start);
                }
                let from = start
                    .max(cut.from.get(actor).copied().unwrap_or(0))
                    .min(end);
                if activation_cut.contains_key(actor) || from > 0 {
                    counted.insert(actor.clone(), from);
                }
                let lower = column.partition_point(|(index, _, _)| *index < from);
                for &(index, instant, position) in &column[lower..] {
                    if index >= end {
                        break;
                    }
                    ordered.push((instant, actor.clone(), index, position));
                }
            }
            let activation = if cut.from.is_empty()
                || activation_cut
                    .iter()
                    .any(|(actor, index)| *index > cut.from.get(actor).copied().unwrap_or(0))
            {
                Some((
                    opened,
                    RecordedInstant::from_millis(instants[epoch_index]),
                    circular_runtime::LogCut::new(
                        activation_cut
                            .iter()
                            .map(|(actor, index)| (actor.clone(), ArrivalIndex::new(*index))),
                    ),
                ))
            } else {
                None
            };
            ordered.sort_by(|a, b| (&a.0, &a.1, a.2).cmp(&(&b.0, &b.1, b.2)));
            let mut prefixes = counted
                .iter()
                .map(|(actor, from)| (actor.clone(), ObservedPrefix::from(*from)))
                .collect::<BTreeMap<_, _>>();
            let mut groups = Vec::new();
            let mut index = 0;
            while index < ordered.len() {
                let instant = ordered[index].0;
                let mut position = ordered[index].3;
                let mut advanced = BTreeMap::new();
                while index < ordered.len() && ordered[index].0 == instant {
                    let (_, actor, ordinal, source) = &ordered[index];
                    let prefix = prefixes
                        .entry(actor.clone())
                        .or_insert_with(|| ObservedPrefix::from(0));
                    if prefix.observe(*ordinal) {
                        advanced.insert(actor.clone(), prefix.len());
                    }
                    position = *source;
                    index += 1;
                }
                if !advanced.is_empty() {
                    groups.push((instant, position, advanced.into_iter().collect()));
                }
            }
            planned.push(TimelineEpoch {
                revision: *revision,
                activation,
                counted,
                groups,
            });
        }
        Ok(planned)
    }

    pub(crate) fn timeline_since(
        &self,
        since: Option<&WireCut>,
    ) -> Result<ProjectionSnapshot, String> {
        let cut = self.query_cut(since, false)?;
        let epochs = self.timeline_epochs(&cut)?;
        let checkpoints = epochs
            .iter()
            .map(|epoch| usize::from(epoch.activation.is_some()) + epoch.groups.len())
            .sum::<usize>();
        let revision = epochs
            .last()
            .ok_or("journal has no GraphRevision")?
            .revision;
        let availability = Value::array([
            Value::Int(1),
            Value::object([
                ("latest_revision_epoch", Value::UInt(revision.get())),
                ("source_checkpoints", Value::UInt(checkpoints as u64)),
            ])
            .map_err(|error| format!("timeline availability: {error:?}"))?,
        ]);
        let anchor = Value::object([("epoch_plans", availability)])
            .map_err(|error| format!("timeline anchor: {error:?}"))?;
        let mut snapshot = ProjectionSnapshot::recorded(anchor, Vec::new(), Vec::new());
        snapshot.records_tail = Some(Box::new(TimelineTail {
            store: self.store.clone(),
            points: TimelinePoints::new(epochs),
            stream: self.run.get(),
        }));
        snapshot.cut = Some(cut.cut);
        snapshot.folded_from = cut.folded_from;
        Ok(snapshot)
    }
}
