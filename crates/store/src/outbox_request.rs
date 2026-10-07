use circular_core::{
    Boundary, Ceilings, EncodedPayload, PayloadVersionTag, RecordedInstant, Value,
};
use circular_runtime::{EffectId, EffectOccasion, EffectTerm};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductOutboxRequest {
    pub term: EffectTerm,
    pub cause: circular_core::Stamp<circular_runtime::ActorId>,
    pub submitted_at: RecordedInstant,
}

impl ProductOutboxRequest {
    pub fn encode(&self) -> Result<EncodedPayload, String> {
        let value = Value::array([
            Value::bytes(
                circular_runtime::try_encode_term(&self.term).map_err(|e| format!("{e:?}"))?,
            ),
            Value::bytes(
                circular_runtime::encode_effect_stamp(&self.cause).map_err(|e| e.to_string())?,
            ),
            Value::UInt(self.submitted_at.millis()),
        ]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|e| e.to_string())?;
        Ok(EncodedPayload::new(PayloadVersionTag::FIRST, &bytes))
    }

    pub fn decode(payload: &EncodedPayload) -> Result<Self, String> {
        if payload.version_tag() != PayloadVersionTag::FIRST {
            return Err("unsupported outbox request version".into());
        }
        let value =
            circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
                .map_err(|e| e.to_string())?;
        let Some([Value::Bytes(term), Value::Bytes(cause), Value::UInt(millis)]) = value.as_array()
        else {
            return Err("outbox request requires [term Bytes, cause Bytes, submitted UInt]".into());
        };
        Ok(Self {
            term: circular_runtime::decode_term(term).map_err(|e| format!("{e:?}"))?,
            cause: circular_runtime::decode_effect_stamp(cause).map_err(|e| e.to_string())?,
            submitted_at: RecordedInstant::from_millis(*millis),
        })
    }

    /// Validate the independent recorded cause against the canonical issuance coordinate.
    pub fn validate_key(&self, key: &EffectId) -> Result<(), String> {
        if let EffectOccasion::Delivery(_, cause) = key.occasion()
            && cause != &self.cause
        {
            return Err("outbox cause differs from its Delivery EffectId".into());
        }
        Ok(())
    }
}

/// Read-only custody image before the request owner chooses recovery actions.
/// Replaying transactions grants no writable handle and performs no dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductOutboxCustody {
    request: ProductOutboxRequest,
    phase: crate::TransactionOutboxPhase,
}

impl ProductOutboxCustody {
    pub fn request(&self) -> &ProductOutboxRequest {
        &self.request
    }

    pub const fn phase(&self) -> crate::TransactionOutboxPhase {
        self.phase
    }
}

pub struct ProductCustodySnapshot {
    pub approval_requests: std::collections::BTreeMap<EffectId, EncodedPayload>,
    outboxes: Vec<(EffectId, ProductOutboxCustody)>,
    pub checkpoints: Vec<crate::TransactionCheckpoint<crate::ProductTransaction>>,
    settled_outboxes: Vec<(
        EffectId,
        ProductOutboxRequest,
        crate::TransactionOutboxPhase,
        EncodedPayload,
    )>,
}
impl ProductCustodySnapshot {
    /// Immutable validated key/request pairs. Callers may clone observations,
    /// but cannot replace a key or request inside this recovery authority.
    pub fn outboxes(&self) -> &[(EffectId, ProductOutboxCustody)] {
        &self.outboxes
    }

    pub fn settled_outboxes(
        &self,
    ) -> &[(
        EffectId,
        ProductOutboxRequest,
        crate::TransactionOutboxPhase,
        EncodedPayload,
    )] {
        &self.settled_outboxes
    }

    pub fn read(
        snapshot: &crate::SqliteJournalSnapshot,
    ) -> Result<Self, crate::SqliteJournalError> {
        Self::read_from(snapshot, false)
    }

    pub fn read_from(
        snapshot: &crate::SqliteJournalSnapshot,
        after_horizon: bool,
    ) -> Result<Self, crate::SqliteJournalError> {
        Self::read_from_source(snapshot, after_horizon, None)
    }

    pub(crate) fn read_from_source(
        snapshot: &crate::SqliteJournalSnapshot,
        after_horizon: bool,
        source: Option<&crate::SqliteJournal>,
    ) -> Result<Self, crate::SqliteJournalError> {
        let invalid = |detail: String| crate::SqliteJournalError::Integrity { detail };
        let mut model = if after_horizon {
            crate::RecoveredTransactionModel::<crate::ProductTransaction>::empty_after_horizon()
        } else {
            crate::RecoveredTransactionModel::<crate::ProductTransaction>::empty_for_journal_replay(
            )
        };
        let mut fold = CustodyFold::default();
        let mut reader = crate::ProductJournalReader::default();
        for entry in snapshot.entries() {
            let transaction = match source {
                Some(source) => reader.read_at(source, entry.sequence().get(), entry.payload())?,
                None => reader.read_entry(entry)?,
            };
            fold.observe(entry, &transaction)?;
            model = model
                .fold_committed(transaction)
                .map_err(|e| invalid(format!("custody fold: {e:?}")))?
                .0;
        }
        fold.finish(&model)
    }
}

#[derive(Default)]
pub struct CustodyFold {
    requests:
        std::collections::BTreeMap<EffectId, (ProductOutboxRequest, crate::TransactionOutboxPhase)>,
    delivered: std::collections::BTreeMap<EffectId, (EncodedPayload, EncodedPayload)>,
    approval_requests: std::collections::BTreeMap<EffectId, EncodedPayload>,
}

impl CustodyFold {
    pub fn observe(
        &mut self,
        entry: &crate::SqliteJournalEntry,
        transaction: &crate::StoreTransaction<crate::ProductTransaction>,
    ) -> Result<(), crate::SqliteJournalError> {
        use crate::JournalProjection;
        let invalid = |detail: String| crate::SqliteJournalError::Integrity { detail };
        for record in crate::ArrivalProjection::new()
            .project(transaction)
            .map_err(|e| crate::ProductJournalCodec.projection_error(entry, transaction, e))?
        {
            self.observe_record(&record);
        }
        self.observe_operations(transaction).map_err(invalid)
    }

    pub fn observe_record(&mut self, record: &crate::Record<crate::ProductStore>) {
        if let crate::Record::Boundary(boundary) = record {
            let crate::BoundaryFact::Arrival { origin, .. } = boundary.fact() else {
                return;
            };
            if let crate::ArrivalOrigin::EffectOutcome {
                effect,
                term,
                outcome,
            } = origin.as_ref()
            {
                self.delivered
                    .insert(effect.clone(), (term.clone(), outcome.clone()));
            }
        }
    }

    pub fn observe_operations(
        &mut self,
        transaction: &crate::StoreTransaction<crate::ProductTransaction>,
    ) -> Result<(), String> {
        for operation in transaction.operations() {
            match operation {
                crate::StoreTransactionOp::OpenApproval { key, approval } => {
                    self.approval_requests.insert(key.clone(), approval.clone());
                }
                crate::StoreTransactionOp::OpenOutbox { effect, request } => {
                    let request = ProductOutboxRequest::decode(request)?;
                    request.validate_key(effect)?;
                    self.requests
                        .entry(effect.clone())
                        .or_insert((request, crate::TransactionOutboxPhase::Committed));
                }
                crate::StoreTransactionOp::SubmitOutbox { effect }
                | crate::StoreTransactionOp::AcquireOutboxDispatch { effect } => {
                    self.requests
                        .get_mut(effect)
                        .ok_or_else(|| "outbox submission lacks request".to_owned())?
                        .1 = crate::TransactionOutboxPhase::Submitted;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn finish(
        &self,
        model: &crate::RecoveredTransactionModel<crate::ProductTransaction>,
    ) -> Result<ProductCustodySnapshot, crate::SqliteJournalError> {
        let invalid = |detail: String| crate::SqliteJournalError::Integrity { detail };
        let requests = &self.requests;
        let delivered = &self.delivered;
        for (key, (term, boundary_outcome)) in delivered {
            if let Some(outcome) = model.outcome(key) {
                let (request, _) = requests
                    .get(key)
                    .ok_or_else(|| invalid("settled effect lacks its original request".into()))?;
                let expected_term = circular_runtime::try_encode_term(&request.term)
                    .map_err(|e| invalid(format!("settled term: {e:?}")))?;
                if outcome != boundary_outcome || term.body() != expected_term {
                    return Err(invalid(
                        "durable outbox settlement conflicts with its terminal Boundary".into(),
                    ));
                }
            }
        }
        let outboxes = model
            .open_outboxes()
            .map(|(key, row)| {
                let (request, _) = requests
                    .get(key)
                    .ok_or_else(|| invalid("open outbox lacks its decoded request".into()))?;
                Ok((
                    key.clone(),
                    ProductOutboxCustody {
                        request: request.clone(),
                        phase: row.phase(),
                    },
                ))
            })
            .collect::<Result<_, crate::SqliteJournalError>>()?;
        Ok(ProductCustodySnapshot {
            approval_requests: self.approval_requests.clone(),
            settled_outboxes: requests
                .iter()
                .filter(|(key, _)| !delivered.contains_key(*key))
                .filter_map(|(key, (request, phase))| {
                    model
                        .outcome(key)
                        .cloned()
                        .map(|outcome| (key.clone(), request.clone(), *phase, outcome))
                })
                .collect(),
            outboxes,
            checkpoints: model
                .current_checkpoints()
                .map(|(_, row)| row.clone())
                .collect(),
        })
    }
}

