
use crate::transaction::{
    StoreTransaction, StoreTransactionOp, TransactionAppend, TransactionCheckpoint,
    TransactionObservation, TransactionSchema,
};

pub const APPEND_FRONTIER: u8 = 16;

pub const fn op_tag<S: TransactionSchema>(operation: &StoreTransactionOp<S>) -> u8 {
    match operation {
        StoreTransactionOp::Append(_) => 1,
        StoreTransactionOp::AppendObservation(_) => 2,
        StoreTransactionOp::OpenOutbox { .. } => 5,
        StoreTransactionOp::SubmitOutbox { .. } => 6,
        StoreTransactionOp::AcquireOutboxDispatch { .. } => 7,
        StoreTransactionOp::SettleOutbox { .. } => 8,
        StoreTransactionOp::CancelCommittedOutbox { .. } => 9,
        StoreTransactionOp::OpenApproval { .. } => 10,
        StoreTransactionOp::ApproveApproval { .. } => 11,
        StoreTransactionOp::SettleApproval { .. } => 12,
        StoreTransactionOp::ReplaceCheckpoint(_) => 13,
        StoreTransactionOp::SettleCheckpoint { .. } => 14,
    }
}

pub trait TransactionValueCodec<S: TransactionSchema> {
    type Error: Into<TransactionCodecError>;

    fn record_key(&self, value: &S::RecordKey) -> Vec<u8>;
    fn record(&self, value: &S::Record) -> Vec<u8>;
    fn effect_id(&self, value: &S::EffectId) -> Vec<u8>;
    fn outbox(&self, value: &S::Outbox) -> Vec<u8>;
    fn approval_key(&self, value: &S::ApprovalKey) -> Vec<u8>;
    fn approval(&self, value: &S::Approval) -> Vec<u8>;
    fn approval_ticket(&self, value: &S::ApprovalTicket) -> Vec<u8>;
    fn actor_id(&self, value: &S::ActorId) -> Result<Vec<u8>, TransactionCodecError>;
    fn incarnation(&self, value: &S::Incarnation) -> Vec<u8>;
    fn checkpoint_stamp(&self, value: &S::CheckpointStamp) -> Vec<u8>;
    fn checkpoint_state(&self, value: &S::CheckpointState) -> Vec<u8>;
    fn outcome(&self, value: &S::Outcome) -> Vec<u8>;
    fn observation_key(&self, value: &S::ObservationKey) -> Vec<u8>;
    fn observation(&self, value: &S::Observation) -> Vec<u8>;

    fn decode(&self, bytes: TransactionParts<'_>) -> Result<StoreTransactionOp<S>, Self::Error>;
}

#[derive(Clone, Copy, Debug)]
pub struct TransactionParts<'bytes> {
    pub tag: u8,
    pub fields: &'bytes [&'bytes [u8]],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionCodecError {
    Truncated,
    UnencodableIdentity(&'static str),
    UnknownOperation(u8),
    /// A retired record vocabulary is identified before storage canonicality checks.
    UnsupportedRecordFormat {
        vocabulary: &'static str,
        found: u64,
    },
    UnexpectedFieldCount { tag: u8, expected: u8, actual: u8 },
    MalformedField { tag: u8, field: u8 },
    LengthOutOfRange,
    Empty,
}

fn push_field(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), TransactionCodecError> {
    let length = u32::try_from(bytes.len()).map_err(|_| TransactionCodecError::LengthOutOfRange)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

pub fn encode_transaction<S, C>(
    transaction: &StoreTransaction<S>,
    codec: &C,
) -> Result<Vec<u8>, TransactionCodecError>
where
    S: TransactionSchema,
    C: TransactionValueCodec<S>,
{
    let operations = transaction.operations();
    let count =
        u32::try_from(operations.len()).map_err(|_| TransactionCodecError::LengthOutOfRange)?;
    let mut output = count.to_be_bytes().to_vec();
    for operation in operations {
        output.push(op_tag(operation));
        let fields = operation_fields(operation, codec)?;
        let field_count =
            u8::try_from(fields.len()).map_err(|_| TransactionCodecError::LengthOutOfRange)?;
        output.push(field_count);
        for field in &fields {
            push_field(&mut output, field)?;
        }
    }
    Ok(output)
}

fn observation_fields<S, C>(observation: &TransactionObservation<S>, codec: &C) -> Vec<Vec<u8>>
where
    S: TransactionSchema,
    C: TransactionValueCodec<S>,
{
    vec![
        codec.observation_key(observation.key()),
        codec.observation(observation.value()),
    ]
}

fn checkpoint_fields<S, C>(
    checkpoint: &TransactionCheckpoint<S>,
    codec: &C,
) -> Result<Vec<Vec<u8>>, TransactionCodecError>
where
    S: TransactionSchema,
    C: TransactionValueCodec<S>,
{
    let mut fields = vec![
        codec.actor_id(checkpoint.actor())?,
        codec.incarnation(checkpoint.owner()),
        codec.checkpoint_stamp(checkpoint.at()),
        codec.checkpoint_state(checkpoint.state()),
    ];
    let pending = checkpoint.pending();
    let mut encoded = u32::try_from(pending.len())
        .unwrap_or(u32::MAX)
        .to_be_bytes()
        .to_vec();
    for effect in pending {
        let bytes = codec.effect_id(effect);
        encoded.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&bytes);
    }
    fields.push(encoded);
    Ok(fields)
}

fn append_fields<S, C>(append: &TransactionAppend<S>, codec: &C) -> Vec<Vec<u8>>
where
    S: TransactionSchema,
    C: TransactionValueCodec<S>,
{
    vec![
        codec.record_key(append.key()),
        codec.record(append.record()),
    ]
}

fn operation_fields<S, C>(
    operation: &StoreTransactionOp<S>,
    codec: &C,
) -> Result<Vec<Vec<u8>>, TransactionCodecError>
where
    S: TransactionSchema,
    C: TransactionValueCodec<S>,
{
    Ok(match operation {
        StoreTransactionOp::Append(append) => append_fields(append, codec),
        StoreTransactionOp::AppendObservation(observation) => {
            observation_fields(observation, codec)
        }
        StoreTransactionOp::OpenOutbox { effect, request } => {
            vec![codec.effect_id(effect), codec.outbox(request)]
        }
        StoreTransactionOp::SubmitOutbox { effect }
        | StoreTransactionOp::AcquireOutboxDispatch { effect } => {
            vec![codec.effect_id(effect)]
        }
        StoreTransactionOp::SettleOutbox {
            effect,
            outcome,
            observation,
        }
        | StoreTransactionOp::CancelCommittedOutbox {
            effect,
            outcome,
            observation,
        } => {
            let mut fields = vec![codec.effect_id(effect), codec.outcome(outcome)];
            fields.extend(observation_fields(observation, codec));
            fields
        }
        StoreTransactionOp::OpenApproval { key, approval } => {
            vec![codec.approval_key(key), codec.approval(approval)]
        }
        StoreTransactionOp::ApproveApproval { key, ticket } => {
            vec![codec.approval_key(key), codec.approval_ticket(ticket)]
        }
        StoreTransactionOp::SettleApproval { key, observation } => {
            let mut fields = vec![codec.approval_key(key)];
            fields.extend(observation_fields(observation, codec));
            fields
        }
        StoreTransactionOp::ReplaceCheckpoint(checkpoint) => checkpoint_fields(checkpoint, codec)?,
        StoreTransactionOp::SettleCheckpoint {
            actor,
            at,
            observation,
        } => {
            let mut fields = vec![codec.actor_id(actor)?, codec.checkpoint_stamp(at)];
            fields.extend(observation_fields(observation, codec));
            fields
        }
    })
}

pub fn decode_transaction<S, C>(
    bytes: &[u8],
    codec: &C,
) -> Result<StoreTransaction<S>, TransactionCodecError>
where
    S: TransactionSchema,
    C: TransactionValueCodec<S>,
{
    let mut cursor = 0_usize;
    let count = read_u32(bytes, &mut cursor)? as usize;
    let mut operations = Vec::with_capacity(count);
    for _ in 0..count {
        let tag = *bytes.get(cursor).ok_or(TransactionCodecError::Truncated)?;
        cursor += 1;
        if tag == 0 || tag >= APPEND_FRONTIER {
            return Err(TransactionCodecError::UnknownOperation(tag));
        }
        let field_count = *bytes.get(cursor).ok_or(TransactionCodecError::Truncated)?;
        cursor += 1;
        let mut fields = Vec::with_capacity(field_count as usize);
        for _ in 0..field_count {
            let length = read_u32(bytes, &mut cursor)? as usize;
            let end = cursor
                .checked_add(length)
                .ok_or(TransactionCodecError::LengthOutOfRange)?;
            fields.push(
                bytes
                    .get(cursor..end)
                    .ok_or(TransactionCodecError::Truncated)?,
            );
            cursor = end;
        }
        let parts = TransactionParts {
            tag,
            fields: &fields,
        };
        operations.push(codec.decode(parts).map_err(Into::into)?);
    }
    StoreTransaction::try_new(operations).map_err(|_| TransactionCodecError::Empty)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, TransactionCodecError> {
    let end = cursor
        .checked_add(4)
        .ok_or(TransactionCodecError::LengthOutOfRange)?;
    let slice = bytes
        .get(*cursor..end)
        .ok_or(TransactionCodecError::Truncated)?;
    *cursor = end;
    Ok(u32::from_be_bytes(slice.try_into().expect("4 bytes")))
}

#[cfg(test)]
mod tests {
    use super::{
        APPEND_FRONTIER, TransactionCodecError, TransactionParts, TransactionValueCodec,
        decode_transaction, encode_transaction, op_tag,
    };
    use crate::transaction::{
        StoreTransaction, StoreTransactionOp, TransactionAppend, TransactionObservation,
        TransactionSchema,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Schema;

    macro_rules! u64_types {
        ($($name:ident),* $(,)?) => { $(type $name = u64;)* };
    }

    impl TransactionSchema for Schema {
        type WriteContext = ();
        fn record_digest(record: &Self::Record) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&record.to_be_bytes())
        }

        fn observation_digest(observation: &Self::Observation) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&observation.to_be_bytes())
        }

        u64_types!(
            RecordKey,
            Record,
            EffectId,
            Outbox,
            ApprovalKey,
            Approval,
            ApprovalTicket,
            ActorId,
            Incarnation,
            CheckpointStamp,
            CheckpointState,
            Outcome,
            ObservationKey,
            Observation,
        );
    }

    struct Codec;

    fn be(value: &u64) -> Vec<u8> {
        value.to_be_bytes().to_vec()
    }

    fn read(bytes: &[u8]) -> u64 {
        u64::from_be_bytes(bytes.try_into().expect("eight bytes"))
    }

    impl TransactionValueCodec<Schema> for Codec {
        type Error = TransactionCodecError;

        fn record_key(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn record(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn effect_id(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn outbox(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn approval_key(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn approval(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn approval_ticket(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn actor_id(&self, value: &u64) -> Result<Vec<u8>, TransactionCodecError> {
            Ok(be(value))
        }
        fn incarnation(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn checkpoint_stamp(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn checkpoint_state(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn outcome(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn observation_key(&self, value: &u64) -> Vec<u8> {
            be(value)
        }
        fn observation(&self, value: &u64) -> Vec<u8> {
            be(value)
        }

        fn decode(
            &self,
            parts: TransactionParts<'_>,
        ) -> Result<StoreTransactionOp<Schema>, Self::Error> {
            let field = |index: usize| -> Result<u64, TransactionCodecError> {
                parts.fields.get(index).copied().map(read).ok_or(
                    TransactionCodecError::MalformedField {
                        tag: parts.tag,
                        field: u8::try_from(index).expect("the test field slot is a u8"),
                    },
                )
            };
            Ok(match parts.tag {
                1 => StoreTransactionOp::Append(TransactionAppend::new(field(0)?, field(1)?)),
                2 => StoreTransactionOp::AppendObservation(TransactionObservation::new(
                    field(0)?,
                    field(1)?,
                )),
                6 => StoreTransactionOp::SubmitOutbox { effect: field(0)? },
                12 => StoreTransactionOp::SettleApproval {
                    key: field(0)?,
                    observation: TransactionObservation::new(field(1)?, field(2)?),
                },
                _ => return Err(TransactionCodecError::UnknownOperation(parts.tag)),
            })
        }
    }

    #[test]
    fn a_transaction_round_trips_through_its_bytes() {
        let transaction = StoreTransaction::<Schema>::try_new(vec![
            StoreTransactionOp::Append(TransactionAppend::new(7, 70)),
            StoreTransactionOp::SubmitOutbox { effect: 9 },
            StoreTransactionOp::SettleApproval {
                key: 3,
                observation: TransactionObservation::new(4, 40),
            },
        ])
        .expect("three is not empty");

        let bytes = encode_transaction(&transaction, &Codec).expect("encode");
        let decoded = decode_transaction::<Schema, Codec>(&bytes, &Codec).expect("decode");
        assert_eq!(decoded, transaction);

        let again = encode_transaction(&decoded, &Codec).expect("re-encode");
        assert_eq!(again, bytes);
    }

    #[test]
    fn a_reserved_or_unassigned_operation_tag_is_refused() {
        let transaction =
            StoreTransaction::<Schema>::try_new(vec![StoreTransactionOp::SubmitOutbox {
                effect: 9,
            }])
            .expect("one is not empty");
        let good = encode_transaction(&transaction, &Codec).expect("encode");
        let tag_at = 4;
        assert_eq!(good[tag_at], 6);

        for bad in [0_u8, APPEND_FRONTIER, 255] {
            let mut broken = good.clone();
            broken[tag_at] = bad;
            assert_eq!(
                decode_transaction::<Schema, Codec>(&broken, &Codec),
                Err(TransactionCodecError::UnknownOperation(bad)),
                "tag {bad} passed"
            );
        }
    }
}
