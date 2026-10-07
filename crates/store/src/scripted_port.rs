
use crate::{
    AtomicStoreTransactionPort, CheckpointOwners, CrashDurableTransactionPort,
    RecoveredTransactionModel, RecoveringTransactionModel, RecoveryTerminal, StoreTransaction,
    StoreTransactionFailure, StoreTransactionReceipt, TransactionSchema,
};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptedCommitStep<S: TransactionSchema> {
    Exact,
    ReturnReceiptFor(StoreTransaction<S>),
    CommitThenReturnReceiptFor(StoreTransaction<S>),
    DuplicateFirstOperation,
    BackendFailure(Box<str>),
}

impl<S: TransactionSchema> Default for ScriptedCommitStep<S> {
    fn default() -> Self {
        Self::Exact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptedPortError {
    reason: Box<str>,
}

impl ScriptedPortError {
    #[must_use]
    pub fn new(reason: impl Into<Box<str>>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    #[must_use]
    pub const fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for ScriptedPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for ScriptedPortError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptedDurablePort<S: TransactionSchema> {
    model: RecoveredTransactionModel<S>,
    steps: VecDeque<ScriptedCommitStep<S>>,
    commits: Vec<StoreTransaction<S>>,
}

impl<S: TransactionSchema> ScriptedDurablePort<S> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_script([])
    }

    #[must_use]
    pub fn with_script(steps: impl IntoIterator<Item = ScriptedCommitStep<S>>) -> Self {
        let model = RecoveringTransactionModel::empty()
            .recover(
                &CheckpointOwners::default(),
                |_, _| -> Result<RecoveryTerminal<S>, Infallible> {
                    unreachable!("an empty model does not terminalize")
                },
            )
            .expect("recovery of an empty model succeeds")
            .into_parts()
            .0;
        Self {
            model,
            steps: steps.into_iter().collect(),
            commits: Vec::new(),
        }
    }

    pub fn push_step(&mut self, step: ScriptedCommitStep<S>) {
        self.steps.push_back(step);
    }

    #[must_use]
    pub fn pending_steps(&self) -> usize {
        self.steps.len()
    }

    #[must_use]
    pub const fn commit_count(&self) -> usize {
        self.commits.len()
    }

    #[must_use]
    pub fn commits(&self) -> &[StoreTransaction<S>] {
        &self.commits
    }

    #[must_use]
    pub const fn model(&self) -> &RecoveredTransactionModel<S> {
        &self.model
    }

    #[must_use]
    pub fn into_model(self) -> RecoveredTransactionModel<S> {
        self.model
    }

    fn exact(
        model: &mut RecoveredTransactionModel<S>,
        transaction: StoreTransaction<S>,
    ) -> Result<StoreTransactionReceipt<S>, StoreTransactionFailure<S, ScriptedPortError>> {
        model
            .commit(transaction)
            .map_err(|failure| failure.map_backend(|never| match never {}))
    }
}

impl<S: TransactionSchema> Default for ScriptedDurablePort<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: TransactionSchema> AtomicStoreTransactionPort<S> for ScriptedDurablePort<S> {
    type BackendError = ScriptedPortError;

    fn commit(
        &mut self,
        transaction: StoreTransaction<S>,
    ) -> Result<StoreTransactionReceipt<S>, StoreTransactionFailure<S, Self::BackendError>> {
        self.commits.push(transaction.clone());
        match self.steps.pop_front().unwrap_or_default() {
            ScriptedCommitStep::Exact => Self::exact(&mut self.model, transaction),
            ScriptedCommitStep::ReturnReceiptFor(scripted) => {
                Self::exact(&mut self.model, scripted)
            }
            ScriptedCommitStep::CommitThenReturnReceiptFor(scripted) => {
                let mut candidate = self.model.clone();
                Self::exact(&mut candidate, transaction.clone())?;
                match Self::exact(&mut candidate, scripted) {
                    Ok(receipt) => {
                        self.model = candidate;
                        Ok(receipt)
                    }
                    Err(failure) => Err(StoreTransactionFailure::backend(
                        transaction,
                        ScriptedPortError::new(format!(
                            "foreign receipt script rejected: {:?}",
                            failure.reason()
                        )),
                    )),
                }
            }
            ScriptedCommitStep::DuplicateFirstOperation => {
                let mut operations = transaction.operations().to_vec();
                operations.push(operations[0].clone());
                let duplicated = StoreTransaction::try_new(operations)
                    .expect("the input transaction is not empty");
                Self::exact(&mut self.model, duplicated)
            }
            ScriptedCommitStep::BackendFailure(reason) => Err(StoreTransactionFailure::backend(
                transaction,
                ScriptedPortError::new(reason),
            )),
        }
    }
}

impl<S: TransactionSchema> CrashDurableTransactionPort<S> for ScriptedDurablePort<S> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        StoreTransactionFailureReason, StoreTransactionOp, StoreTransactionRef, TransactionAppend,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestSchema;

    impl TransactionSchema for TestSchema {
        type WriteContext = ();
        fn record_digest(record: &Self::Record) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(record.as_bytes())
        }

        fn observation_digest(observation: &Self::Observation) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(observation.as_bytes())
        }

        type RecordKey = u8;
        type Record = &'static str;
        type EffectId = u8;
        type Outbox = &'static str;
        type ApprovalKey = u8;
        type Approval = &'static str;
        type ApprovalTicket = &'static str;
        type ActorId = u8;
        type Incarnation = u8;
        type CheckpointStamp = u8;
        type CheckpointState = &'static str;
        type Outcome = &'static str;
        type ObservationKey = u8;
        type Observation = &'static str;
    }

    fn append(key: u8, record: &'static str) -> StoreTransaction<TestSchema> {
        StoreTransaction::try_new(vec![StoreTransactionOp::Append(TransactionAppend::new(
            key, record,
        ))])
        .expect("single append")
    }

    #[test]
    fn exact_is_default_and_records_calls() {
        let mut port = ScriptedDurablePort::new();
        let transaction = append(1, "one");
        let receipt = port.commit(transaction.clone()).expect("exact commit");

        assert_eq!(port.commit_count(), 1);
        assert_eq!(port.commits(), &[transaction]);
        assert_eq!(
            port.model().record_digest(&1),
            Some(TestSchema::record_digest(&"one"))
        );
        assert_eq!(receipt.refs(), &[StoreTransactionRef::Appended(1)]);
    }

    #[test]
    fn receipt_substitution_supports_before_and_after_commit_shapes() {
        let mut substitute =
            ScriptedDurablePort::with_script([ScriptedCommitStep::ReturnReceiptFor(append(
                2, "foreign",
            ))]);
        let receipt = substitute.commit(append(1, "input")).expect("substitute");
        assert_eq!(receipt.refs(), &[StoreTransactionRef::Appended(2)]);
        assert_eq!(substitute.model().record_digest(&1), None);

        let mut after =
            ScriptedDurablePort::with_script([ScriptedCommitStep::CommitThenReturnReceiptFor(
                append(2, "foreign"),
            )]);
        after.commit(append(1, "input")).expect("foreign receipt");
        assert_eq!(
            after.model().record_digest(&1),
            Some(TestSchema::record_digest(&"input"))
        );
        assert_eq!(
            after.model().record_digest(&2),
            Some(TestSchema::record_digest(&"foreign"))
        );
    }

    #[test]
    fn duplicate_and_backend_failure_are_fifo_programmable() {
        let mut port = ScriptedDurablePort::with_script([
            ScriptedCommitStep::DuplicateFirstOperation,
            ScriptedCommitStep::BackendFailure("offline".into()),
        ]);
        let receipt = port.commit(append(1, "one")).expect("duplicate receipt");
        assert_eq!(receipt.refs().len(), 2);

        let failure = port.commit(append(2, "two")).expect_err("backend failure");
        assert!(matches!(
            failure.reason(),
            StoreTransactionFailureReason::Backend(error) if error.reason() == "offline"
        ));
        assert_eq!(port.model().record_digest(&2), None);
        assert_eq!(port.pending_steps(), 0);
    }
}
