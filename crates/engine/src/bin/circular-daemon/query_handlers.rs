//! Registered query implementations. Discovery and dispatch use the same row;
//! these functions own actual argument admission, distinct from UI descriptors.
use super::*;
use circular_protocol::declaration_payload::Query;
use circular_protocol::rejection_code::RejectionReason;

pub(crate) fn authoring_snapshot(
    correlation: u32,
    query: Query,
    sources: &QuerySources<'_>,
    open: &mut Vec<OpenAuthoringQuery>,
) -> QueryResult {
    authoring_snapshot_page(
        correlation,
        query,
        |scope| {
            let historical;
            let authority = match &sources.bound {
                None => sources.authoring,
                Some(bound) => {
                    let standing = sources.server.ok_or("no stream is held for this cut")?;
                    let revision = standing
                        .arrival_revision_within(bound)?
                        .ok_or("the cut contains no arrival with an authoring revision")?;
                    historical = sources.authoring_store.load_at(revision.get())?;
                    &historical
                }
            };
            authority
                .snapshot(scope)
                .map_err(|reason| reason.to_string())
        },
        open,
    )
}

fn null_no_page(query: &Query) -> Result<(), Rejected> {
    if query.page.is_some() || query.args != Value::Null {
        return Err(rejection(
            RejectionReason::Malformed,
            &format!("{} takes Null args and no page cursor", query.name),
        ));
    }
    Ok(())
}

fn no_page(query: &Query) -> Result<(), Rejected> {
    if query.page.is_some() {
        return Err(rejection(
            RejectionReason::Malformed,
            &format!("{} does not take a page cursor", query.name),
        ));
    }
    Ok(())
}

fn page(result: Result<QueryPage, String>, reason: RejectionReason) -> Result<QueryPage, Rejected> {
    result.map_err(|message| rejection(reason, &message))
}

fn complete(anchor: Value, items: Vec<Value>) -> QueryPage {
    QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor,
        items,
        terminal: Terminal::Complete,
    }
}

pub(crate) fn agent_harnesses(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    null_no_page(&query)?;
    let standing = sources
        .system
        .and_then(|system| system.pipeline.harnesses().ok())
        .unwrap_or_default();
    page(
        crate::daemon::environment::configured_agent_harnesses(sources.state_directory)
            .map(|saved| agent_harnesses_page(&standing, &saved)),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn agent_harness_candidates(
    query: Query,
    _: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    null_no_page(&query)?;
    page(
        crate::daemon::environment::detected_agent_harnesses()
            .and_then(agent_harness_candidates_page),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn daemon_health(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    null_no_page(&query)?;
    let view = sources
        .system
        .map(ledger::SystemRuntime::read_view)
        .or_else(|| sources.server.map(|server| server.view().clone()));
    let stream = sources
        .system
        .map(|system| system.stream)
        .or_else(|| sources.server.map(ledger::ServerRead::stream));
    let ingress = sources.server.and_then(ledger::ServerRead::ingress);
    page(
        crate::daemon::daemon_health::health_page_within(
            view.as_ref(),
            stream,
            sources
                .system
                .and_then(|system| system.journal.recorder_stop().map(|stop| stop.code()))
                .or_else(|| ingress.and_then(ledger::ServerIngress::recorder_stop)),
            sources.execution.config_defaults(),
            sources.bound.as_ref(),
            ingress
                .and_then(ledger::ServerIngress::journal_measure)
                .transpose()
                .map_err(|message| rejection(RejectionReason::Unresolved, &message))?,
        ),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn catalog(query: Query, sources: &QuerySources<'_>) -> Result<QueryPage, Rejected> {
    null_no_page(&query)?;
    page(
        crate::daemon::query_catalog::catalog_page(QueryCatalogState {
            pipeline_available: sources.server.is_some(),
        }),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn actor_catalog(query: Query, _: &QuerySources<'_>) -> Result<QueryPage, Rejected> {
    null_no_page(&query)?;
    page(
        actor_catalog::catalog_items()
            .map(|items| complete(actor_catalog::catalog_anchor(), items)),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn actor_create_inputs(
    query: Query,
    _: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    null_no_page(&query)?;
    page(
        actor_catalog::create_input_items()
            .map(|items| complete(actor_catalog::create_input_anchor(), items)),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn actor_configuration_admission(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    no_page(&query)?;
    actor_catalog::configuration_admission_page(query.args, sources.authoring)
}

pub(crate) fn actor_create_admission(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    no_page(&query)?;
    actor_catalog::create_admission_page(query.args, sources.authoring)
}

pub(crate) fn authoring_actor_ports(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    no_page(&query)?;
    page(
        (|| {
            let scope = authoring_snapshot_scope(query.args)?;
            let snapshot = sources
                .authoring
                .snapshot(scope.clone())
                .map_err(|rejection| rejection.to_string())?;
            let items = actor_catalog::authoring_port_items(
                sources
                    .authoring
                    .current()
                    .actor_ports(&scope)
                    .map_err(|rejection| rejection.to_string())?,
            )?;
            Ok(complete(
                current_revision_value(snapshot.authoring_revision.as_deref()),
                items,
            ))
        })(),
        RejectionReason::Malformed,
    )
}

pub(crate) fn authoring_actor_access(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    no_page(&query)?;
    page(
        (|| {
            let scope = authoring_snapshot_scope(query.args)?;
            let snapshot = sources
                .authoring
                .snapshot(scope.clone())
                .map_err(|rejection| rejection.to_string())?;
            let items = actor_access::authoring_access_items(
                sources.authoring.current(),
                sources.execution,
                &scope,
            )?;
            Ok(complete(
                current_revision_value(snapshot.authoring_revision.as_deref()),
                items,
            ))
        })(),
        RejectionReason::Malformed,
    )
}

pub(crate) fn pipelines(query: Query, sources: &QuerySources<'_>) -> Result<QueryPage, Rejected> {
    let standing = sources.server.ok_or_else(no_standing_pipeline_rejection)?;
    null_no_page(&query)?;
    page(
        standing
            .standing_pipelines()
            .map(|(anchor, items)| complete(anchor, items)),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn runtime_approvals(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    null_no_page(&query)?;
    let Some(standing) = sources.server else {
        return page(
            engine::RuntimeApprovalQueue::recorded(&engine::state_journal::state_journal_path(
                sources.state_directory,
            ))
            .snapshot()
            .and_then(|snapshot| {
                approval::queue_page(snapshot, ApprovalProducerAvailability::Available)
            }),
            RejectionReason::Unresolved,
        );
    };
    page(
        standing
            .approval_snapshot()
            .map_err(|error| format!("runtime approval snapshot failed: {error:?}"))
            .and_then(|snapshot| {
                approval::queue_page(snapshot, ApprovalProducerAvailability::Available)
            }),
        RejectionReason::Unresolved,
    )
}

pub(crate) fn presentation(_: Query, sources: &QuerySources<'_>) -> Result<QueryPage, Rejected> {
    let standing = sources.server.ok_or_else(no_standing_pipeline_rejection)?;
    page(
        standing.presentation(sources.authoring_store).map(|value| {
            complete(
                Value::int(i64::try_from(standing.stream().get()).unwrap_or(i64::MAX)),
                vec![value],
            )
        }),
        RejectionReason::Malformed,
    )
}

pub(crate) fn rollup(query: Query, sources: &QuerySources<'_>) -> Result<QueryPage, Rejected> {
    let standing = sources.server.ok_or_else(no_standing_pipeline_rejection)?;
    page(
        standing.rollup_within(query.since.as_ref(), sources.bound.as_ref()),
        RejectionReason::Malformed,
    )
}

fn standing_null<'a>(
    query: &Query,
    sources: &QuerySources<'a>,
) -> Result<Option<&'a ledger::ServerRead>, Rejected> {
    if query.args != Value::Null {
        return Err(rejection(
            RejectionReason::Unresolved,
            &format!("{} takes Null args", query.name),
        ));
    }
    Ok(sources.server)
}

fn unrecorded_stream(sources: &QuerySources<'_>) -> Result<u64, Rejected> {
    crate::daemon::run_control_store::state_stream(Some(sources.state_directory))
        .map(|stream| stream.get())
        .map_err(|message| rejection(RejectionReason::Unresolved, &message))
}

fn unrecorded_observation_page() -> ledger::projection::ProjectionSnapshot {
    ledger::projection::ProjectionSnapshot::recorded(Value::Null, Vec::new(), Vec::new())
}

fn snapshot(
    result: Result<ledger::projection::ProjectionSnapshot, String>,
) -> Result<ledger::projection::ProjectionSnapshot, Rejected> {
    result.map_err(|message| rejection(RejectionReason::Unresolved, &message))
}

pub(crate) fn dead_letters(
    query: &Query,
    sources: &QuerySources<'_>,
) -> Result<ledger::projection::ProjectionSnapshot, Rejected> {
    let Some(standing) = standing_null(query, sources)? else {
        return Ok(unrecorded_observation_page());
    };
    snapshot(standing.dead_letters_snapshot_within(sources.bound.as_ref()))
}

pub(crate) fn transitions(
    query: &Query,
    sources: &QuerySources<'_>,
) -> Result<ledger::projection::ProjectionSnapshot, Rejected> {
    let Some(standing) = standing_null(query, sources)? else {
        return Ok(unrecorded_observation_page());
    };
    snapshot(standing.transitions_snapshot_within(sources.bound.as_ref()))
}

pub(crate) fn timeline(
    query: &Query,
    sources: &QuerySources<'_>,
) -> Result<ledger::projection::ProjectionSnapshot, Rejected> {
    let Some(standing) = standing_null(query, sources)? else {
        let anchor = Value::object([(
            "epoch_plans",
            Value::array([
                Value::Int(2),
                Value::object([("reason", Value::string("revision history is empty"))]).map_err(
                    |error| {
                        rejection(
                            RejectionReason::Unresolved,
                            &format!("timeline anchor: {error:?}"),
                        )
                    },
                )?,
            ]),
        )])
        .map_err(|error| {
            rejection(
                RejectionReason::Unresolved,
                &format!("timeline anchor: {error:?}"),
            )
        })?;
        return Ok(ledger::projection::ProjectionSnapshot::recorded(
            anchor,
            Vec::new(),
            Vec::new(),
        ));
    };
    snapshot(standing.timeline_since(query.since.as_ref()))
}

pub(crate) fn actor_events(
    query: &Query,
    sources: &QuerySources<'_>,
) -> Result<ledger::projection::ProjectionSnapshot, Rejected> {
    let Some(standing) = standing_null(query, sources)? else {
        unrecorded_stream(sources)?;
        let cut = crate::daemon::replay::replay_wire_cut(&circular_runtime::LogCut::empty())
            .and_then(|cut| {
                cut.to_value()
                    .map_err(|error| format!("query cut: {error:?}"))
            })
            .map_err(|message| rejection(RejectionReason::Unresolved, &message))?;
        let mut snapshot = ledger::projection::ProjectionSnapshot::recorded(
            Value::UInt(0),
            Vec::new(),
            Vec::new(),
        );
        snapshot.cut = Some(cut);
        snapshot.folded_from = query
            .since
            .as_ref()
            .is_some_and(|since| !since.components.is_empty())
            .then(|| Value::Array(Vec::new()));
        return Ok(snapshot);
    };
    snapshot(standing.actor_events_within(query.since.as_ref(), sources.bound.as_ref()))
}

pub(crate) fn arrival_scan(
    query: &Query,
    sources: &QuerySources<'_>,
) -> Result<ledger::projection::ProjectionSnapshot, Rejected> {
    let mount = query.args.as_str().ok_or_else(|| {
        rejection(
            RejectionReason::Malformed,
            "arrival.scan args must be a registered mount name",
        )
    })?;
    let Some(standing) = sources.server else {
        return Err(rejection(
            RejectionReason::Unresolved,
            &format!("no mount named {mount:?} is registered"),
        ));
    };
    snapshot(standing.arrivals_within(mount, query.since.as_ref(), sources.bound.as_ref()))
}

pub(crate) fn admit_observation_scan(query: &Query) -> Result<(), Rejected> {
    if query.args == Value::Null {
        Ok(())
    } else {
        Err(rejection(
            RejectionReason::Malformed,
            "observation-scan takes Null args",
        ))
    }
}

pub(crate) fn admit_records(_: &Query) -> Result<(), Rejected> {
    Ok(())
}

pub(crate) fn read_observations(
    _: &Query,
    directory: &std::path::Path,
    _: &mut dyn FnMut() -> crate::daemon::subscription::records::Sources,
) -> Result<ledger::projection::ProjectionSnapshot, String> {
    crate::daemon::restart_query::capture_snapshot(directory)
}

pub(crate) fn read_records(
    query: &Query,
    directory: &std::path::Path,
    sources: &mut dyn FnMut() -> crate::daemon::subscription::records::Sources,
) -> Result<ledger::projection::ProjectionSnapshot, String> {
    crate::daemon::subscription::records::FrozenRecords::capture(
        &engine::state_journal::state_journal_path(directory),
        query.args.clone(),
        sources(),
    )
}

fn timeline_rejection(
    query: &Query,
    reason: &circular_protocol::timeline::TimelineArgsRejection,
) -> Rejected {
    rejection(
        RejectionReason::Malformed,
        &format!("{} arguments: {reason}", query.name),
    )
}

pub(crate) fn timeline_bins(
    query: Query,
    sources: &QuerySources<'_>,
) -> Result<QueryPage, Rejected> {
    no_page(&query)?;
    let args = circular_protocol::timeline::TimelineBinsArgs::from_value(query.args.clone())
        .map_err(|reason| timeline_rejection(&query, &reason))?;
    let answer = match sources.server {
        Some(standing) => standing
            .timeline_bins(&args)
            .map_err(|message| rejection(RejectionReason::Unresolved, &message))?,
        None if args.actor.is_some() => {
            return Err(rejection(
                RejectionReason::Unresolved,
                "this state has recorded nothing, so it has no actor to narrow to",
            ));
        }
        None => ledger::unrecorded_timeline_bins(&args)
            .map_err(|reason| timeline_rejection(&query, &reason))?,
    };
    Ok(complete(answer.to_value(), Vec::new()))
}

pub(crate) fn timeline_at(query: Query, sources: &QuerySources<'_>) -> Result<QueryPage, Rejected> {
    no_page(&query)?;
    let args = circular_protocol::timeline::TimelineAtArgs::from_value(query.args.clone())
        .map_err(|reason| timeline_rejection(&query, &reason))?;
    let standing = sources.server.ok_or_else(|| {
        rejection(
            RejectionReason::Unresolved,
            "this state has recorded no revision, so no replay coordinate stands",
        )
    })?;
    let answer = standing
        .timeline_at(args.at_ms)
        .map_err(|message| rejection(RejectionReason::Unresolved, &message))?;
    let anchor = answer.to_value().map_err(|error| {
        rejection(
            RejectionReason::QueryResultEncodingFailed,
            &format!("timeline.at answer: {error:?}"),
        )
    })?;
    Ok(complete(anchor, Vec::new()))
}
