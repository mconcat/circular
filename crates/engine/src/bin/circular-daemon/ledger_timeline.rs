use super::*;
use circular_protocol::declaration_payload::PlanActorKey;
use circular_protocol::timeline::{
    TimelineAt, TimelineBin, TimelineBins, TimelineBinsArgs, TimelineMark, TimelineMarkKind,
};
use circular_store::ObservationFact;

struct Column {
    actor: ActorId,
    first: u64,
    end: u64,
    positions: Vec<usize>,
}

impl Column {
    const fn prefix(&self, within: usize) -> u64 {
        if within == 0 {
            0
        } else {
            self.first + within as u64
        }
    }

    fn before_position(&self, limit: usize) -> u64 {
        self.prefix(self.positions.partition_point(|position| *position < limit))
    }
}

#[derive(Clone, Copy)]
struct Step {
    ordinal: u64,
    envelope: u64,
    regressed: bool,
}

enum Boundary {
    Position(usize),
}

fn bin_of(at: u64, from: u64, covered: u64, bin_ms: u64) -> Option<usize> {
    (from..covered)
        .contains(&at)
        .then(|| usize::try_from((at - from) / bin_ms).expect("bins fit in usize"))
}

impl ServerRead {
    fn timeline_columns(&self) -> Result<Vec<Column>, String> {
        let mut columns = Vec::new();
        for (actor, end) in self.store.arrival_coordinate_bounds()? {
            if !matches!(
                &actor,
                ActorId::Scoped {
                    local: circular_plan::LocalKey::Named(_),
                    ..
                }
            ) {
                continue;
            }
            let positions = self.store.arrival_positions_since(&actor, 0)?;
            let Some(&head) = positions.first() else {
                continue;
            };
            let (first, _) = self.arrival_head(head)?;
            columns.push(Column {
                actor,
                first,
                end,
                positions,
            });
        }
        Ok(columns)
    }

    fn arrival_head(&self, position: usize) -> Result<(u64, u64), String> {
        let row = self.store.record_row(position)?;
        let arrival = crate::daemon::record_rows::arrival_envelope(&row)?
            .ok_or_else(|| format!("journal position {position} is not an arrival"))?;
        Ok((arrival.index, arrival.observed_at.millis()))
    }

    fn walk(&self, column: &Column, mut step: impl FnMut(Step) -> bool) -> Result<(), String> {
        let mut envelope = None::<u64>;
        for &position in &column.positions {
            let (ordinal, observed) = self.arrival_head(position)?;
            let regressed = envelope.is_some_and(|before| observed < before);
            let next = envelope.map_or(observed, |before| before.max(observed));
            envelope = Some(next);
            if !step(Step {
                ordinal,
                envelope: next,
                regressed,
            }) {
                break;
            }
        }
        Ok(())
    }

    fn envelope_at(&self, column: &Column, prefix: u64) -> Result<Option<u64>, String> {
        let mut found = None;
        self.walk(column, |step| {
            if step.ordinal >= prefix {
                return false;
            }
            found = Some(step.envelope);
            true
        })?;
        Ok(found)
    }

    fn boundary_prefix(&self, column: &Column, boundary: &Boundary) -> Result<u64, String> {
        match boundary {
            Boundary::Position(position) => Ok(column.before_position(*position)),
        }
    }

    fn walk_boundaries(
        &self,
        column: &Column,
        boundaries: &[Boundary],
        instants: &mut [u64],
        mut visit: impl FnMut(Step) -> bool,
    ) -> Result<(), String> {
        let mut wanted = BTreeMap::<u64, Vec<usize>>::new();
        for (index, boundary) in boundaries.iter().enumerate() {
            let prefix = self.boundary_prefix(column, boundary)?;
            if prefix > 0 {
                wanted.entry(prefix).or_default().push(index);
            }
        }
        let last_wanted = wanted.keys().next_back().copied().unwrap_or(0);
        self.walk(column, |step| {
            if let Some(marks) = wanted.get(&(step.ordinal + 1)) {
                for &mark in marks {
                    instants[mark] = instants[mark].max(step.envelope);
                }
            }
            visit(step) || step.ordinal + 1 < last_wanted
        })
    }

    pub(super) fn revision_instants(
        &self,
        revisions: &[(RevisionEpochId, usize)],
    ) -> Result<Vec<u64>, String> {
        let boundaries = revisions
            .iter()
            .map(|(_, position)| Boundary::Position(*position))
            .collect::<Vec<_>>();
        let mut instants = vec![0; boundaries.len()];
        for column in self.timeline_columns()? {
            self.walk_boundaries(&column, &boundaries, &mut instants, |_| false)?;
        }
        Ok(instants)
    }

    pub(crate) fn timeline_bins(&self, args: &TimelineBinsArgs) -> Result<TimelineBins, String> {
        let (bin_ms, covered) = args.coverage().map_err(|rejection| rejection.to_string())?;
        let from = args.from_ms;
        let scope = args
            .actor
            .as_ref()
            .map(|key| {
                circular_runtime::product_identity::named_actor_from_wire(key)
                    .map(|actor| actor.as_actor_id())
                    .map_err(|error| format!("timeline actor: {error:?}"))
            })
            .transpose()?;
        let columns = self.timeline_columns()?;
        if let Some(actor) = &scope
            && !columns.iter().any(|column| &column.actor == actor)
            && !self
                .stood_actors
                .iter()
                .any(|stood| &stood.as_actor_id() == actor)
        {
            return Err(format!(
                "stream {} has no actor `{actor}`: it neither stands nor recorded an arrival",
                self.run.get()
            ));
        }

        let sources = self
            .revision_positions()?
            .into_iter()
            .map(|(_, position)| (TimelineMarkKind::Edit, Boundary::Position(position)))
            .collect::<Vec<_>>();
        let (kinds, boundaries): (Vec<_>, Vec<_>) = sources.into_iter().unzip();
        let mut instants = vec![0_u64; boundaries.len()];

        let mut bins = vec![TimelineBin::default(); usize::try_from(args.bins).unwrap_or(0)];
        let mut clock_regressions = 0_u64;
        for column in &columns {
            let counted = scope.as_ref().is_none_or(|actor| actor == &column.actor);
            self.walk_boundaries(column, &boundaries, &mut instants, |step| {
                if counted && let Some(bin) = bin_of(step.envelope, from, covered, bin_ms) {
                    bins[bin].count += 1;
                    clock_regressions += u64::from(step.regressed);
                }
                counted && step.envelope < covered
            })?;
        }

        let (incidents, restarts) =
            self.timeline_observations(scope.as_ref(), args.actor.as_ref())?;
        for at in incidents {
            if let Some(bin) = bin_of(at, from, covered, bin_ms) {
                bins[bin].incidents += 1;
            }
        }
        let mut marks = kinds
            .iter()
            .zip(&instants)
            .map(|(kind, &at_ms)| TimelineMark { kind: *kind, at_ms })
            .chain(
                lifecycle_marks(&self.store)?
                    .into_iter()
                    .map(|(kind, at_ms)| TimelineMark { kind, at_ms }),
            )
            .chain(restarts.into_iter().map(|at_ms| TimelineMark {
                kind: TimelineMarkKind::Restart,
                at_ms,
            }))
            .filter(|mark| bin_of(mark.at_ms, from, covered, bin_ms).is_some())
            .collect::<Vec<_>>();
        marks.sort_by_key(|mark| (mark.at_ms, mark.kind as u8));
        let mut seen = BTreeSet::new();
        marks.retain(|mark| {
            seen.insert((bin_of(mark.at_ms, from, covered, bin_ms), mark.kind as u8))
        });
        Ok(TimelineBins {
            from_ms: from,
            to_ms: covered,
            bin_ms,
            bins,
            marks,
            clock_regressions,
        })
    }

    fn timeline_observations(
        &self,
        actor: Option<&ActorId>,
        key: Option<&PlanActorKey>,
    ) -> Result<(Vec<u64>, Vec<u64>), String> {
        let mut incidents = Vec::new();
        let mut restarts = Vec::new();
        for row in self.store.sealed_rows()? {
            let row = row?;
            if !crate::daemon::record_rows::keeps(&row, |envelope| {
                envelope.class_key_tag == 1
                    && matches!(
                        crate::daemon::record_rows::observation_kind(envelope),
                        Some(
                            BuiltinObservationName::DeadLetterEntry
                                | BuiltinObservationName::DiagnosticOccurrence
                                | BuiltinObservationName::Restart
                        )
                    )
            })? {
                continue;
            }
            let record = crate::daemon::record_rows::rebuild(&row);
            let Record::Observation(observation) = &*record else {
                continue;
            };
            let ClassKey::Observation(ObservationKey::StreamItem(at, bucket, item)) =
                observation.header().key()
            else {
                continue;
            };
            match (item.kind(), observation.fact()) {
                (BuiltinObservationName::DeadLetterEntry, ObservationFact::DeadLetter(_)) => {
                    if actor.is_none_or(|actor| actor == at.producer()) {
                        incidents.push(bucket.millis());
                    }
                }
                (
                    BuiltinObservationName::DiagnosticOccurrence,
                    ObservationFact::Diagnostic(payload),
                ) => {
                    let body = circular_core::decode(
                        payload.body(),
                        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                    )
                    .map_err(|error| format!("timeline diagnostic body: {error:?}"))?;
                    if !matches!(
                        body.as_object().and_then(|fields| fields.get("kind")),
                        Some(Value::String(kind)) if kind == engine::ACTOR_HEALTH_TRANSITION_KIND
                    ) {
                        continue;
                    }
                    let transition =
                        circular_protocol::actor_events::decode_actor_health_transition(body)
                            .map_err(|error| format!("timeline health transition: {error:?}"))?
                            .ok_or("recorded diagnostic is not a health transition")?;
                    if transition.state == ActorHealthState::Failed
                        && key.is_none_or(|key| key == &transition.actor)
                    {
                        incidents.push(bucket.millis());
                    }
                }
                (BuiltinObservationName::Restart, ObservationFact::Restart(_)) => {
                    restarts.push(at.physical_time().get());
                }
                _ => {}
            }
        }
        Ok((incidents, restarts))
    }

    pub(crate) fn timeline_at(&self, at_ms: u64) -> Result<TimelineAt, String> {
        let revisions = self.revision_positions()?;
        let columns = self.timeline_columns()?;
        let mut components = Vec::with_capacity(columns.len());
        for column in &columns {
            let mut component = column.end;
            let mut envelope = None;
            let mut crossed = false;
            self.walk(column, |step| {
                if step.envelope > at_ms {
                    component = step.ordinal;
                    crossed = true;
                    return false;
                }
                envelope = Some(step.envelope);
                true
            })?;
            if crossed && envelope.is_none() {
                component = 0;
            }
            components.push((component, envelope));
        }
        let fits = |(_, position): &(RevisionEpochId, usize)| {
            columns
                .iter()
                .zip(&components)
                .all(|(column, (component, _))| column.before_position(*position) <= *component)
        };
        let fitting = revisions.partition_point(fits);
        let Some(index) = fitting.checked_sub(1) else {
            return Err(format!(
                "no recorded revision of stream {} stands at {at_ms} ms",
                self.run.get()
            ));
        };
        if let Some((_, next)) = revisions.get(index + 1) {
            for (column, (component, envelope)) in columns.iter().zip(components.iter_mut()) {
                let finish = column.before_position(*next);
                if *component > finish {
                    *component = finish;
                    *envelope = self.envelope_at(column, finish)?;
                }
            }
        }
        let resolved_ms = components
            .iter()
            .filter_map(|(_, envelope)| *envelope)
            .max()
            .unwrap_or(0);
        let cut = circular_runtime::LogCut::new(columns.iter().zip(&components).map(
            |(column, (component, _))| {
                (
                    column.actor.clone(),
                    circular_core::ArrivalIndex::new(*component),
                )
            },
        ));
        Ok(TimelineAt::new(
            at_ms,
            resolved_ms,
            self.run.get(),
            revisions[index].0,
            crate::daemon::replay::replay_wire_cut(&cut)?,
        ))
    }
}

/// Pause/Resume marks use System's own acceptance time, not a nearby user arrival.
fn lifecycle_marks(
    records: &circular_store::JournalView,
) -> Result<Vec<(TimelineMarkKind, u64)>, String> {
    use crate::kernel::system::SystemBody;
    let mut marks = Vec::new();
    for row in records.all_rows()? {
        let row = row?;
        let record = row.record();
        let kind = match SystemBody::read(&record)? {
            Some(SystemBody::PauseAccepted { .. }) => TimelineMarkKind::Pause,
            Some(SystemBody::ResumeAccepted) => TimelineMarkKind::Resume,
            _ => continue,
        };
        let ClassKey::Observation(ObservationKey::StreamItem(_, bucket, _)) = record.header().key()
        else {
            continue;
        };
        marks.push((kind, bucket.millis()));
    }
    Ok(marks)
}

pub(crate) fn unrecorded_timeline_bins(
    args: &TimelineBinsArgs,
) -> Result<TimelineBins, circular_protocol::timeline::TimelineArgsRejection> {
    let (bin_ms, covered) = args.coverage()?;
    Ok(TimelineBins {
        from_ms: args.from_ms,
        to_ms: covered,
        bin_ms,
        bins: vec![TimelineBin::default(); usize::try_from(args.bins).unwrap_or(0)],
        marks: Vec::new(),
        clock_regressions: 0,
    })
}
