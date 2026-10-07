
use circular_plan::ActorId;
use circular_runtime::{
    ApprovalDecision, ApprovalRequestOutcome, EffectOutcome, OpenApprovalState, OutcomePayload,
};
use circular_store::{
    ApprovalLedger, ApprovalLedgerDecision, ApprovalLedgerDiagnostic, ApprovalRequest,
    ApprovalRequestDisposition,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Whether this queue has a durable run-journal owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalPersistence {
    Durable,
    Unavailable(RuntimeApprovalPersistenceUnavailable),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalPersistenceUnavailable {
    PendingOutcomeRouteIsProcessLocal,
}

/// Product `Effect` currently fixes its approval summary type to `()`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalSummary {
    Unavailable(RuntimeApprovalSummaryUnavailable),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalSummaryUnavailable {
    ProductEffectSummaryIsUnit,
}

/// One open row projected from the real store ledger plus producer identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeApprovalRow {
    item: circular_runtime::EffectId,
    emitter: ActorId,
    target_effect: circular_runtime::EffectId,
    state: OpenApprovalState<circular_runtime::EffectId>,
    summary: RuntimeApprovalSummary,
    cause: Option<circular_core::ArrivalIndex>,
}

impl RuntimeApprovalRow {
    #[must_use]
    pub fn item(&self) -> circular_runtime::EffectId {
        self.item.clone()
    }

    #[must_use]
    pub const fn emitter(&self) -> &ActorId {
        &self.emitter
    }

    #[must_use]
    pub fn target_effect(&self) -> circular_runtime::EffectId {
        self.target_effect.clone()
    }

    #[must_use]
    pub const fn state(&self) -> &OpenApprovalState<circular_runtime::EffectId> {
        &self.state
    }

    #[must_use]
    pub const fn summary(&self) -> RuntimeApprovalSummary {
        self.summary
    }

    #[must_use]
    pub const fn cause(&self) -> Option<circular_core::ArrivalIndex> {
        self.cause
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeApprovalContext {
    emitter: ActorId,
    cause: Option<circular_core::ArrivalIndex>,
}

/// Queue-local diagnostics not representable by the underlying ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalDiagnostic {
    TargetAlreadyPending {
        item: circular_runtime::EffectId,
        target_effect: circular_runtime::EffectId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalDecision {
    Applied(ApprovalRequestOutcome<circular_runtime::EffectId>),
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalDecisionError {
    Durable(String),
    QueuePoisoned,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeApprovalInvariantError {
    MissingProducerContext { item: circular_runtime::EffectId },
    OrphanedProducerContext { item: circular_runtime::EffectId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeApprovalQueueSnapshot {
    rows: Vec<RuntimeApprovalRow>,
    diagnostics: Vec<RuntimeApprovalDiagnostic>,
    ledger_diagnostics: Vec<ApprovalLedgerDiagnostic<circular_runtime::EffectId>>,
    persistence: RuntimeApprovalPersistence,
}

impl RuntimeApprovalQueueSnapshot {
    #[must_use]
    pub fn rows(&self) -> &[RuntimeApprovalRow] {
        &self.rows
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[RuntimeApprovalDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn ledger_diagnostics(&self) -> &[ApprovalLedgerDiagnostic<circular_runtime::EffectId>] {
        &self.ledger_diagnostics
    }

    #[must_use]
    pub const fn persistence(&self) -> RuntimeApprovalPersistence {
        self.persistence
    }
}

#[derive(Default)]
struct RuntimeApprovalQueueState {
    submitted: BTreeSet<circular_runtime::EffectId>,
    ledger: ApprovalLedger<circular_runtime::EffectId, ()>,
    contexts: BTreeMap<circular_runtime::EffectId, RuntimeApprovalContext>,
    outcomes: VecDeque<EffectOutcome<circular_runtime::EffectId>>,
    refused: BTreeMap<circular_runtime::EffectId, String>,
    diagnostics: Vec<RuntimeApprovalDiagnostic>,
}

impl RuntimeApprovalQueueState {
    fn open(
        &mut self,
        key: &circular_runtime::EffectId,
        approval: &circular_core::EncodedPayload,
    ) -> Result<(), String> {
        let body = crate::restart_custody_codec::decode_approval_body(
            &crate::restart_custody_codec::ProductRestoreNestedCodec,
            approval,
        )?;
        let circular_runtime::EffectTerm::RequestApproval(spec) = body.request_term else {
            return Err("approval body has another term".into());
        };
        if !self.submitted.insert(key.clone()) {
            return Ok(());
        }
        if self.ledger.request(ApprovalRequest::new(
            key.clone(),
            (),
            spec.target_effect().clone(),
        )) != ApprovalRequestDisposition::Inserted
        {
            return Err("approval request conflicts in journal".into());
        }
        self.contexts.insert(
            key.clone(),
            RuntimeApprovalContext {
                emitter: body.actor,
                cause: Some(body.cause.0),
            },
        );
        Ok(())
    }

    fn settle(
        &mut self,
        key: &circular_runtime::EffectId,
        observation: &circular_core::EncodedPayload,
    ) -> Result<(), String> {
        match crate::restart_custody_codec::decode_approval_terminal(observation)? {
            circular_store::ApprovalTerminal::Denied => {
                if let ApprovalLedgerDecision::Applied(outcome) =
                    self.ledger.decide(key, ApprovalDecision::Deny)
                {
                    self.outcomes.push_back(EffectOutcome::new(
                        key.clone(),
                        Ok(OutcomePayload::Approval(outcome)),
                    ));
                }
            }
            circular_store::ApprovalTerminal::Consumed => {
                if let Some(OpenApprovalState::Approved(ticket)) =
                    self.ledger.row(key).map(|row| row.state().clone())
                {
                    let _ = self.ledger.authorize(ticket.target_effect(), Some(&ticket));
                }
            }
        }
        if self.ledger.row(key).is_some() {
            return Err("recorded approval settlement does not close the request".into());
        }
        Ok(())
    }
}

type PublishedApprovals = Result<RuntimeApprovalQueueSnapshot, RuntimeApprovalInvariantError>;

#[derive(Clone, Default)]
pub struct RuntimeApprovalQueue {
    source: Option<ApprovalSource>,
    returns: Option<ApprovalReturns>,
    inbox: std::sync::Arc<
        std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<ApprovalCommand>>>,
    >,
}

#[derive(Clone)]
pub(crate) struct ApprovalReturns(tokio::sync::mpsc::UnboundedSender<ApprovalCommand>);

type DecisionResult = Result<RuntimeApprovalDecision, RuntimeApprovalDecisionError>;

enum ApprovalCommand {
    Register(circular_runtime::EffectId, crate::kernel::Address),
    Decide(
        circular_runtime::EffectId,
        ApprovalDecision,
        std::sync::mpsc::SyncSender<DecisionResult>,
    ),
}

impl ApprovalReturns {
    pub(crate) fn register(
        &self,
        item: circular_runtime::EffectId,
        address: crate::kernel::Address,
    ) -> Result<(), String> {
        self.0
            .send(ApprovalCommand::Register(item, address))
            .map_err(|_| "the approval ledger stopped receiving return addresses".to_owned())
    }
}

#[derive(Clone)]
enum ApprovalSource {
    Live(crate::ProductDurableArrivalJournal),
    Recorded(std::path::PathBuf),
}

impl RuntimeApprovalQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(journal: crate::ProductDurableArrivalJournal) -> Result<Self, String> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        Ok(Self {
            source: Some(ApprovalSource::Live(journal)),
            returns: Some(ApprovalReturns(sender)),
            inbox: std::sync::Arc::new(std::sync::Mutex::new(Some(receiver))),
        })
    }

    #[must_use]
    pub fn recorded(path: &std::path::Path) -> Self {
        Self {
            source: Some(ApprovalSource::Recorded(path.to_path_buf())),
            returns: None,
            inbox: Default::default(),
        }
    }

    pub(crate) fn start(&self, runtime: &tokio::runtime::Handle) -> Option<ApprovalReturns> {
        let inbox = self.inbox.lock().ok()?.take()?;
        let ledger = Self {
            source: self.source.clone(),
            returns: None,
            inbox: Default::default(),
        };
        runtime.spawn(ledger.receive(inbox));
        self.returns.clone()
    }

    async fn receive(self, mut receiver: tokio::sync::mpsc::UnboundedReceiver<ApprovalCommand>) {
        let mut addresses = BTreeMap::new();
        while let Some(command) = receiver.recv().await {
            match command {
                ApprovalCommand::Register(item, address) => {
                    match self.fold() {
                        Ok((state, _)) => {
                            if let Some(reason) = state.refused.get(&item) {
                                let _ = address.send(crate::kernel::Message::ApprovalUnavailable(
                                    format!("recorded approval request is refused: {reason}"),
                                ));
                            } else if let Some(outcome) = state
                                .outcomes
                                .into_iter()
                                .find(|outcome| outcome.correlation() == &item)
                            {
                                let _ = address
                                    .send(crate::kernel::Message::Settled(Box::new(outcome)));
                            } else {
                                addresses.insert(item, address);
                            }
                        }
                        Err(reason) => {
                            let _ =
                                address.send(crate::kernel::Message::ApprovalUnavailable(reason));
                        }
                    }
                }
                ApprovalCommand::Decide(item, decision, reply) => {
                    let result = self.decide_recorded(item.clone(), decision).await;
                    if let Ok(RuntimeApprovalDecision::Applied(outcome)) = &result {
                        if let Some(address) = addresses.remove(&item) {
                            let _ = address.send(crate::kernel::Message::Settled(Box::new(
                                EffectOutcome::new(
                                    item,
                                    Ok(OutcomePayload::Approval(outcome.clone())),
                                ),
                            )));
                        }
                    }
                    let _ = reply.send(result);
                }
            }
        }
    }

    fn fold(&self) -> Result<(RuntimeApprovalQueueState, RuntimeApprovalPersistence), String> {
        match &self.source {
            Some(ApprovalSource::Live(journal)) => {
                let (source, prefix) = journal.approval_prefix()?;
                Ok((
                    Self::fold_prefix(&prefix, &source)?,
                    RuntimeApprovalPersistence::Durable,
                ))
            }
            Some(ApprovalSource::Recorded(path)) => {
                let prefix = circular_store::SqliteJournal::read_only_namespace(
                    path,
                    crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
                )
                .map_err(|e| e.to_string())?;
                Ok((
                    Self::fold_prefix(
                        &prefix,
                        &circular_store::SqliteJournal::open_read_only_namespace(
                            path,
                            crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
                        )
                        .map_err(|e| e.to_string())?,
                    )?,
                    RuntimeApprovalPersistence::Durable,
                ))
            }
            None => Ok((
                RuntimeApprovalQueueState::default(),
                RuntimeApprovalPersistence::Unavailable(
                    RuntimeApprovalPersistenceUnavailable::PendingOutcomeRouteIsProcessLocal,
                ),
            )),
        }
    }

    fn fold_prefix(
        snapshot: &circular_store::SqliteJournalSnapshot,
        source: &circular_store::SqliteJournal,
    ) -> Result<RuntimeApprovalQueueState, String> {
        use circular_store::JournalProjection;
        let mut state = RuntimeApprovalQueueState::default();
        let mut all_records = Vec::new();
        let mut decoder = circular_store::ProductJournalReader::default();
        for entry in snapshot.entries() {
            let transaction = decoder
                .read_at(source, entry.sequence().get(), entry.payload())
                .map_err(|e| format!("approval journal: {e:?}"))?;
            let records = circular_store::ArrivalProjection::new()
                .project(&transaction)
                .map_err(|e| format!("approval projection: {e:?}"))?;
            all_records.extend(records);
            for op in transaction.operations() {
                match op {
                    circular_store::StoreTransactionOp::OpenApproval { key, approval } => {
                        if let Err(reason) = state.open(key, approval) {
                            state.refused.insert(key.clone(), reason);
                        }
                    }
                    circular_store::StoreTransactionOp::ApproveApproval { key, .. } => {
                        if state.refused.contains_key(key) {
                            continue;
                        }
                        if let ApprovalLedgerDecision::Applied(outcome) =
                            state.ledger.decide(key, ApprovalDecision::Approve)
                        {
                            state.outcomes.push_back(EffectOutcome::new(
                                key.clone(),
                                Ok(OutcomePayload::Approval(outcome)),
                            ));
                        }
                    }
                    circular_store::StoreTransactionOp::SettleApproval { key, observation } => {
                        if !state.refused.contains_key(key)
                            && let Err(reason) = state.settle(key, observation.value())
                        {
                            state.refused.insert(key.clone(), reason);
                        }
                        state.contexts.remove(key);
                    }
                    _ => {}
                }
            }
        }
        let recorded = all_records
            .iter()
            .filter_map(|record| match record.header().key() {
                circular_store::ClassKey::Boundary(circular_store::BoundaryKey::Arrival {
                    origin,
                    ..
                }) => match origin.as_ref() {
                    circular_store::ArrivalKey::EffectOutcome { effect } => Some(effect.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        state
            .outcomes
            .retain(|outcome| !recorded.contains(outcome.correlation()));
        Ok(state)
    }

    pub fn decide(
        &self,
        item: circular_runtime::EffectId,
        decision: ApprovalDecision,
    ) -> DecisionResult {
        let Some(returns) = &self.returns else {
            return Err(RuntimeApprovalDecisionError::Durable(
                "a stopped stream takes no approval decision".into(),
            ));
        };
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        returns
            .0
            .send(ApprovalCommand::Decide(item, decision, reply))
            .map_err(|_| {
                RuntimeApprovalDecisionError::Durable("the approval ledger stopped".into())
            })?;
        answer.recv().map_err(|_| {
            RuntimeApprovalDecisionError::Durable(
                "the approval ledger stopped before deciding".into(),
            )
        })?
    }

    /// Apply a human decision through the canonical store ledger.
    ///
    /// Only an applied first decision is recorded. The ledger then sends the outcome to the
    /// request's return address; the actor records its own arrival. Rejected, unknown, and
    /// duplicate decisions fail closed.
    async fn decide_recorded(
        &self,
        item: circular_runtime::EffectId,
        decision: ApprovalDecision,
    ) -> DecisionResult {
        let Some(ApprovalSource::Live(journal)) = &self.source else {
            return Err(RuntimeApprovalDecisionError::Durable(
                "a stopped stream takes no approval decision".to_owned(),
            ));
        };
        let (mut state, _) = self.fold().map_err(RuntimeApprovalDecisionError::Durable)?;
        if let Some(reason) = state.refused.get(&item) {
            return Err(RuntimeApprovalDecisionError::Durable(format!(
                "recorded approval request is refused: {reason}"
            )));
        }
        let result = state.ledger.decide(&item, decision);
        let mutation = match &result {
            ApprovalLedgerDecision::Applied(ApprovalRequestOutcome::Approved(_)) => Some(
                crate::product_arrival_journal::ApprovalMutation::Approve(item.clone()),
            ),
            ApprovalLedgerDecision::Applied(ApprovalRequestOutcome::Denied) => {
                Some(crate::product_arrival_journal::ApprovalMutation::Settle(
                    item.clone(),
                    circular_store::ApprovalTerminal::Denied,
                ))
            }
            _ => None,
        };
        if let Some(mutation) = mutation {
            journal
                .submit_approval(mutation)
                .await
                .map_err(|_| {
                    RuntimeApprovalDecisionError::Durable("the approval writer stopped".into())
                })?
                .map_err(RuntimeApprovalDecisionError::Durable)?;
        }
        Ok(match result {
            ApprovalLedgerDecision::Applied(outcome) => RuntimeApprovalDecision::Applied(outcome),
            ApprovalLedgerDecision::Rejected => RuntimeApprovalDecision::Rejected,
            ApprovalLedgerDecision::Unknown => RuntimeApprovalDecision::Unknown,
        })
    }

    pub fn snapshot(&self) -> Result<RuntimeApprovalQueueSnapshot, String> {
        let (state, persistence) = self.fold()?;
        project_snapshot(&state, persistence).map_err(|e| format!("approval snapshot: {e:?}"))
    }
}

fn project_snapshot(
    state: &RuntimeApprovalQueueState,
    persistence: RuntimeApprovalPersistence,
) -> PublishedApprovals {
    let standing = |item: &&circular_runtime::EffectId| !state.refused.contains_key(*item);
    for item in state.contexts.keys().filter(standing) {
        if state.ledger.row(item).is_none() {
            return Err(RuntimeApprovalInvariantError::OrphanedProducerContext {
                item: item.clone(),
            });
        }
    }
    let rows = state
        .ledger
        .rows()
        .keys()
        .filter(standing)
        .cloned()
        .map(|item| project_row(state, item))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RuntimeApprovalQueueSnapshot {
        rows,
        diagnostics: state.diagnostics.clone(),
        ledger_diagnostics: state.ledger.diagnostics().to_vec(),
        persistence,
    })
}

fn project_row(
    state: &RuntimeApprovalQueueState,
    item: circular_runtime::EffectId,
) -> Result<RuntimeApprovalRow, RuntimeApprovalInvariantError> {
    let row = state
        .ledger
        .row(&item)
        .ok_or(RuntimeApprovalInvariantError::MissingProducerContext { item: item.clone() })?;
    let context = state
        .contexts
        .get(&item)
        .ok_or(RuntimeApprovalInvariantError::MissingProducerContext { item: item.clone() })?;
    Ok(RuntimeApprovalRow {
        item,
        emitter: context.emitter.clone(),
        target_effect: row.request().target().clone(),
        state: row.state().clone(),
        summary: RuntimeApprovalSummary::Unavailable(
            RuntimeApprovalSummaryUnavailable::ProductEffectSummaryIsUnit,
        ),
        cause: context.cause,
    })
}
