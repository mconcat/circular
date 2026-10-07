//! Actor-column storage. A column is a sequence of segments: every writing hand
//! an actor opens (spawn, restart, replacement) starts a new segment with a
//! CURRENT keyframe and reads nothing from the journal. A writer holds only its
//! own segment's context; only a reader may hold contexts for several segments,
//! and only for that read's lifetime.
mod bytes;
mod effect;
mod format;
 mod reader;
mod record;
use crate::{ProductTransaction, StoreTransaction, TransactionCodecError as Error};
use bytes::{Reader, field, uint};
use format::*;
pub use reader::ProductJournalReader;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn invalid() -> Error {
    Error::MalformedField {
        tag: APPEND,
        field: 1,
    }
}

/// Encoder policy only. No decoder accepts this value.
#[derive(Clone, Copy, Debug)]
pub enum KeyframePolicy {
    Every(std::num::NonZeroU64),
    CommitBoundary,
}
impl Default for KeyframePolicy {
    fn default() -> Self {
        Self::Every(std::num::NonZeroU64::new(256).unwrap())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    owner: Vec<u8>,
    address: Option<(u64, u32)>,
    definitions: Vec<(u8, Vec<u8>)>,
    stamps: BTreeMap<u64, [u64; 4]>,
    arrival: Option<u64>,
    observed: Option<u64>,
    external: BTreeMap<u8, Vec<u8>>,
    since_keyframe: u64,
}
impl State {
    fn define(&mut self, kind: u8, value: &[u8]) -> u64 {
        if let Some(i) = self
            .definitions
            .iter()
            .position(|(k, v)| *k == kind && v == value)
        {
            return i as u64 + 1;
        }
        self.definitions.push((kind, value.to_vec()));
        self.definitions.len() as u64
    }
    fn definition(&self, kind: u8, id: u64) -> Result<&[u8], Error> {
        let (k, v) = self
            .definitions
            .get(usize::try_from(id.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?)
            .ok_or_else(invalid)?;
        if *k != kind {
            return Err(invalid());
        }
        Ok(v)
    }
    fn put_ref(&mut self, out: &mut Vec<u8>, kind: u8, value: &[u8]) {
        uint(out, self.define(kind, value));
    }
    fn get_ref(&self, r: &mut Reader<'_>, kind: u8) -> Result<Vec<u8>, Error> {
        Ok(self.definition(kind, r.uint()?)?.to_vec())
    }
}

/// One producer's encoder material for one segment of its column. Clone shares
/// that hand, never an actor-to-dictionary registry. A new hand starts empty;
/// its first append is the segment's CURRENT keyframe.
#[derive(Clone, Debug)]
pub struct ColumnWriteContext {
    state: Arc<Mutex<State>>,
    policy: KeyframePolicy,
}
impl ColumnWriteContext {
    pub fn new(owner: &circular_runtime::ActorId) -> Result<Self, Error> {
        Self::with_policy(owner, KeyframePolicy::default())
    }
    pub fn with_policy(
        owner: &circular_runtime::ActorId,
        policy: KeyframePolicy,
    ) -> Result<Self, Error> {
        use crate::RecordIdentityCodec;
        let owner = crate::ProductRecordCodec
            .producer(owner)
            .map_err(|_| invalid())?;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                owner,
                ..State::default()
            })),
            policy,
        })
    }
    pub fn bind(
        &self,
        transaction: StoreTransaction<ProductTransaction>,
    ) -> StoreTransaction<ProductTransaction> {
        transaction.with_write_context(self.clone())
    }
}

/// Fixture support only. The column a finished record is stored in: the
/// receiving actor for Arrival and Admission, otherwise its stamp's producer.
/// Product submissions never ask; each already carries its own context.
#[cfg(any(test, feature = "test-support"))]
pub fn fixture_column(record: &crate::Record<crate::ProductStore>) -> &circular_runtime::ActorId {
    match record.header().key() {
        crate::ClassKey::Boundary(
            crate::BoundaryKey::Arrival { actor, .. } | crate::BoundaryKey::Admission { actor, .. },
        ) => actor,
        _ => record.header().at().producer(),
    }
}

/// Fixture support only: commit finished records as their owning actors
/// would, in one SQLite group. Each list is split into runs of one column and
/// each run is one transaction. Each call opens one new hand per owner, as a
/// newly standing actor would, so each owner gets a new segment. Product
/// writers never look a column up; they carry their own context.
#[cfg(any(test, feature = "test-support"))]
pub fn commit_fixture_transactions(
    journal: &mut crate::SqliteJournal,
    transactions: Vec<Vec<crate::Record<crate::ProductStore>>>,
) -> Result<(), String> {
    let mut columns: Vec<(circular_runtime::ActorId, ColumnWriteContext)> = Vec::new();
    let mut bound = Vec::new();
    for records in &transactions {
        for run in records.chunk_by(|a, b| fixture_column(a) == fixture_column(b)) {
            let owner = fixture_column(&run[0]);
            let column = match columns.iter().find(|(known, _)| known == owner) {
                Some((_, column)) => column.clone(),
                None => {
                    let column = ColumnWriteContext::new(owner).map_err(|e| format!("{e:?}"))?;
                    columns.push((owner.clone(), column.clone()));
                    column
                }
            };
            let batch = crate::AppendBatch::try_new(run.to_vec()).map_err(|e| format!("{e:?}"))?;
            let transaction = crate::arrival_transaction(&batch).map_err(|e| format!("{e:?}"))?;
            bound.push(column.bind(transaction));
        }
    }
    let prepared = prepare(&bound).map_err(|(index, e)| format!("fixture {index}: {e:?}"))?;
    let receipt = journal
        .commit_group(&prepared.payloads)
        .map_err(|e| e.to_string())?;
    if let Some(committed) = prepared.committed {
        committed(receipt.receipts());
    }
    Ok(())
}

fn journal_error(sequence: u64, error: Error) -> crate::SqliteJournalError {
    crate::ProductJournalCodec.decode_error_at(sequence, error)
}

fn option(out: &mut Vec<u8>, value: Option<u64>) {
    out.push(if value.is_some() { PRESENT } else { ABSENT });
    if let Some(value) = value {
        uint(out, value);
    }
}
fn read_option(r: &mut Reader<'_>) -> Result<Option<u64>, Error> {
    match r.byte()? {
        ABSENT => Ok(None),
        PRESENT => Ok(Some(r.uint()?)),
        _ => Err(invalid()),
    }
}
fn number(out: &mut Vec<u8>, value: u64, before: Option<u64>) {
    let mut choices = Vec::new();
    for tag in NUMBER_TIE_ORDER {
        let n = match tag {
            ABSOLUTE => Some(value),
            INCREASE => before.and_then(|b| value.checked_sub(b)),
            DECREASE => before.and_then(|b| b.checked_sub(value)),
            _ => unreachable!(),
        };
        if let Some(n) = n {
            let mut b = vec![tag];
            uint(&mut b, n);
            choices.push(b);
        }
    }
    let best = choices.iter().min_by_key(|b| b.len()).unwrap();
    out.extend_from_slice(best);
}
fn read_number(r: &mut Reader<'_>, before: Option<u64>) -> Result<u64, Error> {
    let tag = r.byte()?;
    let n = r.uint()?;
    match tag {
        ABSOLUTE => Some(n),
        INCREASE => before.and_then(|b| b.checked_add(n)),
        DECREASE => before.and_then(|b| b.checked_sub(n)),
        _ => None,
    }
    .ok_or_else(invalid)
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct StampValue {
    producer: Vec<u8>,
    numbers: [u64; 4],
}
impl StampValue {
    fn raw(r: &mut Reader<'_>) -> Result<Self, Error> {
        let l = r.u64()?;
        let c = r.u64()?;
        let producer = r.fixed_field()?.to_vec();
        let sequence = r.u64()?;
        let revision = r.u64()?;
        Ok(Self {
            producer,
            numbers: [revision, l, c, sequence],
        })
    }
    fn put_raw(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        out.extend_from_slice(&self.numbers[1].to_be_bytes());
        out.extend_from_slice(&self.numbers[2].to_be_bytes());
        bytes::fixed_field(out, &self.producer)?;
        out.extend_from_slice(&self.numbers[3].to_be_bytes());
        out.extend_from_slice(&self.numbers[0].to_be_bytes());
        Ok(())
    }
}
fn stamp(
    out: &mut Vec<u8>,
    value: &StampValue,
    state: &mut State,
    seen: &mut Vec<(u64, [u64; 4])>,
    full: bool,
) {
    let producer = state.define(ACTOR, &value.producer);
    let mut choices = Vec::new();
    for tag in STAMP_TIE_ORDER {
        let mut b = vec![tag];
        match tag {
            STAMP_LOCAL => {
                let Some(i) = seen.iter().position(|v| *v == (producer, value.numbers)) else {
                    continue;
                };
                uint(&mut b, i as u64 + 1);
            }
            STAMP_PREVIOUS if !full && state.stamps.get(&producer) == Some(&value.numbers) => {
                uint(&mut b, producer)
            }
            STAMP_PREVIOUS => continue,
            STAMP_ABSOLUTE => {
                uint(&mut b, producer);
                for n in value.numbers {
                    uint(&mut b, n);
                }
            }
            STAMP_DELTA if !full => {
                let Some(base) = state.stamps.get(&producer) else {
                    continue;
                };
                uint(&mut b, producer);
                for (v, b0) in value.numbers.into_iter().zip(base) {
                    number(&mut b, v, Some(*b0));
                }
            }
            STAMP_DELTA => continue,
            _ => unreachable!(),
        }
        choices.push(b);
    }
    out.extend_from_slice(choices.iter().min_by_key(|b| b.len()).unwrap());
    seen.push((producer, value.numbers));
}
fn read_stamp(
    r: &mut Reader<'_>,
    state: &State,
    seen: &mut Vec<(u64, [u64; 4])>,
    full: bool,
) -> Result<StampValue, Error> {
    let tag = r.byte()?;
    let id = r.uint()?;
    let (producer, numbers) = match tag {
        STAMP_LOCAL => *seen
            .get(usize::try_from(id.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?)
            .ok_or_else(invalid)?,
        STAMP_PREVIOUS if !full => (id, *state.stamps.get(&id).ok_or_else(invalid)?),
        STAMP_ABSOLUTE => {
            let mut n = [0; 4];
            for v in &mut n {
                *v = r.uint()?;
            }
            (id, n)
        }
        STAMP_DELTA if !full => {
            let base = state.stamps.get(&id).ok_or_else(invalid)?;
            let mut n = [0; 4];
            for (v, b) in n.iter_mut().zip(base) {
                *v = read_number(r, Some(*b))?;
            }
            (id, n)
        }
        _ => return Err(invalid()),
    };
    if numbers[0] == 0 || numbers[3] == u64::MAX {
        return Err(invalid());
    }
    let value = StampValue {
        producer: state.definition(ACTOR, producer)?.to_vec(),
        numbers,
    };
    seen.push((producer, numbers));
    Ok(value)
}
fn definitions(out: &mut Vec<u8>, definitions: &[(u8, Vec<u8>)]) {
    uint(out, definitions.len() as u64);
    for (kind, bytes) in definitions {
        out.push(*kind);
        field(out, bytes);
    }
}
fn read_definitions(r: &mut Reader<'_>, state: &mut State) -> Result<(), Error> {
    let count = r.uint()?;
    for _ in 0..count {
        let kind = r.byte()?;
        if ![ACTOR, EDGE, SCOPE, PORT].contains(&kind) {
            return Err(invalid());
        }
        let value = r.field()?;
        if state
            .definitions
            .iter()
            .any(|(k, v)| *k == kind && v == value)
        {
            return Err(invalid());
        }
        state.definitions.push((kind, value.to_vec()));
    }
    Ok(())
}
fn bases(out: &mut Vec<u8>, state: &State) {
    uint(out, state.stamps.len() as u64);
    for (producer, numbers) in &state.stamps {
        uint(out, *producer);
        for n in numbers {
            uint(out, *n);
        }
    }
    option(out, state.arrival);
    option(out, state.observed);
    uint(out, state.external.len() as u64);
    for (kind, value) in &state.external {
        out.push(*kind);
        field(out, value);
    }
}
fn read_bases(r: &mut Reader<'_>, state: &mut State) -> Result<(), Error> {
    for _ in 0..r.uint()? {
        let id = r.uint()?;
        state.definition(ACTOR, id)?;
        let mut numbers = [0; 4];
        for n in &mut numbers {
            *n = r.uint()?;
        }
        if numbers[0] == 0 || numbers[3] == u64::MAX || state.stamps.insert(id, numbers).is_some() {
            return Err(invalid());
        }
    }
    state.arrival = read_option(r)?;
    state.observed = read_option(r)?;
    for _ in 0..r.uint()? {
        let kind = r.byte()?;
        if kind != FROM_EXTERNAL {
            return Err(invalid());
        }
        let value = r.field()?.to_vec();
        if state.external.insert(kind, value).is_some() {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(super) struct Operation<'a> {
    pub tag: u8,
    pub bytes: &'a [u8],
}
pub(super) fn operations(bytes: &[u8]) -> Result<Vec<Operation<'_>>, Error> {
    let mut r = Reader::new(bytes);
    let version = r.byte()?;
    if version != VERSION {
        return Err(Error::UnsupportedRecordFormat {
            vocabulary: "column_version",
            found: version.into(),
        });
    }
    let count = r.uint()?;
    if count == 0 {
        return Err(Error::Empty);
    }
    let mut result = Vec::new();
    for _ in 0..count {
        let tag = r.byte()?;
        if !OPERATIONS.contains(&tag) {
            return Err(Error::UnknownOperation(tag));
        }
        result.push(Operation {
            tag,
            bytes: r.field()?,
        });
    }
    r.done()?;
    Ok(result)
}
fn append_frame<'a>(op: &Operation<'a>) -> Result<(u64, record::Frame<'a>), Error> {
    let mut r = Reader::new(op.bytes);
    let key = r.uint()?;
    Ok((key, record::Frame::parse(r.rest())?))
}

struct Candidate {
    context: ColumnWriteContext,
    state: State,
    pending_address: Option<(usize, u32)>,
}

pub(crate) fn prepare(
    transactions: &[StoreTransaction<ProductTransaction>],
) -> Result<crate::sqlite::PreparedSqliteBatch, (usize, Error)> {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut payloads = Vec::new();
    for (transaction_index, transaction) in transactions.iter().enumerate() {
        let has_append = transaction
            .operations()
            .iter()
            .any(|op| matches!(op, crate::StoreTransactionOp::Append(_)));
        let candidate = if has_append {
            let context = transaction.write_context().ok_or((
                transaction_index,
                Error::UnencodableIdentity("missing column write context"),
            ))?;
            let index = if let Some(index) = candidates
                .iter()
                .position(|c| Arc::ptr_eq(&c.context.state, &context.state))
            {
                index
            } else {
                let state = context
                    .state
                    .lock()
                    .map_err(|_| (transaction_index, invalid()))?
                    .clone();
                candidates.push(Candidate {
                    context: context.clone(),
                    state,
                    pending_address: None,
                });
                candidates.len() - 1
            };
            Some(&mut candidates[index])
        } else {
            None
        };
        payloads.push(
            encode_transaction(transaction, candidate, transaction_index)
                .map_err(|error| (transaction_index, error))?,
        );
    }
    Ok(crate::sqlite::PreparedSqliteBatch {
        payloads,
        committed: Some(Box::new(move |receipts| {
            for mut candidate in candidates {
                if let Some((transaction, operation)) = candidate.pending_address {
                    candidate.state.address =
                        Some((receipts[transaction].sequence().get(), operation));
                }
                *candidate
                    .context
                    .state
                    .lock()
                    .expect("column writer owns context") = candidate.state;
            }
        })),
    })
}
fn encode_transaction(
    transaction: &StoreTransaction<ProductTransaction>,
    mut candidate: Option<&mut Candidate>,
    transaction_index: usize,
) -> Result<Vec<u8>, Error> {
    let raw =
        crate::transaction_codec::encode_transaction(transaction, &crate::ProductTransactionCodec)?;
    let mut r = Reader::new(&raw);
    let count = r.u32()?;
    let mut out = vec![VERSION];
    uint(&mut out, count.into());
    let mut first_append = true;
    let mut local = None;
    for operation in 0..count {
        let logical_tag = r.byte()?;
        let fields = r.byte()?;
        let tag = logical_tag;
        if !OPERATIONS.contains(&tag) {
            return Err(invalid());
        }
        let mut encoded = Vec::new();
        if tag == APPEND {
            if fields != 2 {
                return Err(invalid());
            }
            let key = u64::from_be_bytes(r.fixed_field()?.try_into().map_err(|_| invalid())?);
            uint(&mut encoded, key);
            let payload = circular_core::EncodedPayload::from_bytes(r.fixed_field()?)
                .map_err(|_| invalid())?;
            let parts = record::Parts::parse(&payload)?;
            let candidate = candidate.as_deref_mut().ok_or_else(invalid)?;
            let address = if let Some((c, o)) = candidate.state.address {
                record::Address::Committed(c, o)
            } else if let Some(o) = local {
                record::Address::Local(o)
            } else {
                record::Address::Current
            };
            let keyframe = address == record::Address::Current
                || match candidate.context.policy {
                    KeyframePolicy::Every(k) => candidate.state.since_keyframe >= k.get(),
                    KeyframePolicy::CommitBoundary => first_append,
                };
            encoded.extend(record::encode_frame(
                &parts,
                &mut candidate.state,
                keyframe,
                address,
            )?);
            if address == record::Address::Current {
                local = Some(operation);
                candidate.pending_address = Some((transaction_index, operation));
            }
            first_append = false;
        } else {
            uint(&mut encoded, fields.into());
            for _ in 0..fields {
                field(&mut encoded, r.fixed_field()?);
            }
        }
        out.push(tag);
        field(&mut out, &encoded);
    }
    r.done()?;
    Ok(out)
}

pub(super) fn logical_operation(
    tag: u8,
    encoded: &[u8],
    append: Option<(u64, circular_core::EncodedPayload)>,
) -> Result<Vec<u8>, Error> {
    if !OPERATIONS.contains(&tag) {
        return Err(invalid());
    }
    let mut out = vec![tag];
    if let Some((key, payload)) = append {
        out.push(2);
        bytes::fixed_field(&mut out, &key.to_be_bytes())?;
        bytes::fixed_field(&mut out, payload.as_bytes())?;
    } else {
        let mut r = Reader::new(encoded);
        let n = u8::try_from(r.uint()?).map_err(|_| invalid())?;
        out.push(n);
        for _ in 0..n {
            bytes::fixed_field(&mut out, r.field()?)?;
        }
        r.done()?;
    }
    Ok(out)
}
