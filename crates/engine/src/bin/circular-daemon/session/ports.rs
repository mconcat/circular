//! Product handlers behind the six existing semantic ports.
use super::domain::Domain;
use super::*;
use crate::daemon::subscription::FrameCredits;
use circular_protocol::rejection_code::RejectionReason;
use circular_protocol::{self as p, declaration_payload as w};
pub(super) struct DeclarationPort<'a> {
    pub payload: &'a [u8],
    pub world: &'a crate::daemon::read_world::SharedWorld,
    pub store: &'a authoring_store::AuthoringStore,
    pub execution: &'a ProductExecutionProfile,
    pub candidate: &'a mut SessionEpoch,
}
pub(super) struct InjectionPort<'a> {
    pub world: &'a crate::daemon::read_world::SharedWorld,
}
pub(super) struct TransitionPort<'a> {
    pub world: &'a crate::daemon::read_world::SharedWorld,
    pub execution: &'a ProductExecutionProfile,
}
pub(super) struct LifecyclePort<'a> {
    pub world: &'a crate::daemon::read_world::SharedWorld,
    pub execution: &'a ProductExecutionProfile,
}
pub(super) struct QueryPort<'a> {
    pub header: EnvelopeHeader,
    pub world: &'a crate::daemon::read_world::SharedWorld,
    pub store: &'a authoring_store::AuthoringStore,
    pub execution: &'a ProductExecutionProfile,
    pub queries: &'a mut OpenQueries,
    pub lenses: &'a Lenses,
}
pub(super) struct SubscriptionPort<'a, S> {
    pub stream: &'a mut S,
    pub world: &'a crate::daemon::read_world::SharedWorld,
    pub store: &'a authoring_store::AuthoringStore,
    pub subscriptions: &'a mut Subscriptions,
    pub lenses: &'a Lenses,
    pub wake: &'a std::sync::Arc<engine::wake::Wake>,
}
fn semantic(result: w::CommandResult) -> p::CommandResult<w::Accepted, w::Rejected> {
    match result {
        w::CommandResult::Accepted(a) => p::CommandResult::Accepted(a),
        w::CommandResult::Rejected(r) => p::CommandResult::Rejected(r),
    }
}
impl p::DeclarationPort<u32> for DeclarationPort<'_> {
    type Domain = Domain;
    type Accepted = w::Accepted;
    type Rejection = w::Rejected;
    fn dispatch_declaration(
        &mut self,
        _: &p::CorrelationId<u32>,
        command: domain::Command,
    ) -> p::CommandResult<Self::Accepted, Self::Rejection> {
        let Self {
            payload,
            world,
            store,
            execution,
            candidate,
        } = self;
        let result = match command {
            domain::Command::CommitEpoch { epoch } => {
                crate::daemon::declaration::commit_decoded_epoch_outside_world(
                    epoch
                        .as_deref()
                        .expect("session admission supplied an epoch reference"),
                    candidate.take_active(),
                    world,
                    store,
                    execution,
                )
            }
            command => {
                let prefix = world.read();
                let mut authoring = prefix.authoring.as_ref().clone();
                super::dispatch_candidate_declaration(
                    payload,
                    command,
                    candidate,
                    &mut authoring,
                    execution,
                )
            }
        };
        semantic(result)
    }
}
impl p::EventInjectionPort<u32> for InjectionPort<'_> {
    type Mount = w::PlanExportKey;
    type Payload = Value;
    type IdempotencyKey = Vec<u8>;
    type Accepted = w::Accepted;
    type Rejection = w::Rejected;
    fn dispatch_injection(
        &mut self,
        _: &p::CorrelationId<u32>,
        i: p::Injection<w::PlanExportKey, Value, Vec<u8>>,
    ) -> p::CommandResult<Self::Accepted, Self::Rejection> {
        semantic(
            self.world
                .inject_decoded(w::Inject {
                    mount: i.mount().clone(),
                    payload: i.payload().clone(),
                    idempotency: i.idempotency().clone(),
                })
                .into_command_result(),
        )
    }
}
impl p::LedgerTransitionPort<u32> for TransitionPort<'_> {
    type Domain = Domain;
    type Accepted = w::Accepted;
    type Rejection = w::Rejected;
    fn dispatch_ledger_transition(
        &mut self,
        _: &p::CorrelationId<u32>,
        transition: p::LedgerTransition<Domain>,
    ) -> p::CommandResult<Self::Accepted, Self::Rejection> {
        match transition {
            p::LedgerTransition::ApprovalDecide { item, decision } => {
                let prefix = self.world.read();
                let owner = prefix.server.as_ref().and_then(|run| run.approval_owner());
                let answer =
                    crate::daemon::run_control::decide_decoded_approval(item, decision, owner);
                semantic(answer)
            }
            p::LedgerTransition::SetObservationControl { .. } => {
                p::CommandResult::Rejected(RejectionReason::Unresolved.reject(
                    p::Partition::LedgerTransition,
                    "observation control is unavailable",
                ))
            }
            p::LedgerTransition::SetAgentHarness { harness } => semantic(
                crate::daemon::run_control::set_agent_harness(self.world, self.execution, harness),
            ),
        }
    }
}
impl p::LifecyclePort<u32> for LifecyclePort<'_> {
    type Domain = p::lifecycle_payload::WireLifecycleDomain;
    type Rejection = w::Rejected;
    fn dispatch_lifecycle(
        &mut self,
        _: &p::CorrelationId<u32>,
        control: p::Lifecycle<Self::Domain>,
    ) -> p::CommandResult<p::LifecycleAccepted, w::Rejected> {
        let result = match control {
            p::Lifecycle::Resume {
                expected_authoring_revision,
            } => crate::daemon::run_control::start(
                self.world,
                self.execution,
                expected_authoring_revision,
            ),
            p::Lifecycle::Pause { mode } => crate::daemon::run_control::stop(self.world, mode),
        };
        match result {
            p::LifecycleResult::Accepted(a) => p::CommandResult::Accepted(a),
            p::LifecycleResult::Rejected(r) => p::CommandResult::Rejected(r),
        }
    }
}
impl p::QueryPort<u32> for QueryPort<'_> {
    type Name = String;
    type Arguments = Value;
    type Limit = Option<std::num::NonZeroU64>;
    type Cursor = Value;
    type Since = p::replay_payload::LogCut;
    type Page = p::QueryPageOf<Domain>;
    type Rejection = w::Rejected;
    fn dispatch_query(
        &mut self,
        _: &p::CorrelationId<u32>,
        q: p::QueryRequestOf<Domain>,
    ) -> p::QueryResult<Self::Page, Self::Rejection> {
        let page = match q.page() {
            p::PageStep::First { limit } => limit.map(|limit| w::PageRequest {
                limit,
                cursor: None,
            }),
            p::PageStep::Continue { limit, cursor } => limit.map(|limit| w::PageRequest {
                limit,
                cursor: Some(cursor.clone()),
            }),
        };
        let request = w::Query {
            since: q.since().cloned(),
            upto: q.upto().cloned(),
            name: q.name().clone(),
            args: q.arguments().clone(),
            page,
            lens: q.lens(),
        };
        let Self {
            header,
            world,
            store,
            execution,
            queries,
            lenses,
            ..
        } = self;
        if request.upto.is_some() && request.lens.is_some() {
            return semantic_query(w::QueryResult::Rejected(RejectionReason::Malformed.reject(
                p::Partition::Query,
                "a read carries one bound: upto or lens, not both".to_owned(),
            )));
        }
        let replay = match lenses.named(request.lens) {
            Ok(lens) => lens,
            Err(message) => {
                return semantic_query(w::QueryResult::Rejected(
                    RejectionReason::Unresolved.reject(p::Partition::Query, message),
                ));
            }
        };
        let registration = crate::daemon::query_catalog::registration(&request.name);
        let result = if let Some(reader) =
            registration.and_then(|registration| registration.handler().record_reader())
        {
            let prefix = world.read();
            match crate::daemon::query::record_sources(
                crate::daemon::subscription::records::Sources::of(&prefix, store),
                prefix.server.as_deref(),
                replay,
                request.upto.as_ref(),
            ) {
                Ok(sources) => crate::daemon::query::prepare_registered_record_query(
                    header.correlation(),
                    &request,
                    reader,
                    store.state_directory(),
                    queries,
                    || sources.clone(),
                ),
                Err(message) => w::QueryResult::Rejected(
                    RejectionReason::Unresolved.reject(p::Partition::Query, message),
                ),
            }
        } else {
            let prefix = world.read();
            let crate::daemon::read_world::WorldRead {
                authoring,
                server,
                system,
                state_directory,
                ..
            } = &*prefix;
            crate::daemon::query::prepare_registered_query(
                *header,
                request,
                registration,
                QueryRequestContext {
                    server: server.as_deref(),
                    replay_lens: replay,
                    authoring,
                    system: system.as_deref(),
                    authoring_store: store,
                    execution,
                    state_directory,
                    authoring_queries: queries,
                },
            )
        };
        semantic_query(result)
    }

    fn close_query(
        &mut self,
        _: &p::CorrelationId<u32>,
    ) -> p::QueryResult<Self::Page, Self::Rejection> {
        semantic_query(self.queries.close(self.header.correlation()))
    }
}
fn semantic_query(result: w::QueryResult) -> p::QueryResult<p::QueryPageOf<Domain>, w::Rejected> {
    match result {
        w::QueryResult::Rejected(r) => p::QueryResult::Rejected(r),
        w::QueryResult::Page(page) => {
            let end = match page.terminal {
                w::Terminal::Complete => p::PageEnd::Complete,
                w::Terminal::More(next) => p::PageEnd::More { next },
                w::Terminal::Diagnostic(code) => p::PageEnd::Diagnostic { code },
            };
            p::QueryResult::Accepted(p::QueryPage::new(
                page.anchor,
                page.items,
                end,
                page.cut,
                page.folded_from,
                page.reached,
            ))
        }
    }
}
impl<S: circular_transport::LocalByteStream<Error = std::io::Error>> p::SubscriptionPort<u32>
    for SubscriptionPort<'_, S>
{
    type Target = p::subscription_payload::Subscribe;
    type Credit = FrameCredits;
    type Opened = w::Accepted;
    type Credited = w::Accepted;
    type Closed = w::Accepted;
    type Rejection = w::Rejected;
    fn dispatch_subscribe(
        &mut self,
        correlation: &p::CorrelationId<u32>,
        s: p::Subscribe<Self::Target>,
    ) -> p::CommandResult<Self::Opened, Self::Rejection> {
        let s = s.target();
        let correlation = *correlation.value();
        let Self {
            store,
            world,
            subscriptions,
            lenses,
            wake,
            ..
        } = self;
        if crate::daemon::subscription_catalog::resolve(&s.target).is_some_and(|target| {
            target.answered_by() == crate::daemon::subscription_catalog::AnsweredBy::Actors
        }) {
            let system = world.read().system.clone();
            return semantic(subscriptions.slot(correlation, |live| {
                crate::daemon::subscription::edge_depths::open(
                    s,
                    correlation,
                    system.as_deref(),
                    wake,
                    live,
                )
            }));
        }
        let replay = match lenses.named(s.lens) {
            Ok(lens) => lens,
            Err(message) => {
                return p::CommandResult::Rejected(
                    RejectionReason::Unresolved.reject(p::Partition::Subscription, message),
                );
            }
        };
        if crate::daemon::subscription_catalog::resolve(&s.target).is_some_and(|target| {
            target.answered_by() == crate::daemon::subscription_catalog::AnsweredBy::StateJournal
        }) {
            let sources = match crate::daemon::subscription::records::Sources::of_read(
                &world.read(),
                store,
                replay,
            ) {
                Ok(sources) => sources,
                Err(message) => {
                    return p::CommandResult::Rejected(
                        RejectionReason::Unresolved.reject(p::Partition::Subscription, message),
                    );
                }
            };
            return semantic(subscriptions.slot(correlation, |live| {
                crate::daemon::subscription::records::open(
                    store.journal_path(),
                    s.args.clone(),
                    &sources,
                    correlation,
                    s.lens,
                    live,
                )
            }));
        }
        let world = world.read();
        let retained = store.retained();
        semantic(subscriptions.slot(correlation, |live| {
            crate::daemon::subscription::subscribe_decoded(
                s.clone(),
                correlation,
                world.server.as_deref(),
                &world.authoring,
                &retained,
                live,
                replay,
            )
        }))
    }
    fn dispatch_credit(
        &mut self,
        correlation: &p::CorrelationId<u32>,
        n: p::PositiveCredit<FrameCredits>,
    ) -> p::CommandResult<Self::Credited, Self::Rejection> {
        let Self {
            stream,
            world,
            store,
            subscriptions,
            lenses,
            ..
        } = self;
        subscriptions.slot(*correlation.value(), |live| {
            let lens = match live.as_ref().and_then(Subscription::lens) {
                None => None,
                Some(key) => lenses.get(key),
            };
            credit_one(*stream, world, store, n, live, lens)
        })
    }
    fn dispatch_unsubscribe(
        &mut self,
        correlation: &p::CorrelationId<u32>,
    ) -> p::CommandResult<Self::Closed, Self::Rejection> {
        semantic(self.subscriptions.slot(*correlation.value(), |live| {
            crate::daemon::subscription::unsubscribe_live(live)
        }))
    }
}

fn credit_one<S: circular_transport::LocalByteStream<Error = std::io::Error>>(
    stream: &mut S,
    world: &crate::daemon::read_world::SharedWorld,
    store: &authoring_store::AuthoringStore,
    credit: p::PositiveCredit<FrameCredits>,
    live: &mut Option<Subscription>,
    lens: Option<&ReplayLens>,
) -> p::CommandResult<w::Accepted, w::Rejected> {
    let result = crate::daemon::subscription::grant_credit(credit, live);
    semantic(match result {
        w::CommandResult::Accepted(_) => {
            let prefix = world.read();
            let retained = store.retained();
            let bounded_records =
                crate::daemon::subscription::records::Sources::of_read(&prefix, store, lens);
            if bounded_records.is_err()
                && matches!(
                    live,
                    Some(Subscription {
                        source: crate::daemon::subscription::SubscriptionSource::Records { .. },
                        ..
                    })
                )
            {
                return semantic(w::CommandResult::Accepted(w::Accepted::Transition(
                    Value::object([("pending_after", Value::UInt(0))]).unwrap(),
                )));
            }
            let records = bounded_records.unwrap_or_else(|_| {
                crate::daemon::subscription::records::Sources::of(&prefix, store)
            });
            let pending_after = pump_subscription(
                stream,
                live,
                prefix.server.as_deref(),
                &prefix.authoring,
                &retained,
                &records,
                lens,
            );
            w::CommandResult::Accepted(w::Accepted::Transition(
                Value::object([("pending_after", Value::UInt(pending_after))]).unwrap(),
            ))
        }
        rejected => rejected,
    })
}
