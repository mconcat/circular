
use crate::{
    AgentInvokeSpec, ApprovalConsumeRejection, ApprovalTicket, Capability, Effect, EffectFailure,
    FileReadSpec, FileWriteMode, FileWriteSpec, FsReadGrant, FsWriteGrant, HttpRequestSpec,
    InterpreterFault, NotificationSpec, OutcomePayload, PathScopes, ProcessSpec,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectTerm {
    Http {
        ticket: Option<ApprovalTicket>,
        spec: HttpRequestSpec,
    },
    FileRead {
        ticket: Option<ApprovalTicket>,
        spec: FileReadSpec,
    },
    FileWrite {
        ticket: Option<ApprovalTicket>,
        spec: FileWriteSpec,
    },
    Spawn {
        ticket: Option<ApprovalTicket>,
        spec: ProcessSpec,
    },
    Notify {
        ticket: Option<ApprovalTicket>,
        spec: NotificationSpec,
    },
    AgentInvoke {
        ticket: Option<ApprovalTicket>,
        invoke: AgentInvokeSpec,
    },
    RequestApproval(crate::ApprovalSpec),
    Peer(crate::PeerEffectTerm),
}

impl EffectTerm {
    #[must_use]
    pub const fn constructor(&self) -> crate::EffectCtor {
        use crate::{EffectCtor, PeerEffectTerm};
        match self {
            Self::Http { .. } => EffectCtor::Http,
            Self::FileRead { .. } => EffectCtor::FileRead,
            Self::FileWrite { .. } => EffectCtor::FileWrite,
            Self::Spawn { .. } => EffectCtor::Spawn,
            Self::Notify { .. } => EffectCtor::Notify,
            Self::AgentInvoke { .. } => EffectCtor::AgentInvoke,
            Self::RequestApproval(_) => EffectCtor::RequestApproval,
            Self::Peer(term) => match term {
                PeerEffectTerm::Discover(_) => EffectCtor::PeerDiscover,
                PeerEffectTerm::Bind(_) => EffectCtor::PeerBind,
                PeerEffectTerm::Send(_) => EffectCtor::PeerSend,
                PeerEffectTerm::Unbind(_) => EffectCtor::PeerUnbind,
                PeerEffectTerm::Receive { .. } => EffectCtor::PeerReceive,
            },
        }
    }

    #[must_use]
    pub fn from_effect<E>(effect: E) -> Option<Self>
    where
        Option<Self>: From<E>,
    {
        effect.into()
    }

    #[must_use]
    pub fn from_recorded(effect: &Effect) -> Option<Self> {
        match effect {
            Effect::RequestApproval { spec, .. } => Some(Self::RequestApproval(spec.clone())),
            other => Self::from_effect(other),
        }
    }
}

impl From<&Effect> for Option<EffectTerm> {
    fn from(effect: &Effect) -> Self {
        match effect {
            Effect::Http { ticket, spec, .. } => Some(EffectTerm::Http {
                ticket: ticket.clone(),
                spec: spec.clone(),
            }),
            Effect::FileRead { ticket, spec, .. } => Some(EffectTerm::FileRead {
                ticket: ticket.clone(),
                spec: spec.clone(),
            }),
            Effect::FileWrite { ticket, spec, .. } => Some(EffectTerm::FileWrite {
                ticket: ticket.clone(),
                spec: spec.clone(),
            }),
            Effect::Spawn { ticket, spec, .. } => Some(EffectTerm::Spawn {
                ticket: ticket.clone(),
                spec: spec.clone(),
            }),
            Effect::Notify { ticket, spec, .. } => Some(EffectTerm::Notify {
                ticket: ticket.clone(),
                spec: spec.clone(),
            }),
            Effect::AgentInvoke { ticket, invoke, .. } => Some(EffectTerm::AgentInvoke {
                ticket: ticket.clone(),
                invoke: invoke.clone(),
            }),
            Effect::RequestApproval { .. }
            | Effect::MutateInstance { .. }
            | Effect::Schedule { .. } => None,
        }
    }
}

impl From<&crate::PeerEffect> for Option<EffectTerm> {
    fn from(effect: &crate::PeerEffect) -> Self {
        Some(EffectTerm::Peer(effect.into()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmitError<I> {
    DuplicateEffectId(I),
    ProgramNotExecutable,
}

pub trait Interpreter<I> {
    fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>>;

    fn submit_peer(
        &mut self,
        correlation: I,
        effect: &crate::PeerEffect,
    ) -> Result<(), SubmitError<I>>;

    fn next_outcome(&mut self) -> Option<crate::EffectOutcome<I>>;

    /// Register the existing completion owner's wake. Asynchronous interpreters
    /// wake after enqueuing an outcome; synchronous submissions are already ready.
    fn set_outcome_waker(&mut self, _waker: std::task::Waker) {}

    /// Begin the canonical `Draining` cancellation phase for work accepted by
    /// this interpreter.
    ///
    /// Synchronous interpreters have no work left after `submit` returns and
    /// therefore need no implementation. Asynchronous interpreters override
    /// this hook to stop their external handles and must not begin new work
    /// after it is called. Already-produced outcomes remain available for the
    /// owner to settle or discard at the run boundary.
    fn begin_cancel(&mut self) {}

    /// Accepted local subprocess results still owned by this interpreter.
    /// Process shutdown reaps only these results, never human decisions or clocks.
    fn pending_process_outcomes(&self) -> usize {
        0
    }

    fn pause(&mut self, _force: bool) {}

    /// Reopen intake after every accepted pause outcome has been settled.
    /// This does not undo the terminal `begin_cancel` lifecycle.
    fn resume(&mut self) {}

    fn next_outcome_with_detail(&mut self) -> Option<(crate::EffectOutcome<I>, Option<String>)> {
        self.next_outcome().map(|outcome| (outcome, None))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalGateDisposition {
    Required,
    Authorized,
    Rejected(ApprovalConsumeRejection),
}

pub trait ApprovalGate {
    fn authorize(
        &mut self,
        target_effect: crate::EffectId,
        ticket: Option<&ApprovalTicket>,
    ) -> ApprovalGateDisposition;
}

pub struct LiveInterpreter<I> {
    read_scopes: PathScopes,
    write_scopes: PathScopes,
    submitted: BTreeSet<I>,
    settled: VecDeque<crate::EffectOutcome<I>>,
    external_contacts: usize,
}

impl<I: Clone + Ord> LiveInterpreter<I> {
    #[must_use]
    pub fn new(read_grant: &FsReadGrant, write_grant: &FsWriteGrant) -> Self {
        Self {
            read_scopes: read_grant.parameters().clone(),
            write_scopes: write_grant.parameters().clone(),
            submitted: BTreeSet::new(),
            settled: VecDeque::new(),
            external_contacts: 0,
        }
    }

    pub fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>> {
        if !self.submitted.insert(correlation.clone()) {
            return Err(SubmitError::DuplicateEffectId(correlation));
        }
        let result = self.perform(effect);
        self.settled
            .push_back(crate::EffectOutcome::new(correlation, result));
        Ok(())
    }

    pub fn submit_peer(
        &mut self,
        correlation: I,
        _effect: &crate::PeerEffect,
    ) -> Result<(), SubmitError<I>> {
        if !self.submitted.insert(correlation.clone()) {
            return Err(SubmitError::DuplicateEffectId(correlation));
        }
        self.settled.push_back(crate::EffectOutcome::new(
            correlation,
            Err(EffectFailure::EndpointGone),
        ));
        Ok(())
    }

    #[must_use]
    pub fn next_outcome(&mut self) -> Option<crate::EffectOutcome<I>> {
        self.settled.pop_front()
    }

    #[must_use]
    pub const fn external_contacts(&self) -> usize {
        self.external_contacts
    }

    fn perform(&mut self, effect: &Effect) -> Result<OutcomePayload, EffectFailure> {
        match effect {
            Effect::FileRead { spec, .. } => self.read_file(spec),
            Effect::FileWrite { spec, .. } => self.write_file(spec),
            Effect::Http { .. } => Err(EffectFailure::EndpointGone),
            Effect::Spawn { .. } => Err(EffectFailure::EndpointGone),
            Effect::AgentInvoke { .. } => Err(EffectFailure::EndpointGone),
            Effect::Notify { .. } => Err(EffectFailure::EndpointGone),
            Effect::RequestApproval { .. } => Err(EffectFailure::EndpointGone),
            Effect::MutateInstance { .. } | Effect::Schedule { .. } => {
                Err(EffectFailure::EndpointGone)
            }
        }
    }

    fn read_file(&mut self, spec: &FileReadSpec) -> Result<OutcomePayload, EffectFailure> {
        let path = scoped_file_path(&self.read_scopes, spec.path(), Capability::FsRead, false)?;
        self.external_contacts += 1;
        let mut file = File::open(path).map_err(io_failure)?;
        let mut bytes = Vec::new();
        if let Some(range) = spec.range() {
            file.seek(SeekFrom::Start(range.start()))
                .map_err(io_failure)?;
            file.take(range.length())
                .read_to_end(&mut bytes)
                .map_err(io_failure)?;
        } else {
            file.read_to_end(&mut bytes).map_err(io_failure)?;
        }
        Ok(OutcomePayload::FileBytes(bytes.into_boxed_slice()))
    }

    fn write_file(&mut self, spec: &FileWriteSpec) -> Result<OutcomePayload, EffectFailure> {
        let path = scoped_file_path(&self.write_scopes, spec.path(), Capability::FsWrite, true)?;
        self.external_contacts += 1;
        let mut options = OpenOptions::new();
        options.write(true);
        match spec.mode() {
            FileWriteMode::Create => {
                options.create_new(true);
            }
            FileWriteMode::Replace => {
                options.create(true).truncate(true);
            }
        }
        let mut file = options.open(path).map_err(io_failure)?;
        file.write_all(spec.body()).map_err(io_failure)?;
        let written = u64::try_from(spec.body().len())
            .map_err(|_| EffectFailure::InterpreterFault(InterpreterFault::ResourceExhausted))?;
        Ok(OutcomePayload::WrittenLength(written))
    }
}

impl<I: Clone + Ord> Interpreter<I> for LiveInterpreter<I> {
    fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>> {
        LiveInterpreter::submit(self, correlation, effect)
    }

    fn submit_peer(
        &mut self,
        correlation: I,
        effect: &crate::PeerEffect,
    ) -> Result<(), SubmitError<I>> {
        LiveInterpreter::submit_peer(self, correlation, effect)
    }

    fn next_outcome(&mut self) -> Option<crate::EffectOutcome<I>> {
        LiveInterpreter::next_outcome(self)
    }
}

fn scoped_file_path(
    scopes: &PathScopes,
    path: &crate::NormalizedPath,
    capability: Capability,
    create: bool,
) -> Result<std::path::PathBuf, EffectFailure> {
    let denied = || EffectFailure::ParameterDenied { capability };
    if !scopes.allows(path) {
        return Err(denied());
    }
    let resolved = match std::fs::canonicalize(path.as_path()) {
        Ok(path) => path,
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path.as_path()) {
                Ok(_) => return Err(io_failure(error)),
                Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_failure(error)),
            }
            let parent = path.as_path().parent().ok_or_else(denied)?;
            std::fs::canonicalize(parent)
                .map_err(io_failure)?
                .join(path.as_path().file_name().ok_or_else(denied)?)
        }
        Err(error) => return Err(io_failure(error)),
    };
    if !scopes.iter().any(|scope| {
        scope.contains(path)
            && std::fs::canonicalize(scope.root().as_path())
                .is_ok_and(|root| resolved.starts_with(root))
    }) {
        return Err(denied());
    }
    Ok(resolved)
}

fn io_failure(error: std::io::Error) -> EffectFailure {
    EffectFailure::InterpreterFault(error.kind().into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedOutcome<I> {
    term: EffectTerm,
    outcome: crate::EffectOutcome<I>,
}

impl<I> RecordedOutcome<I> {
    #[must_use]
    pub const fn new(term: EffectTerm, outcome: crate::EffectOutcome<I>) -> Self {
        Self { term, outcome }
    }

    #[must_use]
    pub const fn term(&self) -> &EffectTerm {
        &self.term
    }

    #[must_use]
    pub const fn outcome(&self) -> &crate::EffectOutcome<I> {
        &self.outcome
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VirtualizedBuildError<I> {
    DuplicateEffectId(I),
}

/// A virtualized replay reached its terminal history boundary with unmatched
/// recorded outcomes.
///
/// The full records are retained so a comparison layer can project their
/// effect terms and outcome payloads. Their slice order only mirrors the
/// iterator supplied to [`VirtualizedInterpreter::try_new`]; it is not a
/// canonical or causal order. Product comparison must recover ordering from
/// persisted record metadata instead of this diagnostic sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualizedHistoryResidual<I> {
    unmatched_records: Box<[RecordedOutcome<I>]>,
}

impl<I> VirtualizedHistoryResidual<I> {
    #[must_use]
    pub const fn unmatched_records(&self) -> &[RecordedOutcome<I>] {
        &self.unmatched_records
    }
}

impl<I> std::fmt::Display for VirtualizedHistoryResidual<I> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "virtualized replay history ended with {} unmatched record(s)",
            self.unmatched_records.len()
        )
    }
}

impl<I: std::fmt::Debug> std::error::Error for VirtualizedHistoryResidual<I> {}

pub struct VirtualizedInterpreter<I> {
    recorded: BTreeMap<I, RecordedOutcome<I>>,
    /// Stable copy of the caller's enumeration used only to return every
    /// unmatched full record without imposing `BTreeMap` key order. This is
    /// deliberately not an admission condition or product comparison order.
    supplied_enumeration: Box<[I]>,
    submitted: BTreeSet<I>,
    matched: BTreeSet<I>,
    settled: VecDeque<crate::EffectOutcome<I>>,
}

impl<I: Clone + Ord> VirtualizedInterpreter<I> {
    pub fn try_new(
        recorded: impl IntoIterator<Item = RecordedOutcome<I>>,
    ) -> Result<Self, VirtualizedBuildError<I>> {
        let mut by_id = BTreeMap::new();
        let mut supplied_enumeration = Vec::new();
        for item in recorded {
            let id = item.outcome.correlation().clone();
            if by_id.insert(id.clone(), item).is_some() {
                return Err(VirtualizedBuildError::DuplicateEffectId(id));
            }
            supplied_enumeration.push(id);
        }
        Ok(Self {
            recorded: by_id,
            supplied_enumeration: supplied_enumeration.into_boxed_slice(),
            submitted: BTreeSet::new(),
            matched: BTreeSet::new(),
            settled: VecDeque::new(),
        })
    }

    pub fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>> {
        self.submit_term(correlation, EffectTerm::from_recorded(effect))
    }

    pub fn submit_peer(
        &mut self,
        correlation: I,
        effect: &crate::PeerEffect,
    ) -> Result<(), SubmitError<I>> {
        self.submit_term(correlation, EffectTerm::from_effect(effect))
    }

    fn submit_term(
        &mut self,
        correlation: I,
        term: Option<EffectTerm>,
    ) -> Result<(), SubmitError<I>> {
        if !self.submitted.insert(correlation.clone()) {
            return Err(SubmitError::DuplicateEffectId(correlation));
        }
        let result = match term {
            None => Err(EffectFailure::EndpointGone),
            Some(submitted_term) => match self.recorded.get(&correlation) {
                Some(record) if record.term == submitted_term => {
                    self.matched.insert(correlation.clone());
                    record.outcome.result().clone()
                }
                Some(_) => Err(EffectFailure::Diverged(crate::Divergence::EffectMismatch)),
                None => Err(EffectFailure::Diverged(crate::Divergence::MissingRecord)),
            },
        };
        self.settled
            .push_back(crate::EffectOutcome::new(correlation, result));
        Ok(())
    }

    #[must_use]
    pub fn next_outcome(&mut self) -> Option<crate::EffectOutcome<I>> {
        self.settled.pop_front()
    }

    /// Borrow every currently unmatched full record.
    ///
    /// Iteration mirrors supplied enumeration only; product comparison must
    /// order these records from persisted metadata.
    pub fn residual_records(&self) -> impl Iterator<Item = &RecordedOutcome<I>> {
        self.supplied_enumeration
            .iter()
            .filter(|id| !self.matched.contains(*id))
            .map(|id| {
                self.recorded
                    .get(id)
                    .expect("supplied enumeration and recorded lookup remain aligned")
            })
    }

    /// Consume the replay interpreter and prove only that every recorded
    /// effect/outcome pair was matched by an exact `(EffectId, EffectTerm)`
    /// lookup.
    ///
    /// This is a history-completeness check, not a router/frontier settlement
    /// acknowledgement. It intentionally ignores the interpreter's outcome
    /// FIFO: dequeueing an outcome cannot prove that the runtime admitted and
    /// routed it, while a caller that already did so does not need to feed that
    /// fact back into this lookup primitive.
    ///
    /// On a strict prefix, every unmatched full record is returned. The result
    /// preserves supplied iterator order only to avoid losing information;
    /// product comparison must use persisted record metadata for canonical
    /// ordering.
    pub fn finish_history(mut self) -> Result<(), VirtualizedHistoryResidual<I>> {
        let unmatched_records = self
            .supplied_enumeration
            .into_vec()
            .into_iter()
            .filter(|id| !self.matched.contains(id))
            .map(|id| {
                self.recorded
                    .remove(&id)
                    .expect("supplied enumeration and recorded lookup remain aligned")
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        if unmatched_records.is_empty() {
            Ok(())
        } else {
            Err(VirtualizedHistoryResidual { unmatched_records })
        }
    }
}

impl<I: Clone + Ord> Interpreter<I> for VirtualizedInterpreter<I> {
    fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>> {
        VirtualizedInterpreter::submit(self, correlation, effect)
    }

    fn submit_peer(
        &mut self,
        correlation: I,
        effect: &crate::PeerEffect,
    ) -> Result<(), SubmitError<I>> {
        VirtualizedInterpreter::submit_peer(self, correlation, effect)
    }

    fn next_outcome(&mut self) -> Option<crate::EffectOutcome<I>> {
        VirtualizedInterpreter::next_outcome(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentHarnessGrant, AgentHarnessName, AgentPayload, AgentProgressRecord, AgentSessionId,
        AgentStepNext, AgentStepRequest, AgentStepResult, AgentToolCall, AgentToolCallId,
        AgentToolResult, ConcreteExternalEffectTag, ConcreteOutcomePayload, FileWriteSpec,
        FsWriteGrant, GrantIssuer, NormalizedPath, PathScope, ToolName,
    };

    fn assert_interpreter<T: Interpreter<u64>>() {}

    #[test]
    fn live_and_virtualized_modes_share_the_submit_then_settle_port() {
        assert_interpreter::<LiveInterpreter<u64>>();
        assert_interpreter::<VirtualizedInterpreter<u64>>();
    }

    #[test]
    fn http_term_round_trips_through_virtualized_replay_without_an_executor() {
        use crate::{
            GrantIssuer, HttpFetchGrant, HttpHosts, HttpMethod, HttpRequestSpec, HttpResponse,
            HttpUrl,
        };

        let grant = HttpFetchGrant::http_fetch(HttpHosts::exact(["example.test"]));
        let effect = Effect::http(
            GrantIssuer::new().issue(&grant),
            None,
            HttpRequestSpec::try_new(
                HttpMethod::Get,
                HttpUrl::try_new("https://example.test/data").expect("absolute URL"),
                [],
                [],
            )
            .expect("bodyless GET"),
        );
        let term = EffectTerm::from_effect(&effect).expect("HTTP is an external term");
        assert_eq!(term, EffectTerm::from_recorded(&effect).unwrap());

        let recorded = crate::EffectOutcome::new(
            7_u64,
            Ok(OutcomePayload::HttpResponse(HttpResponse::new(
                200,
                b"replayed".to_vec(),
                false,
                None,
            ))),
        );
        let mut replay = VirtualizedInterpreter::try_new([RecordedOutcome::new(term, recorded)])
            .expect("unique HTTP record");
        replay.submit(7, &effect).expect("HTTP term submits");
        let outcome = replay.next_outcome().expect("recorded outcome");
        let Ok(OutcomePayload::HttpResponse(response)) = outcome.result() else {
            panic!("HTTP replay must return the recorded HTTP response")
        };
        assert_eq!(response.body(), b"replayed");
        replay
            .finish_history()
            .expect("exact replay consumes history");
    }

    #[test]
    fn exact_recorded_agent_and_tool_terms_replay_without_external_handles() {
        let harness = AgentHarnessName::try_from_normalized("recorded-agent")
            .expect("nonempty normalized harness");
        let session = AgentSessionId::new(harness.clone(), b"session-1".to_vec());
        let call_id =
            AgentToolCallId::try_from_bytes(b"call-1".to_vec()).expect("nonempty call id");
        let call = AgentToolCall::new(
            call_id.clone(),
            ToolName::try_from_normalized("write-note").expect("nonempty normalized tool"),
            AgentPayload::new(br#"{"text":"hello"}"#.to_vec()),
        );

        let issuer = GrantIssuer::new();
        let agent_grant = AgentHarnessGrant::agent_harness([harness.clone()]);
        let first_invoke = Effect::agent_invoke(
            issuer.issue(&agent_grant),
            None,
            AgentInvokeSpec::try_new(
                harness.clone(),
                None,
                AgentStepRequest::user_turn(AgentPayload::new(b"remember hello".to_vec())),
            )
            .expect("canonical first agent step"),
        );
        let first_result = AgentStepResult::try_new(
            &harness,
            session.clone(),
            [AgentProgressRecord::new(AgentPayload::new(
                b"choosing tool".to_vec(),
            ))],
            AgentStepNext::ToolRequest { call: call.clone() },
        )
        .expect("result stays in the harness namespace");

        let write_root =
            NormalizedPath::new("/recorded-agent").expect("absolute normalized test path");
        let write_grant = FsWriteGrant::fs_write(PathScopes::new([PathScope::new(write_root)]));
        let tool_effect = Effect::file_write(
            issuer.issue(&write_grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-agent/note").expect("absolute normalized test path"),
                b"hello".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let tool_result = AgentToolResult::succeeded(
            call_id.clone(),
            ConcreteExternalEffectTag::FileWrite,
            ConcreteOutcomePayload::WrittenLength(5),
        )
        .expect("tool result matches the concrete effect");

        let second_invoke = Effect::agent_invoke(
            issuer.issue(&agent_grant),
            None,
            AgentInvokeSpec::try_new(
                harness.clone(),
                Some(session.clone()),
                AgentStepRequest::tool_result(call_id, tool_result)
                    .expect("tool result keeps call identity"),
            )
            .expect("canonical follow-up agent step"),
        );
        let final_result = AgentStepResult::try_new(
            &harness,
            session,
            Box::<[AgentProgressRecord]>::default(),
            AgentStepNext::Final {
                output: AgentPayload::new(b"remembered".to_vec()),
                metadata: AgentPayload::new(b"recorded".to_vec()),
            },
        )
        .expect("terminal result stays in the harness namespace");

        let tape = [
            RecordedOutcome::new(
                EffectTerm::from_recorded(&first_invoke).expect("it is an external term"),
                crate::EffectOutcome::new(
                    10,
                    Ok(OutcomePayload::AgentStepResult(first_result.clone())),
                ),
            ),
            RecordedOutcome::new(
                EffectTerm::from_recorded(&tool_effect).expect("it is an external term"),
                crate::EffectOutcome::new(11, Ok(OutcomePayload::WrittenLength(5))),
            ),
            RecordedOutcome::new(
                EffectTerm::from_recorded(&second_invoke).expect("it is an external term"),
                crate::EffectOutcome::new(
                    12,
                    Ok(OutcomePayload::AgentStepResult(final_result.clone())),
                ),
            ),
        ];

        let mut replay = VirtualizedInterpreter::try_new(tape).expect("unique recorded effects");
        replay
            .submit(10, &first_invoke)
            .expect("recorded first step");
        replay.submit(11, &tool_effect).expect("recorded tool I/O");
        replay
            .submit(12, &second_invoke)
            .expect("recorded follow-up step");

        assert_eq!(
            replay.next_outcome().expect("first outcome").result(),
            &Ok(OutcomePayload::AgentStepResult(first_result))
        );
        assert_eq!(
            replay.next_outcome().expect("tool outcome").result(),
            &Ok(OutcomePayload::WrittenLength(5))
        );
        assert_eq!(
            replay.next_outcome().expect("final outcome").result(),
            &Ok(OutcomePayload::AgentStepResult(final_result))
        );
        assert!(replay.next_outcome().is_none());
        assert!(replay.residual_records().next().is_none());
    }

    #[test]
    fn changed_agent_request_term_diverges_without_live_agent_fallback() {
        let harness = AgentHarnessName::try_from_normalized("recorded-agent")
            .expect("nonempty normalized harness");
        let grant = AgentHarnessGrant::agent_harness([harness.clone()]);
        let issuer = GrantIssuer::new();
        let recorded = Effect::agent_invoke(
            issuer.issue(&grant),
            None,
            AgentInvokeSpec::try_new(
                harness.clone(),
                None,
                AgentStepRequest::user_turn(AgentPayload::new(b"original".to_vec())),
            )
            .expect("canonical recorded request"),
        );
        let changed = Effect::agent_invoke(
            issuer.issue(&grant),
            None,
            AgentInvokeSpec::try_new(
                harness,
                None,
                AgentStepRequest::user_turn(AgentPayload::new(b"changed".to_vec())),
            )
            .expect("canonical changed request"),
        );
        let mut replay = VirtualizedInterpreter::try_new([RecordedOutcome::new(
            EffectTerm::from_recorded(&recorded).expect("it is an external term"),
            crate::EffectOutcome::new(7, Err(EffectFailure::EndpointGone)),
        )])
        .expect("one recorded effect");

        replay
            .submit(7, &changed)
            .expect("divergence is delivered as a terminal outcome");
        assert_eq!(
            replay.next_outcome().expect("divergence outcome").result(),
            &Err(EffectFailure::Diverged(crate::Divergence::EffectMismatch))
        );
        assert_eq!(
            replay
                .residual_records()
                .map(|record| *record.outcome().correlation())
                .collect::<Vec<_>>(),
            [7]
        );
    }

    #[test]
    fn expected_term_mismatch_does_not_mask_a_later_exact_record() {
        let root =
            NormalizedPath::new("/recorded-mismatch").expect("absolute normalized test path");
        let grant = FsWriteGrant::fs_write(PathScopes::new([PathScope::new(root)]));
        let issuer = GrantIssuer::new();
        let first = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-mismatch/first")
                    .expect("absolute normalized test path"),
                b"first".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let changed_first = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-mismatch/changed")
                    .expect("absolute normalized test path"),
                b"changed".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let second = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-mismatch/second")
                    .expect("absolute normalized test path"),
                b"second".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let mut replay = VirtualizedInterpreter::try_new([
            RecordedOutcome::new(
                EffectTerm::from_recorded(&first).expect("it is an external term"),
                crate::EffectOutcome::new(1, Ok(OutcomePayload::WrittenLength(5))),
            ),
            RecordedOutcome::new(
                EffectTerm::from_recorded(&second).expect("it is an external term"),
                crate::EffectOutcome::new(2, Ok(OutcomePayload::WrittenLength(6))),
            ),
        ])
        .expect("unique ordered tape");

        replay
            .submit(1, &changed_first)
            .expect("term divergence is a terminal replay outcome");
        replay
            .submit(2, &second)
            .expect("the later exact record remains independently observable");

        assert_eq!(
            replay.next_outcome().expect("divergence outcome").result(),
            &Err(EffectFailure::Diverged(crate::Divergence::EffectMismatch))
        );
        assert_eq!(
            replay.next_outcome().expect("later exact outcome").result(),
            &Ok(OutcomePayload::WrittenLength(6))
        );
        assert_eq!(
            replay
                .residual_records()
                .map(|record| *record.outcome().correlation())
                .collect::<Vec<_>>(),
            [1]
        );
        let terminal = replay
            .finish_history()
            .expect_err("the mismatched original remains in replay history");
        assert_eq!(terminal.unmatched_records().len(), 1);
        assert_eq!(
            terminal.unmatched_records()[0].term(),
            &EffectTerm::from_recorded(&first).expect("it is an external term")
        );
        assert_eq!(
            terminal.unmatched_records()[0].outcome().result(),
            &Ok(OutcomePayload::WrittenLength(5))
        );
    }

    #[test]
    fn reordered_exact_boundary_terms_match_by_id_and_term() {
        let root = NormalizedPath::new("/recorded-order").expect("absolute normalized test path");
        let grant = FsWriteGrant::fs_write(PathScopes::new([PathScope::new(root)]));
        let issuer = GrantIssuer::new();
        let first = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-order/first")
                    .expect("absolute normalized test path"),
                b"first".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let second = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-order/second")
                    .expect("absolute normalized test path"),
                b"second".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let mut replay = VirtualizedInterpreter::try_new([
            RecordedOutcome::new(
                EffectTerm::from_recorded(&first).expect("it is an external term"),
                crate::EffectOutcome::new(1, Ok(OutcomePayload::WrittenLength(5))),
            ),
            RecordedOutcome::new(
                EffectTerm::from_recorded(&second).expect("it is an external term"),
                crate::EffectOutcome::new(2, Ok(OutcomePayload::WrittenLength(6))),
            ),
        ])
        .expect("unique ordered tape");

        replay
            .submit(2, &second)
            .expect("exact lookup is independent of supplied enumeration");
        replay
            .submit(1, &first)
            .expect("the other exact lookup may follow in either order");
        let second_outcome = replay.next_outcome().expect("second effect outcome first");
        assert_eq!(second_outcome.correlation(), &2);
        assert_eq!(
            second_outcome.result(),
            &Ok(OutcomePayload::WrittenLength(6))
        );
        let first_outcome = replay.next_outcome().expect("first effect outcome second");
        assert_eq!(first_outcome.correlation(), &1);
        assert_eq!(
            first_outcome.result(),
            &Ok(OutcomePayload::WrittenLength(5))
        );
        assert!(replay.residual_records().next().is_none());
        assert!(replay.finish_history().is_ok());
    }

    #[test]
    fn terminal_history_returns_full_strict_prefix_records_in_supplied_enumeration() {
        let root = NormalizedPath::new("/recorded-finalize").expect("absolute normalized path");
        let grant = FsWriteGrant::fs_write(PathScopes::new([PathScope::new(root)]));
        let issuer = GrantIssuer::new();
        let first = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-finalize/first").expect("normalized path"),
                b"first".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let second = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-finalize/second").expect("normalized path"),
                b"second".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let third = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-finalize/third").expect("normalized path"),
                b"third".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let first_record = RecordedOutcome::new(
            EffectTerm::from_recorded(&first).expect("it is an external term"),
            crate::EffectOutcome::new(30, Ok(OutcomePayload::WrittenLength(5))),
        );
        let second_record = RecordedOutcome::new(
            EffectTerm::from_recorded(&second).expect("it is an external term"),
            crate::EffectOutcome::new(10, Ok(OutcomePayload::WrittenLength(6))),
        );
        let third_record = RecordedOutcome::new(
            EffectTerm::from_recorded(&third).expect("it is an external term"),
            crate::EffectOutcome::new(20, Ok(OutcomePayload::WrittenLength(5))),
        );
        let mut replay = VirtualizedInterpreter::try_new([
            first_record,
            second_record.clone(),
            third_record.clone(),
        ])
        .expect("unique recording");

        replay.submit(30, &first).expect("recorded prefix");
        let error = replay
            .finish_history()
            .expect_err("a strict prefix cannot complete recorded history");
        assert_eq!(error.unmatched_records(), &[second_record, third_record]);
    }

    #[test]
    fn terminal_history_ignores_settlement_fifo_and_accepts_exact_history() {
        let root =
            NormalizedPath::new("/recorded-finalize-outcome").expect("absolute normalized path");
        let grant = FsWriteGrant::fs_write(PathScopes::new([PathScope::new(root)]));
        let issuer = GrantIssuer::new();
        let effect = Effect::file_write(
            issuer.issue(&grant),
            None,
            FileWriteSpec::new(
                NormalizedPath::new("/recorded-finalize-outcome/file").expect("normalized path"),
                b"body".to_vec(),
                FileWriteMode::Replace,
            ),
        );
        let record = RecordedOutcome::new(
            EffectTerm::from_recorded(&effect).expect("it is an external term"),
            crate::EffectOutcome::new(7, Ok(OutcomePayload::WrittenLength(4))),
        );
        let mut exact = VirtualizedInterpreter::try_new([record]).expect("unique recording");
        exact.submit(7, &effect).expect("exact term");
        assert!(
            exact.finish_history().is_ok(),
            "history completeness does not claim router/frontier settlement"
        );
    }
}
