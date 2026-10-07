use crate::{
    ArrivalProjection, CustodyFold, MemoryStore, PagePolicy, ProductCustodySnapshot,
    ProductJournalCodec, ProductStore, ProductTransaction, Record, RecoveredTransactionModel,
    SqliteJournalSnapshot, StoreTransaction, StoreTransactionOp, TransactionCheckpoint,
};

pub struct JournalFoldHooks<'a> {
    pub facts: &'a mut dyn FnMut(
        u64,
        &StoreTransaction<ProductTransaction>,
    ) -> Result<Vec<Record<ProductStore>>, String>,
    pub record: &'a mut dyn FnMut(u64, &Record<ProductStore>) -> Result<(), String>,
}

pub struct ProductJournalFold {
    through: u64,
    reader: crate::ProductJournalReader,
    horizon: u64,
    model: Option<RecoveredTransactionModel<ProductTransaction>>,
    custody: CustodyFold,
    published: MemoryStore<ProductStore>,
    prefix: Option<(String, std::path::PathBuf)>,
    checkpoints: Vec<TransactionCheckpoint<ProductTransaction>>,
    next_record_key: u64,
    commits: u64,
}

impl ProductJournalFold {
    #[must_use]
    pub fn new(_namespace: String, page_policy: PagePolicy) -> Self {
        Self {
            through: 0,
            reader: crate::ProductJournalReader::default(),
            horizon: 0,
            model: Some(RecoveredTransactionModel::empty_for_journal_replay()),
            custody: CustodyFold::default(),
            published: MemoryStore::new(page_policy),
            prefix: None,
            checkpoints: Vec::new(),
            next_record_key: 0,
            commits: 0,
        }
    }

    #[must_use]
    pub fn after_horizon(
        namespace: String,
        page_policy: PagePolicy,
        path: std::path::PathBuf,
        horizon: u64,
    ) -> Self {
        let mut fold = Self::new(namespace.clone(), page_policy);
        if horizon > 0 {
            fold.through = horizon;
            fold.horizon = horizon;
            fold.model = Some(RecoveredTransactionModel::empty_after_horizon());
            fold.prefix = Some((namespace, path));
        }
        fold
    }

    #[must_use]
    pub const fn horizon(&self) -> u64 {
        self.horizon
    }

    pub fn note_prior_manifest(&mut self) {
        self.published.note_prior_manifest();
    }

    pub fn extend(
        &mut self,
        snapshot: &SqliteJournalSnapshot,
        hooks: &mut JournalFoldHooks<'_>,
    ) -> Result<(), String> {
        use crate::port::Store as _;
        let source = self
            .prefix
            .as_ref()
            .map(|(namespace, path)| {
                crate::SqliteJournal::open_read_only_namespace(path, namespace)
            })
            .transpose()
            .map_err(|e| e.to_string())?;
        for entry in snapshot.entries() {
            let sequence = entry.sequence().get();
            if sequence <= self.through {
                return Err(format!(
                    "journal fold tail overlaps its prefix: commit {sequence} at or before {}",
                    self.through
                ));
            }
            let transaction = match &source {
                Some(journal) => self.reader.read_at(journal, sequence, entry.payload()),
                None => self.reader.read_entry(entry),
            }
            .map_err(|error| format!("arrival journal transaction {sequence}: {error}"))?;
            self.custody
                .observe_operations(&transaction)
                .map_err(|error| format!("custody commit {sequence}: {error}"))?;
            for operation in transaction.operations() {
                let key = match operation {
                    StoreTransactionOp::Append(append) => Some(append.key()),
                    StoreTransactionOp::ReplaceCheckpoint(row) => {
                        self.checkpoints.push(row.clone());
                        Some(row.at())
                    }
                    StoreTransactionOp::AppendObservation(observation)
                    | StoreTransactionOp::CancelCommittedOutbox { observation, .. }
                    | StoreTransactionOp::SettleCheckpoint { observation, .. }
                    | StoreTransactionOp::SettleOutbox { observation, .. }
                    | StoreTransactionOp::SettleApproval { observation, .. } => {
                        Some(observation.key())
                    }
                    _ => None,
                };
                if let Some(key) = key {
                    self.next_record_key = self.next_record_key.max(
                        key.get()
                            .checked_add(1)
                            .ok_or("arrival journal record-key domain exhausted")?,
                    );
                }
            }
            let positioned = ArrivalProjection::new()
                .project_positions(&transaction)
                .map_err(|rejection| {
                    ProductJournalCodec
                        .projection_error(entry, &transaction, rejection)
                        .to_string()
                })?;
            for (_, row) in &positioned {
                self.custody.observe_record(row.record());
            }
            let mut rows: Vec<crate::EncodedRow<ProductStore>> =
                Vec::with_capacity(positioned.len());
            for (_, mut row) in positioned {
                crate::product_journal::resolve_emitted_body(&mut row, |producer, emission| {
                    let payload = rows
                        .iter()
                        .find_map(|row| {
                            crate::product_journal::emission_body_of(
                                row.record(),
                                producer,
                                emission,
                            )
                        })
                        .or_else(|| {
                            crate::product_journal::emission_body_in(
                                &self.published,
                                producer,
                                emission,
                            )
                        });
                    if payload.is_some() {
                        return Ok(payload);
                    }
                    match &self.prefix {
                        Some((namespace, path)) => crate::read_emission_body(
                            path,
                            namespace,
                            (self.horizon, u32::MAX),
                            producer,
                            emission,
                        ),
                        None => Ok(None),
                    }
                })?;
                (hooks.record)(sequence, row.record())?;
                rows.push(row);
            }
            for fact in (hooks.facts)(sequence, &transaction)? {
                self.published.push_checkpoint_fact(fact, sequence);
            }
            self.published.note_commit(sequence);
            if !rows.is_empty() {
                let batch = crate::memory::AppendBatch::try_new_rows(rows)
                    .map_err(|_| format!("arrival journal batch {sequence} is empty"))?
                    .at_commit(sequence);
                if let crate::memory::AppendResult::Failed(failure) = self.published.append(batch) {
                    return Err(format!("arrival journal append {sequence}: {failure:?}"));
                }
            }
            if let Some(model) = self.model.take() {
                self.model = Some(
                    model
                        .fold_committed(transaction)
                        .map_err(|error| format!("arrival journal commit {sequence}: {error:?}"))?
                        .0,
                );
            }
            self.through = sequence;
            self.commits += 1;
        }
        if let Some(through) = snapshot.through() {
            self.through = self.through.max(through.get());
            self.published.note_commit(self.through);
        }
        Ok(())
    }

    #[must_use]
    pub const fn through(&self) -> u64 {
        self.through
    }

    pub fn custody(&self) -> Result<ProductCustodySnapshot, String> {
        let model = self
            .model
            .as_ref()
            .ok_or("custody is read before the writer takes the transaction model")?;
        self.custody
            .finish(model)
            .map_err(|error| error.to_string())
    }

    pub fn take_model(&mut self) -> Option<RecoveredTransactionModel<ProductTransaction>> {
        self.model.take()
    }

    #[must_use]
    pub fn checkpoints(&self) -> &[TransactionCheckpoint<ProductTransaction>] {
        &self.checkpoints
    }

    #[must_use]
    pub const fn next_record_key(&self) -> u64 {
        self.next_record_key
    }

    #[must_use]
    pub const fn is_fresh(&self) -> bool {
        self.commits == 0 && self.horizon == 0
    }

    #[must_use]
    pub const fn published(&self) -> &MemoryStore<ProductStore> {
        &self.published
    }

    #[must_use]
    pub fn into_published(self) -> MemoryStore<ProductStore> {
        self.published
    }
}
