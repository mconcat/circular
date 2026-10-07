use super::*;
use crate::daemon::ledger::projection::{ProjectionRecord, ProjectionSnapshot};
use circular_core::BuiltinObservationName;
use circular_store::Class;

type Environment = circular_protocol::declaration_payload::AuthoringEnvironment;

pub(crate) struct FrozenRecords {
    reader: Records,
    sources: Sources,
    cut: u64,
    rows: usize,
    checkpoints: usize,
    records: VecDeque<(u64, Vec<u8>)>,
    final_environment: Option<Environment>,
    after: Option<Vec<u8>>,
    #[cfg(test)]
    pub(super) records_rebuilt: usize,
}

fn kind(
    envelope: &circular_store::RecordEnvelope<'_>,
) -> Result<Option<BuiltinObservationName>, String> {
    if envelope.class != Class::Observation {
        return Ok(None);
    }
    let tag = *envelope
        .class_key_body
        .first()
        .ok_or("records observation kind is absent")?;
    let kind = BuiltinObservationName::from_tag(tag)
        .ok_or_else(|| "records observation kind is unknown".to_owned())?;
    if !matches!(envelope.class_key_tag, 1 | 3) {
        return Ok(None);
    }
    Ok(Some(kind))
}

pub(super) fn is_cell_lifecycle(bytes: &[u8]) -> Result<bool, String> {
    let envelope =
        circular_store::decode_envelope(bytes).map_err(|e| format!("records envelope: {e:?}"))?;
    cell_lifecycle(&envelope)
}

fn cell_lifecycle(envelope: &circular_store::RecordEnvelope<'_>) -> Result<bool, String> {
    Ok(envelope.class == Class::Structure
        || (envelope.class == Class::Boundary && envelope.arrival_index.is_some())
        || matches!(
            kind(envelope)?,
            Some(
                BuiltinObservationName::InstanceTransition
                    | BuiltinObservationName::IncarnationTransition
            )
        ))
}

/// A filter over the existing record envelope, before typed record rebuild.
/// Candidate payload validation still belongs to the existing selected reader.
/// Scope is read from the recorded body; no actor or inferred address is used.
pub(super) fn selected_envelope(
    bytes: &[u8],
    scope: &circular_plan::ScopeId,
) -> Result<bool, String> {
    let envelope =
        circular_store::decode_envelope(bytes).map_err(|e| format!("records envelope: {e:?}"))?;
    let Some(kind) = kind(&envelope)? else {
        return Ok(false);
    };
    let Some(fact_tag) = envelope.observation_fact_tag else {
        return Ok(false);
    };
    if !super::published_fact(kind, fact_tag) {
        return Ok(false);
    }
    let health = kind == BuiltinObservationName::DiagnosticOccurrence;
    let payload = circular_core::EncodedPayload::from_bytes(envelope.payload)
        .map_err(|e| format!("observation payload: {e:?}"))?;
    let body = circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
        .map_err(|e| format!("observation body: {e:?}"))?;
    if health
        && !matches!(body.as_object().and_then(|o| o.get("kind")), Some(Value::String(k)) if k == engine::ACTOR_HEALTH_TRANSITION_KIND)
    {
        return Ok(false);
    }
    if scope == &circular_plan::ScopeId::root() {
        return Ok(true);
    }
    let found = if kind == BuiltinObservationName::InstanceTransition {
        body.as_array()
            .and_then(|fields| fields.get(1))
            .map(circular_store::scope_from_value)
            .transpose()
            .map_err(|e| format!("instance transition scope: {e:?}"))?
    } else if let Some(value) = body.as_object().and_then(|o| o.get("scope")) {
        Some(
            circular_store::scope_from_value(value)
                .map_err(|e| format!("observation scope: {e:?}"))?,
        )
    } else if let Some(value) = body.as_object().and_then(|o| o.get("actor")) {
        Some(
            circular_store::actor_parts_from_value(value)
                .map_err(|e| format!("observation actor: {e:?}"))?
                .0,
        )
    } else {
        None
    };
    Ok(found.is_none_or(|found| &found == scope))
}

impl FrozenRecords {
    pub(crate) fn capture(
        path: &Path,
        args: Value,
        sources: Sources,
    ) -> Result<ProjectionSnapshot, String> {
        let reader = Self::read_with(path, args, sources)?;
        let mut snapshot =
            ProjectionSnapshot::recorded(reader.reader.anchor.clone(), Vec::new(), Vec::new());
        snapshot.records_tail = Some(Box::new(reader));
        Ok(snapshot)
    }

    pub(super) fn read_with(path: &Path, args: Value, sources: Sources) -> Result<Self, String> {
        let (mut reader, since) = Records::prepare(path, args, sources.authoring.creation())?;
        let cut = sources.capped(reader.complete_through(sources.marks())?);
        reader.authoring = Some(sources.authoring.state_through(cut)?.2);
        let concrete = reader
            .scope
            .segments()
            .iter()
            .any(|s| matches!(s, circular_plan::ScopeSeg::Instance { .. }));
        if concrete && let Some(prefix) = sources.arrivals.as_ref() {
            for row in prefix.all_rows()? {
                let row = row?;
                if row.commit() > cut {
                    break;
                }
                let lifecycle =
                    match crate::daemon::record_rows::with_envelope(&row, cell_lifecycle)? {
                        Some(found) => found?,
                        None => true,
                    };
                if !lifecycle {
                    continue;
                }
                let record = crate::daemon::record_rows::rebuild(&row);
                reader.fold_instances(&record, &sources)?;
            }
        }
        if !reader.scope_exists(&sources)? {
            return Err("records.scope does not exist in the retained authoring prefix".into());
        }
        let final_environment = reader
            .authoring
            .as_ref()
            .map(|state| state.environment().clone());
        let after = since
            .as_ref()
            .map(|c| position(c, &reader.anchor))
            .transpose()?;
        Ok(Self {
            reader,
            sources,
            cut,
            rows: 0,
            checkpoints: 0,
            records: VecDeque::new(),
            final_environment,
            after,
            #[cfg(test)]
            records_rebuilt: 0,
        })
    }

    fn next(&mut self) -> Result<Option<ProjectionRecord>, String> {
        loop {
            if let Some((commit, bytes)) = self.records.pop_front() {
                if let Some(target) = &self.after {
                    let envelope = circular_store::decode_envelope(&bytes)
                        .map_err(|e| format!("records envelope: {e:?}"))?;
                    let head = circular_store::OpaqueWitness::for_envelope(&envelope);
                    if !target.starts_with(head.as_bytes()) {
                        continue;
                    }
                }
                if !selected_envelope(&bytes, &self.reader.scope)? {
                    continue;
                }
                let record = ArrivalProjection::new()
                    .record(&bytes)
                    .map_err(|e| format!("records projection: {e:?}"))?;
                #[cfg(test)]
                {
                    self.records_rebuilt += 1;
                }
                if !selected(&record, &self.reader.scope)? {
                    continue;
                }
                let witness = crate::daemon::ledger::record_witness(record.header())?;
                let key = witness.as_bytes().to_vec();
                let body = circular_store::encode_record(&record, &ProductRecordCodec)
                    .map_err(|e| format!("record body: {e:?}"))?;
                if let Some(prior) = self.reader.seen.get(&key) {
                    if prior != &body {
                        return Err("immutable record reference changed".into());
                    }
                    continue;
                }
                self.reader.seen.insert(key.clone(), body.clone());
                if let Some(target) = &self.after {
                    if &key == target {
                        let environment = Some(self.sources.authoring.environment_before(commit)?);
                        if environment != self.final_environment {
                            return Err("records snapshot prefix requires reset".into());
                        }
                        self.after = None;
                    }
                    continue;
                }
                let cursor = cursor(&self.reader.anchor, key);
                let value = ItemBody::record(&record, body)?.payload(cursor)?;
                return Ok(Some(ProjectionRecord::recorded(witness, value)));
            }
            let next = match self.sources.arrivals.as_ref() {
                Some(prefix) => next_arrival(prefix, self.rows, self.checkpoints, self.cut)?,
                None => None,
            };
            let Some((commit, next)) = next else {
                if self.after.is_some() {
                    return Err("records snapshot prefix requires reset".into());
                }
                return Ok(None);
            };
            let prefix = self
                .sources
                .arrivals
                .as_ref()
                .expect("arrival candidate has a prefix");
            let bytes = if next == Next::Row {
                self.rows += 1;
                column_bytes(prefix, self.rows - 1, true)?
            } else {
                self.checkpoints += 1;
                column_bytes(prefix, self.checkpoints - 1, false)?
            };
            self.records.push_back((commit, bytes));
        }
    }

    /// Fill only this page and one selected lookahead needed to distinguish
    /// More from Complete. Rows already materialized are never rebuilt again.
    pub(crate) fn fill(
        &mut self,
        rows: &mut Vec<ProjectionRecord>,
        count: usize,
    ) -> Result<bool, String> {
        while rows.len() < count {
            let Some(row) = self.next()? else {
                return Ok(true);
            };
            rows.push(row);
        }
        Ok(false)
    }
}
