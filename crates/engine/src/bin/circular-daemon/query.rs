use crate::daemon::query_catalog::{QueryCatalogState, QueryId};
use crate::daemon::replay::ReplayLens;
use crate::daemon::session::QUERY_RESULT;
use circular_protocol::Partition;
use circular_protocol::rejection_code::RejectionReason;
#[path = "query_handlers.rs"]
pub(crate) mod handlers;
#[path = "query_pages.rs"]
pub(crate) mod pages;

use pages::{OpenProjectionQuery, projection_page, query_rejection, rejection};

#[derive(Default)]
pub(crate) struct OpenQueries {
    authoring: Vec<OpenAuthoringQuery>,
    projections: Vec<OpenProjectionQuery>,
}
impl OpenQueries {
    pub(crate) fn contains(&self, correlation: u32) -> bool {
        self.authoring.iter().any(|q| q.correlation == correlation)
            || self
                .projections
                .iter()
                .any(|q| q.correlation == correlation)
    }

    pub(crate) fn finish(&mut self, correlation: u32) {
        self.authoring.retain(|q| q.correlation != correlation);
        self.projections.retain(|q| q.correlation != correlation);
    }

    pub(crate) fn close(&mut self, correlation: u32) -> QueryResult {
        let anchor = if let Some(index) = self
            .authoring
            .iter()
            .position(|q| q.correlation == correlation)
        {
            authoring_snapshot_anchor(&self.authoring.remove(index).snapshot)
        } else if let Some(index) = self
            .projections
            .iter()
            .position(|q| q.correlation == correlation)
        {
            Ok(self.projections.remove(index).anchor().clone())
        } else {
            return query_rejection(
                RejectionReason::Unresolved,
                "this correlation names no open page sequence",
            );
        };
        match anchor {
            Ok(anchor) => QueryResult::Page(QueryPage {
                cut: None,
                folded_from: None,
                reached: None,
                anchor,
                items: Vec::new(),
                terminal: Terminal::Diagnostic(
                    circular_protocol::rejection_code::RejectionReason::ClosedByClient
                        .recorded_code(),
                ),
            }),
            Err(message) => query_rejection(RejectionReason::Malformed, &message),
        }
    }
}
use crate::daemon::{actor_access, actor_catalog, approval, ledger};
use circular_core::{Boundary, Ceilings, Value};
use circular_protocol::approval_payload::ApprovalProducerAvailability;
use circular_protocol::authoring_snapshot::{
    current_revision_value, environment_value, scope_identity_value,
};
#[cfg(test)]
use circular_protocol::declaration_payload::decode_query;
use circular_protocol::declaration_payload::{
    QueryPage, QueryResult, Rejected, Terminal, decode_scope_identity,
};
use circular_protocol::{
    Cursor as FsmCursor, EnvelopeHeader, PageEnd as FsmPageEnd, PageStep as FsmPageStep,
    QueryCursorFsm,
};
use circular_transport::{OwnerLocalChannelId, write_envelope};
use engine::authoring_assembly::ledger as authoring;
use engine::execution_profile::ProductExecutionProfile;

fn read_bound_of(
    standing: &ledger::ServerRead,
    upto: &circular_protocol::replay_payload::LogCut,
) -> Result<ledger::ReadBound, String> {
    standing.read_bound(&runtime_cut(upto)?)
}

pub(crate) fn runtime_cut(
    upto: &circular_protocol::replay_payload::LogCut,
) -> Result<circular_runtime::LogCut, String> {
    let mut components = Vec::with_capacity(upto.components.len());
    for component in &upto.components {
        let actor = circular_runtime::product_identity::named_actor_from_wire(&component.actor)
            .map_err(|error| format!("upto actor: {error:?}"))?;
        components.push((
            actor.as_actor_id(),
            circular_core::ArrivalIndex::new(component.index),
        ));
    }
    Ok(circular_runtime::LogCut::new(components))
}

pub(crate) fn record_sources(
    sources: crate::daemon::subscription::records::Sources,
    server: Option<&ledger::ServerRead>,
    lens: Option<&ReplayLens>,
    upto: Option<&circular_protocol::replay_payload::LogCut>,
) -> Result<crate::daemon::subscription::records::Sources, String> {
    if let Some(upto) = upto {
        return sources.with_upto(server, &runtime_cut(upto)?);
    }
    match lens {
        None => Ok(sources),
        Some(lens) => {
            let standing = server
                .filter(|standing| standing.stream().get() == lens.stream())
                .ok_or("the stream this replay reads is no longer held")?;
            sources.within_lens(standing, lens)
        }
    }
}

pub(crate) struct QueryRequestContext<'a> {
    pub(crate) server: Option<&'a ledger::ServerRead>,
    pub(crate) replay_lens: Option<&'a ReplayLens>,
    pub(crate) authoring: &'a authoring::AuthoringState,
    pub(crate) system: Option<&'a ledger::SystemRuntime>,
    pub(crate) authoring_store: &'a crate::daemon::authoring_store::AuthoringStore,
    pub(crate) execution: &'a ProductExecutionProfile,
    pub(crate) state_directory: &'a std::path::Path,
    pub(crate) authoring_queries: &'a mut OpenQueries,
}

/// The immutable sources already held by this request. A handler cannot mutate
/// the session's other queries; retained cursors are owned separately below.
pub(crate) struct QuerySources<'a> {
    server: Option<&'a ledger::ServerRead>,
    bound: Option<ledger::ReadBound>,
    authoring: &'a authoring::AuthoringState,
    system: Option<&'a ledger::SystemRuntime>,
    authoring_store: &'a crate::daemon::authoring_store::AuthoringStore,
    execution: &'a ProductExecutionProfile,
    state_directory: &'a std::path::Path,
}

type ProjectionHandler = fn(
    &circular_protocol::declaration_payload::Query,
    &QuerySources<'_>,
) -> Result<ledger::projection::ProjectionSnapshot, Rejected>;

#[derive(Clone, Copy)]
pub(crate) struct RecordQueryHandler {
    pub(crate) admit: fn(&circular_protocol::declaration_payload::Query) -> Result<(), Rejected>,
    pub(crate) capture: fn(
        &circular_protocol::declaration_payload::Query,
        &std::path::Path,
        &mut dyn FnMut() -> crate::daemon::subscription::records::Sources,
    ) -> Result<ledger::projection::ProjectionSnapshot, String>,
}

#[derive(Clone, Copy)]
pub(crate) enum QueryHandler {
    Immediate(
        fn(
            circular_protocol::declaration_payload::Query,
            &QuerySources<'_>,
        ) -> Result<QueryPage, Rejected>,
    ),
    Retained(
        fn(
            u32,
            circular_protocol::declaration_payload::Query,
            &QuerySources<'_>,
            &mut Vec<OpenAuthoringQuery>,
        ) -> QueryResult,
    ),
    Projection(ProjectionHandler),
    Records(RecordQueryHandler),
}

impl QueryHandler {
    pub(crate) fn record_reader(self) -> Option<RecordQueryHandler> {
        match self {
            Self::Records(reader) => Some(reader),
            _ => None,
        }
    }
}

/// Resolve a registered query. Retained continuations read their original cut,
/// even when the live run or replay lens has changed since the first page.
#[cfg(test)]
pub(crate) fn prepare_query(
    header: EnvelopeHeader,
    payload: &[u8],
    context: QueryRequestContext<'_>,
) -> QueryResult {
    match decode_query(payload, Ceilings::for_boundary(Boundary::Wire)) {
        Ok(query) => prepare_decoded_query(header, query, context),
        Err(rejection) => query_rejection(RejectionReason::Malformed, &format!("{rejection:?}")),
    }
}

#[cfg(test)]
fn prepare_decoded_query(
    header: EnvelopeHeader,
    query: circular_protocol::declaration_payload::Query,
    context: QueryRequestContext<'_>,
) -> QueryResult {
    let registration = crate::daemon::query_catalog::registration(&query.name);
    prepare_registered_query(header, query, registration, context)
}

pub(crate) fn prepare_registered_query(
    header: EnvelopeHeader,
    query: circular_protocol::declaration_payload::Query,
    registration: Option<crate::daemon::query_catalog::QueryRegistration>,
    context: QueryRequestContext<'_>,
) -> QueryResult {
    let QueryRequestContext {
        server,
        replay_lens,
        authoring,
        system,
        authoring_store,
        execution,
        state_directory,
        authoring_queries,
    } = context;
    let mut sources = QuerySources {
        server,
        bound: None,
        authoring,
        system,
        authoring_store,
        execution,
        state_directory,
    };
    if query.since.is_some()
        && !registration.is_some_and(|registration| registration.accepts_since())
    {
        return query_rejection(
            RejectionReason::Malformed,
            "this query does not accept since",
        );
    }
    let bounded = registration.is_some_and(|registration| {
        matches!(
            registration.id(),
            QueryId::Rollup
                | QueryId::ArrivalScan
                | QueryId::ActorEvents
                | QueryId::DaemonHealth
                | QueryId::DeadLetters
                | QueryId::Transitions
                | QueryId::Records
        )
    });
    let accepts_upto = bounded
        || registration.is_some_and(|registration| registration.id() == QueryId::AuthoringSnapshot);
    if query.upto.is_some() && !accepts_upto {
        return query_rejection(
            RejectionReason::Malformed,
            "this query does not accept upto",
        );
    }
    if query.lens.is_some() && !bounded {
        return query_rejection(
            RejectionReason::Malformed,
            "this query does not accept a lens",
        );
    }
    sources.bound = match (query.upto.as_ref(), replay_lens) {
        (Some(upto), _) => {
            let Some(standing) = server else {
                return QueryResult::Rejected(no_standing_pipeline_rejection());
            };
            match read_bound_of(standing, upto) {
                Ok(bound) => Some(bound),
                Err(reason) => return query_rejection(RejectionReason::Unresolved, &reason),
            }
        }
        (None, Some(lens)) if bounded => match server {
            Some(standing) if standing.stream().get() == lens.stream() => {
                Some(lens.position().clone())
            }
            _ => {
                return query_rejection(
                    RejectionReason::Unresolved,
                    "the stream this replay reads is no longer held",
                );
            }
        },
        _ => None,
    };
    let is_projection =
        registration.is_some_and(|registration| registration.uses_immutable_pager());
    if (is_projection
        && authoring_queries
            .authoring
            .iter()
            .any(|entry| entry.correlation == header.correlation()))
        || (!is_projection
            && authoring_queries
                .projections
                .iter()
                .any(|entry| entry.correlation == header.correlation()))
    {
        return query_rejection(
            RejectionReason::Malformed,
            "query registration differs from the retained cursor",
        );
    }
    let Some(registration) = registration else {
        return query_rejection(
            RejectionReason::Unresolved,
            &format!("no query named {:?} is registered", query.name),
        );
    };
    match registration.handler() {
        QueryHandler::Immediate(prepare) => {
            prepare(query, &sources).map_or_else(QueryResult::Rejected, QueryResult::Page)
        }
        QueryHandler::Retained(prepare) => prepare(
            header.correlation(),
            query,
            &sources,
            &mut authoring_queries.authoring,
        ),
        QueryHandler::Projection(capture) => {
            projection_page(
                header.correlation(),
                &query,
                &mut authoring_queries.projections,
                || capture(&query, &sources),
            )
        }
        QueryHandler::Records(reader) => {
            let records = match record_sources(
                crate::daemon::subscription::records::Sources::from_parts(
                    server,
                    system,
                    authoring_store,
                ),
                server,
                replay_lens,
                query.upto.as_ref(),
            ) {
                Ok(records) => records,
                Err(reason) => return query_rejection(RejectionReason::Unresolved, &reason),
            };
            prepare_registered_record_query(
                header.correlation(),
                &query,
                reader,
                state_directory,
                authoring_queries,
                || records.clone(),
            )
        }
    }
}

/// Read-only record query entry: no world, actor driver, lifecycle projection or writer.
#[cfg(test)]
pub(crate) fn prepare_record_query(
    correlation: u32,
    query: &circular_protocol::declaration_payload::Query,
    directory: &std::path::Path,
    open: &mut OpenQueries,
    sources: impl FnMut() -> crate::daemon::subscription::records::Sources,
) -> QueryResult {
    let Some(reader) = crate::daemon::query_catalog::registration(&query.name)
        .and_then(|registration| registration.handler().record_reader())
    else {
        return query_rejection(
            RejectionReason::Malformed,
            "query is not a registered immutable record reader",
        );
    };
    prepare_registered_record_query(correlation, query, reader, directory, open, sources)
}

pub(crate) fn prepare_registered_record_query(
    correlation: u32,
    query: &circular_protocol::declaration_payload::Query,
    reader: RecordQueryHandler,
    directory: &std::path::Path,
    open: &mut OpenQueries,
    mut sources: impl FnMut() -> crate::daemon::subscription::records::Sources,
) -> QueryResult {
    if let Err(rejection) = (reader.admit)(query) {
        return QueryResult::Rejected(rejection);
    }
    if open
        .authoring
        .iter()
        .any(|entry| entry.correlation == correlation)
    {
        return query_rejection(
            RejectionReason::Malformed,
            "query registration differs from the retained cursor",
        );
    }
    projection_page(correlation, query, &mut open.projections, || {
        (reader.capture)(query, directory, &mut sources)
            .map_err(|message| rejection(RejectionReason::Unresolved, &message))
    })
}

fn no_standing_pipeline_rejection() -> Rejected {
    let reason = RejectionReason::NoStandingPipeline;
    rejection(reason, reason.message())
}

fn query_result_encoding_failed() -> QueryResult {
    let reason = RejectionReason::QueryResultEncodingFailed;
    query_rejection(reason, reason.message())
}

fn query_result_encoding_failed_with_reason(error: &impl std::fmt::Debug) -> QueryResult {
    let mut result = query_result_encoding_failed();
    if let QueryResult::Rejected(rejection) = &mut result {
        rejection.hint = Some(format!("{error:?}").chars().take(512).collect());
    }
    result
}

/// Encode and write an owned query result after releasing World.
pub(crate) fn answer_query(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    result: QueryResult,
) {
    let body = match result.encode(Ceilings::for_boundary(Boundary::Wire)) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("circular-daemon: could not build a query result: {error:?}");
            query_result_encoding_failed_with_reason(&error)
                .encode(Ceilings::for_boundary(Boundary::Wire))
                .expect("the fixed query encoding rejection fits the wire ceilings")
        }
    };
    if let Err(error) = write_envelope(
        stream,
        OwnerLocalChannelId::new(1),
        header.protocol_version(),
        QUERY_RESULT,
        header.correlation(),
        &body,
    ) {
        eprintln!("circular-daemon: could not answer the query: {error}");
    }
}

fn agent_harnesses_page(
    standing: &[(
        circular_runtime::AgentHarnessName,
        circular_runtime::NormalizedPath,
    )],
    saved: &[crate::daemon::environment::AgentHarnessBinding],
) -> QueryPage {
    use circular_protocol::agent_harness_payload::{AgentHarnessRow, agent_harness_row_value};
    fn row<'a>(
        rows: &'a mut std::collections::BTreeMap<String, AgentHarnessRow>,
        name: &str,
    ) -> &'a mut AgentHarnessRow {
        rows.entry(name.to_owned())
            .or_insert_with(|| AgentHarnessRow {
                name: name.to_owned(),
                program: None,
                saved: None,
            })
    }
    let text = |path: &std::path::Path| path.to_string_lossy().into_owned();
    let mut rows = std::collections::BTreeMap::new();
    for (name, program) in standing {
        row(&mut rows, name.as_str()).program = Some(text(program.as_path()));
    }
    for binding in saved {
        row(&mut rows, binding.name()).saved = Some(text(binding.program()));
    }
    let items = rows
        .values()
        .map(agent_harness_row_value)
        .collect::<Vec<_>>();
    QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor: Value::Array(items.clone()),
        items,
        terminal: Terminal::Complete,
    }
}

fn agent_harness_candidates_page(
    detected: Vec<(
        &'static str,
        Option<crate::daemon::environment::AgentHarnessBinding>,
    )>,
) -> Result<QueryPage, String> {
    let items = detected
        .iter()
        .map(|(name, found)| {
            Value::object([
                ("name", Value::String((*name).to_owned())),
                (
                    "found",
                    found.as_ref().map_or(Value::Null, |binding| {
                        Value::String(binding.program().to_string_lossy().into_owned())
                    }),
                ),
            ])
            .map_err(|error| format!("agent harness candidate: {error:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor: Value::Null,
        items,
        terminal: Terminal::Complete,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct OpenAuthoringQuery {
    correlation: u32,
    snapshot: authoring::AuthoringSnapshot,
    cursor: QueryCursorFsm<
        &'static str,
        Option<Vec<u8>>,
        Vec<circular_protocol::declaration_payload::ScopeSegment>,
        usize,
    >,
}

pub(crate) fn authoring_snapshot_page(
    correlation: u32,
    query: circular_protocol::declaration_payload::Query,
    capture: impl FnOnce(
        Vec<circular_protocol::declaration_payload::ScopeSegment>,
    ) -> Result<authoring::AuthoringSnapshot, String>,
    open: &mut Vec<OpenAuthoringQuery>,
) -> QueryResult {
    let scope = match authoring_snapshot_scope(query.args) {
        Ok(scope) => scope,
        Err(message) => {
            return QueryResult::Rejected(
                RejectionReason::Malformed.reject(Partition::Query, message),
            );
        }
    };
    let (limit, supplied_cursor) = match query.page {
        None => (pages::DEFAULT_PAGE_LIMIT.get(), None),
        Some(page) => {
            let limit = match usize::try_from(page.limit.get()) {
                Ok(limit) => limit,
                Err(_) => {
                    return QueryResult::Rejected(RejectionReason::Malformed.reject(
                        Partition::Query,
                        "authoring-snapshot page limit exceeds this process".to_owned(),
                    ));
                }
            };
            let cursor = match page.cursor {
                None => None,
                Some(Value::Int(value)) if value >= 0 => match usize::try_from(value) {
                    Ok(offset) => Some(offset),
                    Err(_) => {
                        return QueryResult::Rejected(RejectionReason::Malformed.reject(
                            Partition::Query,
                            "authoring-snapshot cursor exceeds this process".to_owned(),
                        ));
                    }
                },
                Some(_) => {
                    return QueryResult::Rejected(RejectionReason::Malformed.reject(
                        Partition::Query,
                        "authoring-snapshot cursor must be a non-negative Int".to_owned(),
                    ));
                }
            };
            (limit, cursor)
        }
    };

    let existing = open
        .iter()
        .position(|entry| entry.correlation == correlation);
    let (snapshot, offset, is_new, mut cursor, request) = match (existing, supplied_cursor) {
        (None, None) => match capture(scope.clone()) {
            Ok(snapshot) => {
                let cursor = QueryCursorFsm::new(
                    "authoring-snapshot",
                    snapshot.authoring_revision.clone(),
                    scope.clone(),
                );
                (snapshot, 0, true, cursor, FsmPageStep::First { limit })
            }
            Err(rejection) => {
                return QueryResult::Rejected(
                    RejectionReason::Unresolved.reject(Partition::Query, rejection.to_string()),
                );
            }
        },
        (Some(index), Some(offset)) => {
            let entry = &open[index];
            (
                entry.snapshot.clone(),
                offset,
                false,
                entry.cursor.clone(),
                FsmPageStep::Continue {
                    limit,
                    cursor: FsmCursor::new(
                        entry.snapshot.authoring_revision.clone(),
                        scope.clone(),
                        offset,
                    ),
                },
            )
        }
        (Some(_), None) => {
            return QueryResult::Rejected(RejectionReason::Malformed.reject(
                Partition::Query,
                "authoring-snapshot restarted a live correlation without its cursor".to_owned(),
            ));
        }
        (None, Some(_)) => {
            return QueryResult::Rejected(RejectionReason::Malformed.reject(
                Partition::Query,
                "authoring-snapshot cursor has no retained immutable snapshot".to_owned(),
            ));
        }
    };

    if offset > snapshot.items.len() {
        return QueryResult::Rejected(RejectionReason::Malformed.reject(
            Partition::Query,
            "authoring-snapshot cursor is past the retained item list".to_owned(),
        ));
    }
    let end = offset.saturating_add(limit).min(snapshot.items.len());
    let more = end < snapshot.items.len();
    let page_end: FsmPageEnd<FsmCursor<Option<Vec<u8>>, Vec<_>, usize>, ()> = if more {
        FsmPageEnd::More {
            next: FsmCursor::new(snapshot.authoring_revision.clone(), scope, end),
        }
    } else {
        FsmPageEnd::Complete
    };
    if let Err(reason) = cursor.page(&"authoring-snapshot", &request, page_end) {
        return QueryResult::Rejected(RejectionReason::Malformed.reject(
            Partition::Query,
            format!("authoring-snapshot cursor transition rejected: {reason:?}"),
        ));
    }
    if more {
        if is_new {
            open.push(OpenAuthoringQuery {
                correlation,
                snapshot: snapshot.clone(),
                cursor,
            });
        } else if let Some(index) = existing {
            open[index].cursor = cursor;
        }
    } else if let Some(index) = existing {
        open.remove(index);
    }

    let anchor = match authoring_snapshot_anchor(&snapshot) {
        Ok(anchor) => anchor,
        Err(message) => {
            return QueryResult::Rejected(
                RejectionReason::Malformed.reject(Partition::Query, message),
            );
        }
    };
    QueryResult::Page(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor,
        items: snapshot.items[offset..end].to_vec(),
        terminal: if more {
            Terminal::More(Value::Int(i64::try_from(end).unwrap_or(i64::MAX)))
        } else {
            Terminal::Complete
        },
    })
}

fn authoring_snapshot_scope(
    value: Value,
) -> Result<Vec<circular_protocol::declaration_payload::ScopeSegment>, String> {
    let Value::Object(object) = value else {
        return Err("authoring-snapshot args must be an object containing scope".to_owned());
    };
    let mut fields = object.into_map();
    let scope = fields
        .remove("scope")
        .ok_or_else(|| "authoring-snapshot args.scope is required".to_owned())?;
    if let Some((unexpected, _)) = fields.into_iter().next() {
        return Err(format!(
            "authoring-snapshot args carries an unknown field {unexpected:?}"
        ));
    }
    decode_scope_identity(scope).map_err(|error| format!("invalid snapshot scope: {error}"))
}

fn authoring_snapshot_anchor(snapshot: &authoring::AuthoringSnapshot) -> Result<Value, String> {
    let cursor = i64::try_from(snapshot.cursor)
        .map_err(|_| "authoring snapshot cursor exceeds the Int carrier".to_owned())?;
    Value::object([
        (
            "authoring_revision",
            current_revision_value(snapshot.authoring_revision.as_deref()),
        ),
        ("cursor", Value::Int(cursor)),
        ("environment", environment_value(&snapshot.environment)?),
        ("scope", scope_identity_value(&snapshot.scope)),
        (
            "topology_revision",
            current_revision_value(snapshot.topology_revision.as_deref()),
        ),
    ])
    .map_err(|error| format!("authoring snapshot anchor: {error:?}"))
}
 #[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::{agent_harnesses_page, authoring_snapshot_page};
    use circular_core::Value;
    use circular_protocol::declaration_payload::{
        ActorDeclaration, ActorFlags, AddressRef, AuthoredLocal, AuthoringEnvironment, BeginEpoch,
        ExpectedRevision, PageRequest, PlanActorKey, Query, QueryResult, Terminal,
    };
    use engine::authoring_assembly::ledger as authoring;

    fn actor(local: &str) -> PlanActorKey {
        PlanActorKey {
            scope: Vec::new(),
            local: AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        }
    }

    #[test]
    fn no_configured_agent_harnesses_is_a_successful_empty_page() {
        let page = agent_harnesses_page(&[], &[]);
        assert_eq!(page.anchor, Value::Array(Vec::new()));
        assert!(page.items.is_empty());
        assert!(matches!(page.terminal, Terminal::Complete));
    }
}
