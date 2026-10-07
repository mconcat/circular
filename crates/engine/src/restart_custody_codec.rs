
use circular_core::{
    ArrivalIndex, Boundary, Ceilings, EncodedPayload, PayloadVersionTag, RecordedInstant, Tick,
    Value,
};
use circular_plan::{ActorId, Generation};
use circular_runtime::ActorState;
use circular_store::ApprovalTerminal;

/// Supplied by the canonical coordinate/term owner, never by a fallback encoder.
/// decode_approval_term must reject non-RequestApproval terms. Both decoders must
/// consume the whole byte string and reject noncanonical nested representations.
pub trait RestoreNestedCodec {
    type ApprovalTerm: Clone;
    type Stamp: Clone;
    fn encode_approval_term(&self, term: &Self::ApprovalTerm) -> Result<Vec<u8>, String>;
    fn decode_approval_term(&self, bytes: &[u8]) -> Result<Self::ApprovalTerm, String>;
    fn encode_stamp(&self, stamp: &Self::Stamp) -> Result<Vec<u8>, String>;
    fn decode_stamp(&self, bytes: &[u8]) -> Result<Self::Stamp, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRestoreBody<T, S> {
    pub actor: ActorId,
    pub request_term: T,
    pub cause: (ArrivalIndex, S),
    pub submitted_at: RecordedInstant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointRestoreBody<S> {
    pub state: Option<ActorState<u16>>,
    pub covered_arrival: Option<(ArrivalIndex, S)>,
    pub generations: Box<[Generation]>,
    pub continuation: CheckpointContinuation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointContinuation {
    pub held: Box<[ArrivalIndex]>,
    pub issuance: Option<CheckpointIssuance>,
    pub at: RecordedInstant,
    pub revision: circular_core::RevisionEpochId,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CheckpointIssuance {
    pub horizon: ArrivalIndex,
    pub own: Option<circular_core::Hlc>,
    pub emitted: Option<circular_core::Sequence>,
    pub stamped: Option<circular_core::Sequence>,
}

impl CheckpointIssuance {
    fn value(&self) -> Value {
        let sequence = |value: Option<circular_core::Sequence>| {
            value.map_or(Value::Null, |sequence| Value::UInt(sequence.get()))
        };
        Value::array([
            Value::UInt(self.horizon.get()),
            self.own.map_or(Value::Null, |own| {
                Value::array([Value::UInt(own.l().get()), Value::UInt(own.c().get())])
            }),
            sequence(self.emitted),
            sequence(self.stamped),
        ])
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let [horizon, own, emitted, stamped] = array(value)?;
        let sequence = |value: &Value| match value {
            Value::Null => Ok(None),
            value => circular_core::Sequence::new(uint(value)?)
                .map(Some)
                .map_err(|error| format!("checkpoint issuance sequence: {error:?}")),
        };
        Ok(Self {
            horizon: ArrivalIndex::new(uint(horizon)?),
            own: match own {
                Value::Null => None,
                value => {
                    let [l, c] = array(value)?;
                    Some(circular_core::Hlc::new(
                        Tick::new(uint(l)?),
                        circular_core::LogicalCounter::new(uint(c)?),
                    ))
                }
            },
            emitted: sequence(emitted)?,
            stamped: sequence(stamped)?,
        })
    }
}

impl CheckpointContinuation {
    #[must_use]
    pub fn without_issuance(at: RecordedInstant, revision: circular_core::RevisionEpochId) -> Self {
        Self {
            held: Box::new([]),
            issuance: None,
            at,
            revision,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.held.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("checkpoint held arrivals must be strictly ascending".into());
        }
        circular_core::encode(
            &Value::array([
                Value::array(self.held.iter().map(|index| Value::UInt(index.get()))),
                self.issuance
                    .as_ref()
                    .map_or(Value::Null, CheckpointIssuance::value),
                Value::UInt(self.at.millis()),
                Value::UInt(self.revision.get()),
            ]),
            Ceilings::for_boundary(Boundary::Journal),
        )
        .map_err(|error| format!("checkpoint continuation encoding: {error}"))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let value = circular_core::decode(bytes, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|error| format!("checkpoint continuation decoding: {error}"))?;
        let [held, issuance, at, revision] = array(&value)?;
        let held = held
            .as_array()
            .ok_or("checkpoint held arrivals require an array")?
            .iter()
            .map(|value| uint(value).map(ArrivalIndex::new))
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        let issuance = match issuance {
            Value::Null => None,
            value => Some(CheckpointIssuance::from_value(value)?),
        };
        let continuation = Self {
            held,
            issuance,
            at: RecordedInstant::from_millis(uint(at)?),
            revision: circular_core::RevisionEpochId::new(uint(revision)?)
                .ok_or("checkpoint revision must be positive")?,
        };
        if continuation.encode()? != bytes {
            return Err("noncanonical checkpoint continuation".into());
        }
        Ok(continuation)
    }
}

fn encode(value: Value) -> Result<EncodedPayload, String> {
    circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
        .map(|bytes| EncodedPayload::new(PayloadVersionTag::FIRST, &bytes))
        .map_err(|error| format!("restore body encoding: {error}"))
}
fn decode(payload: &EncodedPayload) -> Result<Value, String> {
    if payload.version_tag() != PayloadVersionTag::FIRST {
        return Err("unsupported restore body version".into());
    }
    circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
        .map_err(|error| format!("restore body decoding: {error}"))
}
fn array<const N: usize>(value: &Value) -> Result<&[Value; N], String> {
    value
        .as_array()
        .and_then(|items| items.try_into().ok())
        .ok_or_else(|| format!("restore body requires array arity {N}"))
}
fn uint(value: &Value) -> Result<u64, String> {
    match value {
        Value::UInt(value) => Ok(*value),
        _ => Err("restore body requires UInt".into()),
    }
}

fn bytes(value: &Value) -> Result<&[u8], String> {
    value
        .as_bytes()
        .ok_or_else(|| "restore body requires Bytes".into())
}

pub fn encode_approval_body<C: RestoreNestedCodec>(
    codec: &C,
    body: &ApprovalRestoreBody<C::ApprovalTerm, C::Stamp>,
) -> Result<EncodedPayload, String> {
    encode(Value::array([
        circular_store::actor_value(&body.actor)
            .map_err(|error| format!("approval actor: {error:?}"))?,
        Value::bytes(codec.encode_approval_term(&body.request_term)?),
        Value::array([
            Value::UInt(body.cause.0.get()),
            Value::bytes(codec.encode_stamp(&body.cause.1)?),
        ]),
        Value::UInt(body.submitted_at.millis()),
    ]))
}

pub fn decode_approval_body<C: RestoreNestedCodec>(
    codec: &C,
    payload: &EncodedPayload,
) -> Result<ApprovalRestoreBody<C::ApprovalTerm, C::Stamp>, String> {
    let value = decode(payload)?;
    let [actor, term, cause, submitted] = array(&value)?;
    let [cause_index, cause_stamp] = array(cause)?;
    let body = ApprovalRestoreBody {
        actor: circular_store::actor_from_value(actor)
            .map_err(|error| format!("approval actor: {error:?}"))?,
        request_term: codec.decode_approval_term(bytes(term)?)?,
        cause: (
            ArrivalIndex::new(uint(cause_index)?),
            codec.decode_stamp(bytes(cause_stamp)?)?,
        ),
        submitted_at: RecordedInstant::from_millis(uint(submitted)?),
    };
    if encode_approval_body(codec, &body)? != *payload {
        return Err("noncanonical approval restore body".into());
    }
    Ok(body)
}

pub fn encode_checkpoint_body<C: RestoreNestedCodec>(
    codec: &C,
    body: &CheckpointRestoreBody<C::Stamp>,
) -> Result<EncodedPayload, String> {
    if body.generations.is_empty() {
        return Err("checkpoint requires an actor generation".into());
    }
    let state = body.state.as_ref().map_or(Value::Null, |state| {
        Value::array([
            Value::UInt(u64::from(*state.schema())),
            Value::bytes(state.bytes()),
        ])
    });
    let covered = match &body.covered_arrival {
        None => Value::Null,
        Some((index, stamp)) => Value::array([
            Value::UInt(index.get()),
            Value::bytes(codec.encode_stamp(stamp)?),
        ]),
    };
    let covered_index = body.covered_arrival.as_ref().map(|(index, _)| *index);
    if body
        .continuation
        .held
        .iter()
        .any(|held| covered_index.is_none_or(|covered| *held >= covered))
    {
        return Err("checkpoint held arrival is not before its covered arrival".into());
    }
    encode(Value::array([
        state,
        covered,
        Value::array(
            body.generations
                .iter()
                .map(|generation| Value::UInt(generation.get())),
        ),
        Value::bytes(body.continuation.encode()?),
    ]))
}

pub fn decode_checkpoint_body<C: RestoreNestedCodec>(
    codec: &C,
    payload: &EncodedPayload,
) -> Result<CheckpointRestoreBody<C::Stamp>, String> {
    let value = decode(payload)?;
    let [state, covered, generations, continuation] = array(&value)?;
    let state = match state {
        Value::Null => None,
        value => {
            let [schema, bytes_value] = array(value)?;
            Some(ActorState::new(
                u16::try_from(uint(schema)?).map_err(|_| "actor schema exceeds u16")?,
                bytes(bytes_value)?,
            ))
        }
    };
    let covered_arrival = match covered {
        Value::Null => None,
        value => {
            let [index, stamp] = array(value)?;
            Some((
                ArrivalIndex::new(uint(index)?),
                codec.decode_stamp(bytes(stamp)?)?,
            ))
        }
    };
    let generations = generations
        .as_array()
        .ok_or("checkpoint generations require an array")?
        .iter()
        .map(|value| uint(value).map(Generation::new))
        .collect::<Result<Vec<_>, _>>()?
        .into_boxed_slice();
    let body = CheckpointRestoreBody {
        state,
        covered_arrival,
        generations,
        continuation: CheckpointContinuation::decode(bytes(continuation)?)?,
    };
    if encode_checkpoint_body(codec, &body)? != *payload {
        return Err("noncanonical checkpoint restore body".into());
    }
    Ok(body)
}

pub fn encode_checkpoint_settlement(
    actor: &ActorId,
    owner: circular_store::IncarnationId,
    at: circular_store::OpaqueId,
) -> Result<EncodedPayload, String> {
    encode(Value::array([
        circular_store::actor_value(actor).map_err(|e| format!("checkpoint actor: {e:?}"))?,
        Value::UInt(owner.get()),
        Value::UInt(at.get()),
    ]))
}

pub fn decode_checkpoint_settlement(
    payload: &EncodedPayload,
) -> Result<
    (
        ActorId,
        circular_store::IncarnationId,
        circular_store::OpaqueId,
    ),
    String,
> {
    let value = decode(payload)?;
    let [actor, owner, at] = array(&value)?;
    let actor =
        circular_store::actor_from_value(actor).map_err(|e| format!("checkpoint actor: {e:?}"))?;
    let owner = circular_store::IncarnationId::new(uint(owner)?);
    let at = circular_store::OpaqueId::new(uint(at)?);
    if encode_checkpoint_settlement(&actor, owner, at)? != *payload {
        return Err("noncanonical checkpoint settlement".into());
    }
    Ok((actor, owner, at))
}

pub fn encode_approval_terminal(terminal: ApprovalTerminal) -> Result<EncodedPayload, String> {
    encode(Value::UInt(terminal.tag()))
}
pub fn decode_approval_terminal(payload: &EncodedPayload) -> Result<ApprovalTerminal, String> {
    ApprovalTerminal::from_tag(uint(&decode(payload)?)?)
        .ok_or_else(|| "unknown approval settlement kind".into())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProductRestoreNestedCodec;

impl RestoreNestedCodec for ProductRestoreNestedCodec {
    type ApprovalTerm = circular_runtime::EffectTerm;
    type Stamp = circular_core::Stamp<ActorId>;

    fn encode_approval_term(&self, term: &Self::ApprovalTerm) -> Result<Vec<u8>, String> {
        if !matches!(term, circular_runtime::EffectTerm::RequestApproval(_)) {
            return Err("approval restore requires RequestApproval".into());
        }
        circular_runtime::try_encode_term(term).map_err(|error| format!("{error:?}"))
    }
    fn decode_approval_term(&self, bytes: &[u8]) -> Result<Self::ApprovalTerm, String> {
        let term = circular_runtime::decode_term(bytes).map_err(|error| format!("{error:?}"))?;
        if self.encode_approval_term(&term)? != bytes {
            return Err("noncanonical approval restore term".into());
        }
        Ok(term)
    }
    fn encode_stamp(&self, stamp: &Self::Stamp) -> Result<Vec<u8>, String> {
        circular_runtime::encode_effect_stamp(stamp).map_err(|error| error.to_string())
    }
    fn decode_stamp(&self, bytes: &[u8]) -> Result<Self::Stamp, String> {
        circular_runtime::decode_effect_stamp(bytes).map_err(|error| error.to_string())
    }
}
