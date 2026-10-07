fn actor_event_value(record: &Record<ProductStore>) -> Option<Value> {
    let body = match LiveFrame::of(record) {
        Some(LiveFrame::ActorEvent { actor, record, .. }) => {
            let BoundaryFact::Arrival {
                origin,
                body: arrival_body,
                observed_at,
                route_edge,
                inlet,
                arrival_index,
                causal_parents,
                ..
            } = record.fact()
            else {
                return None;
            };
            let body = match arrival_body {
                circular_store::ArrivalBody::Owned(payload) => circular_core::decode(
                    payload.body(),
                    circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                ).ok()?,
                circular_store::ArrivalBody::Emitted { .. } => return None,
            };
            let actor = circular_store::actor_value(actor).ok()?;
            let origin_stamp = arrival_origin_stamp(record);
            let mut fields = vec![
                (
                    "kind".to_owned(),
                    Value::string(circular_protocol::actor_events::ACTOR_ARRIVAL_KIND),
                ),
                ("body".to_owned(), body),
                ("actor".to_owned(), actor),
                (
                    "at".to_owned(),
                    crate::daemon::restart_query::stamp_value(record.header().at()).ok()?,
                ),
                (
                    "causal_parents".to_owned(),
                    Value::array(
                        causal_parents
                            .iter()
                            .map(|parent| {
                                crate::daemon::restart_query::stamp_value(
                                    circular_core::EventId::stamp(parent),
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .ok()?,
                    ),
                ),
                (
                    "index".to_owned(),
                    Value::int(i64::try_from(arrival_index.get()).unwrap_or(i64::MAX)),
                ),
                (
                    "observed_at_ms".to_owned(),
                    Value::int(i64::try_from(observed_at.millis()).unwrap_or(i64::MAX)),
                ),
            ];
            if let Some(port) = inlet {
                fields.push(("port".into(), Value::string(port.as_str())));
            }
            if let Some(stamp) = origin_stamp {
                fields.push((
                    "origin".into(),
                    crate::daemon::restart_query::stamp_value(stamp).ok()?,
                ));
            }
            let edge = match origin.as_ref() {
                circular_store::ArrivalOrigin::EdgeDelivery { edge, .. } => Some(edge),
                circular_store::ArrivalOrigin::ExternalInject { .. } => route_edge.as_ref(),
                _ => None,
            };
            if let Some(edge) = edge {
                fields.push(("edge".to_owned(), circular_store::edge_value(edge).ok()?));
            }
            if let circular_store::ArrivalOrigin::EffectOutcome { effect, .. } = origin.as_ref()
                && let circular_runtime::EffectOccasion::Delivery(edge, stamp) = effect.occasion()
            {
                let mut occasion = vec![(
                    "origin",
                    crate::daemon::restart_query::stamp_value(stamp).ok()?,
                )];
                if let Some(edge) = edge {
                    occasion.push(("edge", circular_store::edge_value(edge).ok()?));
                }
                fields.push(("occasion".to_owned(), Value::object(occasion).ok()?));
            }
            if let Some(fact) =
                engine::effect_outcome_record::effect_outcome_approval(origin.as_ref()).ok()?
            {
                let (name, value) = crate::daemon::approval::actor_event_approval(fact).ok()?;
                fields.push((name.to_owned(), value));
            }
            Value::object(fields).ok()?
        }
        Some(LiveFrame::ActorEmission { producer, record }) => {
            let BoundaryFact::EmissionBody {
                port,
                cause,
                observed_at,
                payload,
            } = record.fact()
            else {
                return None;
            };
            let body = circular_core::decode(
                payload.body(),
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
            )
            .ok()?;
            Value::object([
                (
                    "kind",
                    Value::string(circular_protocol::actor_events::ACTOR_EMISSION_KIND),
                ),
                ("body", body),
                ("actor", circular_store::actor_value(producer).ok()?),
                (
                    "at",
                    crate::daemon::restart_query::stamp_value(record.header().at()).ok()?,
                ),
                (
                    "index",
                    Value::int(i64::try_from(cause.get()).unwrap_or(i64::MAX)),
                ),
                (
                    "observed_at_ms",
                    Value::int(i64::try_from(observed_at.millis()).unwrap_or(i64::MAX)),
                ),
                ("port", Value::string(port.as_str())),
            ])
            .ok()?
        }
        _ => return None,
    };
    Some(body)
}

pub(crate) struct LiveFeedPage {
    pub(crate) frames: Vec<(circular_store::SurfaceSequence, Value)>,
    pub(crate) pending: u64,
}

impl LiveFeedPage {
    pub(crate) fn take(
        mut rows: impl Iterator<Item = Result<(circular_store::SurfaceSequence, Value), String>>,
        limit: usize,
    ) -> Result<Self, String> {
        let mut frames = Vec::new();
        while frames.len() < limit {
            let Some(row) = rows.next() else {
                return Ok(Self {
                    pending: frames.len() as u64,
                    frames,
                });
            };
            frames.push(row?);
        }
        let mut pending = frames.len() as u64;
        for row in rows {
            row?;
            pending += 1;
        }
        Ok(Self { frames, pending })
    }
}

macro_rules! impl_read_prefix {
    ($read:ty) => {
        impl $read {
            pub(crate) fn view(&self) -> &circular_store::JournalView {
                &self.store
            }

            #[cfg(test)]
            pub(crate) fn records(&self) -> &circular_store::RecordSegments<ProductStore> {
                self.store.tail().records()
            }

            pub(crate) fn unboarded(&self) -> &[String] {
                &self.unboarded
            }

            pub(crate) fn templates(&self) -> &[String] {
                &self.templates
            }

            pub(crate) const fn stream(&self) -> StreamId {
                self.run
            }

            pub(crate) fn recorded(&self) -> Result<usize, String> {
                let prefix = match self.store.prefix() {
                    Some(prefix) => prefix.data_arrivals()?,
                    None => 0,
                };
                Ok(prefix + self.recorded)
            }

            pub(crate) fn mark(&self) -> circular_store::SurfaceSequence {
                self.store.surface_mark()
            }

            fn anchor_mark(&self) -> Result<circular_store::SurfaceSequence, String> {
                self.store.ordinal(self.store.end())
            }

            fn actor_event_records_since(
                &self,
                since: circular_store::SurfaceSequence,
            ) -> impl Iterator<
                Item = Result<
                    (
                        circular_store::SurfaceSequence,
                        std::borrow::Cow<'_, Record<ProductStore>>,
                        Value,
                    ),
                    String,
                >,
            > + '_ {
                let from = usize::try_from(since.get().saturating_sub(1)).unwrap_or(usize::MAX);
                self.store
                    .rows(from.min(self.store.end())..self.store.end())
                    .filter_map(move |row| {
                        let row = match row {
                            Ok(row) => row,
                            Err(error) => return Some(Err(error)),
                        };
                        let surface = circular_store::SurfaceSequence::from_index(row.position());
                        let stored = row.into_record();
                        actor_event_value(&stored).map(|body| Ok((surface, stored, body)))
                    })
            }

            pub(crate) fn actor_events_feed_within(
                &self,
                since: circular_store::SurfaceSequence,
                limit: usize,
                window: Option<(
                    Option<&crate::daemon::ledger::ReadBound>,
                    &crate::daemon::ledger::ReadBound,
                )>,
            ) -> Result<LiveFeedPage, String> {
                let admits = move |record: &Record<ProductStore>,
                                   bound: &crate::daemon::ledger::ReadBound| {
                    match record {
                        Record::Boundary(boundary) => {
                            match (boundary.header().key(), boundary.fact()) {
                                (
                                    ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }),
                                    BoundaryFact::Arrival { arrival_index, .. },
                                ) => bound.admits_arrival(actor, arrival_index.get()),
                                (
                                    ClassKey::Boundary(BoundaryKey::EmissionBody {
                                        producer, ..
                                    }),
                                    BoundaryFact::EmissionBody { cause, .. },
                                ) => bound.admits_arrival(producer, cause.get()),
                                _ => false,
                            }
                        }
                        _ => false,
                    }
                };
                LiveFeedPage::take(
                    self.actor_event_records_since(since)
                        .filter(move |row| {
                            row.as_ref().map_or(true, |(_, record, _)| {
                                window.is_none_or(|(opened, _)| {
                                    opened.is_none_or(|opened| !admits(record, opened))
                                })
                            })
                        })
                        .take_while(move |row| {
                            row.as_ref().map_or(true, |(_, record, _)| {
                                window.is_none_or(|(_, position)| admits(record, position))
                            })
                        })
                        .map(|row| row.map(|(surface, _, body)| (surface, body))),
                    limit,
                )
            }

            fn display_bodies_since(
                &self,
                since: circular_store::SurfaceSequence,
            ) -> impl Iterator<Item = Result<(circular_store::SurfaceSequence, Value), String>> + '_
            {
                let from = usize::try_from(since.get().saturating_sub(1)).unwrap_or(usize::MAX);
                self.store
                    .rows(from.min(self.store.end())..self.store.end())
                    .filter_map(|row| {
                        let row = match row {
                            Ok(row) => row,
                            Err(error) => return Some(Err(error)),
                        };
                        let surface = circular_store::SurfaceSequence::from_index(row.position());
                        let row = row.record();
                        match LiveFrame::of(&row)? {
                            LiveFrame::Display { key, record } => {
                                let body = circular_core::decode(
                                    record.payload().body(),
                                    circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                                ).ok()?;
                                let actor = circular_store::named_actor_value(key.args()).ok()?;
                                Some(Ok((
                                    surface,
                                    display_frame_value(
                                        key,
                                        record.header().at(),
                                        actor,
                                        body,
                                    )?,
                                )))
                            }
                            _ => None,
                        }
                    })
            }

            pub(crate) fn display_frames_feed_within(
                &self,
                since: circular_store::SurfaceSequence,
                limit: usize,
                window: Option<(
                    Option<&crate::daemon::ledger::ReadBound>,
                    &crate::daemon::ledger::ReadBound,
                )>,
            ) -> Result<LiveFeedPage, String> {
                let bound = window.map(|(_, position)| position);
                LiveFeedPage::take(
                    self.display_bodies_since(since).take_while(|row| {
                        row.as_ref().map_or(true, |(surface, _)| {
                            bound.is_none_or(|bound| {
                                bound.admits(
                                    usize::try_from(surface.get().saturating_sub(1))
                                        .unwrap_or(usize::MAX),
                                )
                            })
                        })
                    }),
                    limit,
                )
            }

            pub(crate) fn presentation(
                &self,
                authoring_store: &crate::daemon::authoring_store::AuthoringStore,
            ) -> Result<Value, String> {
                let mut current = None;
                for position in self
                    .store
                    .sealed_structure_positions()?
                    .into_iter()
                    .rev()
                {
                    if let Record::Structure(found) = &*self.store.row(position)?
                        && let StructureFact::GraphRevision(payload) = found.fact()
                    {
                        current = Some(payload.clone());
                        break;
                    }
                }
                let payload = current.ok_or_else(|| "this run has no graph revision".to_owned())?;
                let value = circular_core::decode(
                    payload.body(),
                    circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                )
                .map_err(|error| format!("GraphRevision payload does not decode: {error:?}"))?;
                let cut = GraphRevisionCut::from_value(value)?;
                let replayed = authoring_store.load_at(cut.cursor)?;
                let snapshot = replayed
                    .snapshot(Vec::new())
                    .map_err(|rejection| rejection.to_string())?;
                cut.verify_snapshot(&snapshot)?;
                let plan = replayed
                    .current_plan()
                    .map_err(|rejection| rejection.to_string())?;
                Ok(presentation_value(&plan))
            }

            fn recorded_observation_actors(
                &self,
                actor: NamedActorId,
            ) -> Result<Vec<NamedActorId>, String> {
                let scopes = crate::authoring_assembly::projection::scope_roles(&self.plan);
                if circular_plan::admit_runtime_scope(&scopes, actor.scope()).is_ok() {
                    return Ok(vec![actor]);
                }
                let mut actors = Vec::new();
                let mut seen = BTreeSet::new();
                for position in self.store.lifecycle_positions_of(
                    &BuiltinObservationName::InstanceTransition,
                )? {
                    let record = self.store.row(position)?;
                    let Some(cell) =
                        engine::minted_instance_scope(&record).map_err(|e| e.to_string())?
                    else {
                        continue;
                    };
                    let Ok(admitted) = circular_plan::admit_runtime_scope(&scopes, &cell) else {
                        continue;
                    };
                    if !admitted.declared().is_ancestor_of(actor.scope()) {
                        continue;
                    }
                    let mut segments = cell.segments().to_vec();
                    segments.extend_from_slice(
                        &actor.scope().segments()[admitted.declared().depth()..],
                    );
                    let scope = ScopeId::from_segments(segments).map_err(|e| format!("{e:?}"))?;
                    if circular_plan::admit_runtime_scope(&scopes, &scope).is_err() {
                        continue;
                    }
                    let cell_actor = NamedActorId::new(scope, actor.name().clone());
                    if seen.insert(cell_actor.clone()) {
                        actors.push(cell_actor);
                    }
                }
                Ok(actors)
            }

            pub(crate) fn transitions(&self) -> Result<Vec<Value>, String> {
                Ok(self
                    .transition_projection(None)?
                    .into_iter()
                    .map(|(_, value)| value)
                    .collect())
            }

            fn transition_projection(
                &self,
                bound: Option<&crate::daemon::ledger::ReadBound>,
            ) -> Result<Vec<(circular_store::OpaqueWitness, Value)>, String> {
                let mut projected = Vec::new();
                for row in self.store.sealed_rows()? {
                    let row = row?;
                    if bound.is_some_and(|bound| !bound.admits(row.position())) {
                        break;
                    }
                    if !crate::daemon::record_rows::keeps(&row, |envelope| {
                        envelope.class_key_tag == 1
                            && matches!(
                                crate::daemon::record_rows::observation_kind(envelope),
                                Some(
                                    BuiltinObservationName::InstanceTransition
                                        | BuiltinObservationName::IncarnationTransition
                                )
                            )
                    })? {
                        continue;
                    }
                    let record = crate::daemon::record_rows::rebuild(&row);
                    if record.header().class() != circular_store::Class::Observation
                    {
                        continue;
                    }
                    let value = match engine::instance_transition_value(&record)
                        .map_err(|error| error.to_string())?
                    {
                        Some(value) => value,
                        None => match engine::incarnation_transition_value(&record)? {
                            Some(value) => value,
                            None => continue,
                        },
                    };
                    projected.push((
                        crate::daemon::ledger::record_witness(record.header())?,
                        value,
                    ));
                }
                Ok(projected)
            }

            pub(crate) fn transitions_snapshot_within(
                &self,
                bound: Option<&crate::daemon::ledger::ReadBound>,
            ) -> Result<crate::daemon::ledger::projection::ProjectionSnapshot, String> {
                Ok(
                    crate::daemon::ledger::projection::ProjectionSnapshot::recorded(
                        Value::Null,
                        self.transition_projection(bound)?
                            .into_iter()
                            .map(|(witness, value)| {
                                crate::daemon::ledger::projection::ProjectionRecord::recorded(
                                    witness, value,
                                )
                            })
                            .collect(),
                        Vec::new(),
                    ),
                )
            }

            fn lifecycle_anchor(&self) -> Value {
                Value::int(i64::try_from(self.run.get()).unwrap_or(i64::MAX))
            }

            pub(crate) fn dead_letters(&self) -> Result<Vec<Value>, String> {
                Ok(self
                    .dead_letter_projection(None)?
                    .into_iter()
                    .map(|(_, value)| value)
                    .collect())
            }

            fn dead_letter_projection(
                &self,
                bound: Option<&crate::daemon::ledger::ReadBound>,
            ) -> Result<Vec<(circular_store::OpaqueWitness, Value)>, String> {
                let mut projected = Vec::new();
                for row in self.store.all_rows()? {
                    let row = row?;
                    if bound.is_some_and(|bound| !bound.admits(row.position())) {
                        break;
                    }
                    if !crate::daemon::record_rows::keeps(&row, |envelope| {
                        envelope.class_key_tag == 1
                            && crate::daemon::record_rows::observation_kind(envelope)
                                == Some(BuiltinObservationName::DeadLetterEntry)
                    })? {
                        continue;
                    }
                    let record = crate::daemon::record_rows::rebuild(&row);
                    let Some(value) =
                        engine::dead_letter_value(&record).map_err(|error| error.to_string())?
                    else {
                        continue;
                    };
                    let value = match (record.header().key(), value) {
                        (ClassKey::Observation(key), Value::Object(fields)) => {
                            let mut fields = fields.into_map();
                            fields.insert(
                                "observation_bucket".into(),
                                Value::UInt(key.bucket().millis()),
                            );
                            Value::Object(circular_core::ObjectValue::from_map(fields))
                        }
                        (_, value) => value,
                    };
                    projected.push((
                        crate::daemon::ledger::record_witness(record.header())?,
                        value,
                    ));
                }
                Ok(projected)
            }

            pub(crate) fn dead_letters_snapshot_within(
                &self,
                bound: Option<&crate::daemon::ledger::ReadBound>,
            ) -> Result<crate::daemon::ledger::projection::ProjectionSnapshot, String> {
                Ok(
                    crate::daemon::ledger::projection::ProjectionSnapshot::recorded(
                        Value::Null,
                        self.dead_letter_projection(bound)?
                            .into_iter()
                            .map(|(witness, value)| {
                                crate::daemon::ledger::projection::ProjectionRecord::recorded(
                                    witness, value,
                                )
                            })
                            .collect(),
                        Vec::new(),
                    ),
                )
            }

            pub(crate) fn structure(&self) -> Result<usize, String> {
                self.store.sealed_structure_count()
            }

            /// Concrete child pipeline scopes in the currently bound plan.
            ///
            /// The plan and the live stood-actor set are read under this one `&self`
            /// borrow. Template branches are omitted rather than projected as empty
            /// runnable pipelines. Scope identity uses the already-published product
            /// carrier, and the `BTreeMap` walk gives a unique canonical order.
            pub(crate) fn standing_pipelines(&self) -> Result<(Value, Vec<Value>), String> {
                standing_pipeline_projection(
                    &self.plan,
                    &self.stood_actors,
                    self.run.get(),
                    self.revisions,
                )
            }
        }
    };
}
impl_read_prefix!(ServerRun);
impl_read_prefix!(ServerRead);

#[cfg(test)]
impl ServerRun {
    pub(crate) fn arrivals_at_mount(&self, mount: &str) -> Result<Vec<Value>, String> {
        self.read_prefix().arrivals_at_mount(mount)
    }

    pub(crate) fn arrival_digests_at_mount(&self, mount: &str) -> Result<Vec<Value>, String> {
        self.read_prefix().arrival_digests_at_mount(mount)
    }
}
