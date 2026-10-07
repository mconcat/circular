use super::*;
use std::collections::btree_map::Entry;
use std::ops::ControlFlow;

/// Disposable decoding material for one forward read. A column is a sequence
/// of segments, one per writing hand its actor opened. A segment starts at a
/// CURRENT keyframe and every later item of it names that start, so the read
/// keeps one state per segment start it has met. Segments of one owner may
/// interleave (a closing hand's queued submissions after the next hand's
/// start); each decodes against its own state. Sparse/backward reads rebuild
/// the needed segment; physical cuts never become replay cuts.
#[derive(Default)]
pub struct ProductJournalReader {
    segments: BTreeMap<(u64, u32), State>,
    through: Option<u64>,
}
impl ProductJournalReader {
    pub fn read_entry(
        &mut self,
        entry: &crate::SqliteJournalEntry,
    ) -> Result<StoreTransaction<ProductTransaction>, crate::SqliteJournalError> {
        self.read_payload(entry.sequence().get(), entry.payload())
    }
    /// Forward read of one namespace's payload at its commit sequence. The
    /// caller supplies that namespace's commits in order from its first.
    pub fn read_payload(
        &mut self,
        sequence: u64,
        bytes: &[u8],
    ) -> Result<StoreTransaction<ProductTransaction>, crate::SqliteJournalError> {
        self.read(None, sequence, bytes)
    }
    pub fn read_at(
        &mut self,
        journal: &crate::SqliteJournal,
        sequence: u64,
        bytes: &[u8],
    ) -> Result<StoreTransaction<ProductTransaction>, crate::SqliteJournalError> {
        self.read(Some(journal), sequence, bytes)
    }
    pub fn read_commit(
        &mut self,
        journal: &crate::SqliteJournal,
        sequence: u64,
    ) -> Result<StoreTransaction<ProductTransaction>, crate::SqliteJournalError> {
        let (_, bytes) = journal.namespace_payload_at(sequence)?;
        self.read_at(journal, sequence, &bytes)
    }
    pub(crate) fn decode_at(
        &mut self,
        sequence: u64,
        bytes: &[u8],
    ) -> Result<StoreTransaction<ProductTransaction>, Error> {
        self.read(None, sequence, bytes)
            .map_err(|error| match error {
                crate::SqliteJournalError::UnsupportedRecordFormat {
                    vocabulary, found, ..
                } => Error::UnsupportedRecordFormat { vocabulary, found },
                _ => invalid(),
            })
    }
    fn read(
        &mut self,
        journal: Option<&crate::SqliteJournal>,
        sequence: u64,
        bytes: &[u8],
    ) -> Result<StoreTransaction<ProductTransaction>, crate::SqliteJournalError> {
        let error = |e| journal_error(sequence, e);
        if self.through.is_some_and(|n| {
            sequence <= n || (journal.is_some() && n.checked_add(1) != Some(sequence))
        }) {
            self.segments.clear();
        }
        let ops = operations(bytes).map_err(error)?;
        let mut logical = u32::try_from(ops.len())
            .map_err(|_| error(invalid()))?
            .to_be_bytes()
            .to_vec();
        let mut touched = BTreeMap::new();
        for (index, op) in ops.iter().enumerate() {
            let append = if op.tag == APPEND {
                let (key, frame) = append_frame(op).map_err(error)?;
                let at = (
                    sequence,
                    u32::try_from(index).map_err(|_| error(invalid()))?,
                );
                let segment = frame.address.position(at).map_err(error)?;
                let state = match touched.entry(segment) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => entry.insert(match self.segments.get(&segment) {
                        Some(state) => state.clone(),
                        None if frame.full => State::default(),
                        None => {
                            restore_segment(journal.ok_or_else(|| error(invalid()))?, segment, at)?
                        }
                    }),
                };
                let payload = frame.decode(state, at).map_err(error)?;
                Some((key, payload))
            } else {
                None
            };
            logical.extend(logical_operation(op.tag, op.bytes, append).map_err(error)?);
        }
        let transaction =
            crate::transaction_codec::decode_transaction(&logical, &crate::ProductTransactionCodec)
                .map_err(error)?;
        let canonical = crate::transaction_codec::encode_transaction(
            &transaction,
            &crate::ProductTransactionCodec,
        )
        .map_err(error)?;
        if canonical != logical {
            return Err(error(invalid()));
        }
        self.segments.extend(touched);
        self.through = Some(sequence);
        Ok(transaction)
    }
}

/// Rebuild one segment for a sparse read: find its last keyframe before
/// `before` by physical framing, then decode that segment forward. The search
/// never goes below the segment's own CURRENT keyframe. No checkpoint or
/// shared keyframe index is an input.
fn restore_segment(
    journal: &crate::SqliteJournal,
    segment: (u64, u32),
    before: (u64, u32),
) -> Result<State, crate::SqliteJournalError> {
    let mut start = None;
    journal.visit_namespace_back(
        |sequence, bytes| -> Result<ControlFlow<()>, crate::SqliteJournalError> {
            let commit = sequence.get();
            if commit > before.0 {
                return Ok(ControlFlow::Continue(()));
            }
            if commit < segment.0 {
                return Ok(ControlFlow::Break(()));
            }
            let ops = operations(bytes).map_err(|e| journal_error(commit, e))?;
            for (index, op) in ops.iter().enumerate().rev() {
                let at = (commit, index as u32);
                if at >= before || op.tag != APPEND {
                    continue;
                }
                let (_, frame) = append_frame(op).map_err(|e| journal_error(commit, e))?;
                if frame.full
                    && frame
                        .address
                        .position(at)
                        .map_err(|e| journal_error(commit, e))?
                        == segment
                {
                    start = Some(at);
                    return Ok(ControlFlow::Break(()));
                }
            }
            Ok(ControlFlow::Continue(()))
        },
    )??;
    let start = start.ok_or_else(|| journal_error(before.0, invalid()))?;
    let mut state = State::default();
    journal.visit_namespace(
        start.0 - 1,
        before.0,
        |sequence, bytes| -> Result<(), crate::SqliteJournalError> {
            let commit = sequence.get();
            let ops = operations(bytes).map_err(|e| journal_error(commit, e))?;
            for (index, op) in ops.iter().enumerate() {
                let at = (commit, index as u32);
                if at < start || at >= before || op.tag != APPEND {
                    continue;
                }
                let (_, frame) = append_frame(op).map_err(|e| journal_error(commit, e))?;
                if frame
                    .address
                    .position(at)
                    .map_err(|e| journal_error(commit, e))?
                    == segment
                {
                    frame
                        .decode(&mut state, at)
                        .map_err(|e| journal_error(commit, e))?;
                }
            }
            Ok(())
        },
    )??;
    Ok(state)
}
