use super::*;
use crate::{Class, EnvelopeOrigin};
use bytes::fixed_field;

/// Observation and display names are stored as their catalog tag; the catalog
/// alone decides which tags exist, so retired and unknown tags reject.
fn observation_name(tag: u8) -> Result<u8, Error> {
    circular_core::BuiltinObservationName::from_tag(tag)
        .map(circular_core::BuiltinObservationName::tag)
        .ok_or_else(invalid)
}

#[derive(Clone, Debug)]
pub(super) struct Parts {
    version: u16,
    class: u8,
    fact: u8,
    key_tag: u8,
    observation: Option<u8>,
    position: StampValue,
    attribution: Option<u64>,
    key: Vec<u8>,
    payload: Vec<u8>,
    index: Option<u64>,
    observed: Option<u64>,
    bucket: Option<u64>,
}
impl Parts {
    pub fn parse(payload: &circular_core::EncodedPayload) -> Result<Self, Error> {
        let env = crate::decode_envelope(payload.body()).map_err(|error| match error {
            crate::RecordCodecError::UnknownVersion(found) => Error::UnsupportedRecordFormat {
                vocabulary: "record_version",
                found: found.into(),
            },
            _ => invalid(),
        })?;
        let position = match env.origin {
            EnvelopeOrigin::Stamped {
                l,
                c,
                producer,
                sequence,
                revision,
            } => StampValue {
                producer: producer.to_vec(),
                numbers: [revision.get(), l, c, sequence],
            },
            EnvelopeOrigin::OperationCoordinate { .. } => return Err(invalid()),
        };
        let (class, fact) = match env.class {
            Class::Boundary => (
                BOUNDARY,
                match env.class_key_tag {
                    1 => ARRIVAL,
                    3 => RESERVATION,
                    4 => ADMISSION,
                    5 => EMISSION,
                    _ => return Err(invalid()),
                },
            ),
            Class::Structure => (
                STRUCTURE,
                match env.class_key_tag {
                    1 => MANIFEST,
                    2 => REVISION,
                    _ => return Err(invalid()),
                },
            ),
            Class::Display => (DISPLAY, DISPLAY_ITEM),
            Class::Observation => (
                OBSERVATION,
                match env.observation_fact_tag {
                    Some(1) => LIFECYCLE,
                    Some(2) => DIAGNOSTIC,
                    Some(3) => ACCOUNTING,
                    Some(4) => DEAD_LETTER,
                    Some(6) => REPLAY,
                    Some(7) => RESTART,
                    Some(8) => CHECKPOINT_FACT,
                    _ => return Err(invalid()),
                },
            ),
        };
        Ok(Self {
            version: u16::from_be_bytes(payload.as_bytes()[..2].try_into().unwrap()),
            class,
            fact,
            key_tag: env.class_key_tag,
            observation: env.observation_fact_tag,
            position,
            attribution: match env.attribution_tag {
                1 => Some(u64::from_be_bytes(
                    env.attribution_body.try_into().map_err(|_| invalid())?,
                )),
                2 if env.attribution_body.is_empty() => None,
                _ => return Err(invalid()),
            },
            key: env.class_key_body.to_vec(),
            payload: env.payload.to_vec(),
            index: env.arrival_index.map(|n| n.get()),
            observed: env.observed_at.map(|n| n.millis()),
            bucket: env.observation_bucket.map(|n| n.millis()),
        })
    }
    fn arrival(&self) -> bool {
        self.class == BOUNDARY && [ARRIVAL, ADMISSION].contains(&self.fact)
    }
    pub fn owner(&self) -> Result<&[u8], Error> {
        if self.arrival() {
            let mut r = Reader::new(&self.key);
            return r.fixed_field();
        }
        Ok(&self.position.producer)
    }
    fn flags(&self, ok: bool) -> u64 {
        BASE | if self.attribution.is_some() {
            ATTRIBUTED
        } else {
            0
        } | if self.index.is_some() { INDEX } else { 0 }
            | if self.observed.is_some() { OBSERVED } else { 0 }
            | if self.bucket.is_some() { BUCKET } else { 0 }
            | if ok { OK } else { 0 }
    }
    fn raw(&self) -> Result<circular_core::EncodedPayload, Error> {
        let cls = match self.class {
            BOUNDARY => 1,
            STRUCTURE => 2,
            DISPLAY => 4,
            OBSERVATION => 5,
            _ => return Err(invalid()),
        };
        let mut out = vec![cls];
        let s = &self.position;
        out.push(0);
        out.extend_from_slice(&s.numbers[1].to_be_bytes());
        out.extend_from_slice(&s.numbers[2].to_be_bytes());
        out.extend_from_slice(
            &u16::try_from(s.producer.len())
                .map_err(|_| invalid())?
                .to_be_bytes(),
        );
        out.extend_from_slice(&s.producer);
        out.extend_from_slice(&s.numbers[3].to_be_bytes());
        out.extend_from_slice(&s.numbers[0].to_be_bytes());
        match self.attribution {
            Some(n) => {
                out.push(1);
                fixed_field(&mut out, &n.to_be_bytes())?;
            }
            None => {
                out.push(2);
                fixed_field(&mut out, &[])?;
            }
        }
        out.push(self.key_tag);
        if self.arrival() {
            let mut r = Reader::new(&self.key);
            r.fixed_field()?;
            out.push(r.byte()?);
        }
        if let Some(n) = self.observation {
            out.push(n);
        }
        fixed_field(&mut out, &self.key)?;
        for n in [self.bucket, self.index, self.observed]
            .into_iter()
            .flatten()
        {
            out.extend_from_slice(&n.to_be_bytes());
        }
        fixed_field(&mut out, &self.payload)?;
        let mut record = vec![crate::RECORD_VERSION];
        fixed_field(&mut record, &out)?;
        Ok(circular_core::EncodedPayload::new(
            circular_core::PayloadVersionTag::new(self.version).map_err(|_| invalid())?,
            &record,
        ))
    }
}

pub(super) fn encode(
    parts: &Parts,
    state: &mut State,
    full: bool,
) -> Result<(u64, Vec<u8>, Vec<u8>), Error> {
    if parts.owner()? != state.owner {
        return Err(Error::UnencodableIdentity("column owner"));
    }
    let mut meta = Vec::new();
    let mut body = Vec::new();
    let mut seen = Vec::new();
    uint(&mut meta, parts.version.into());
    stamp(&mut meta, &parts.position, state, &mut seen, full);
    if let Some(n) = parts.attribution {
        uint(&mut meta, n);
    }
    let mut key = Reader::new(&parts.key);
    let mut ok = false;
    if parts.arrival() {
        key.fixed_field()?;
        let origin = key.byte()?;
        meta.push(match origin {
            1 => FROM_EDGE,
            2 => FROM_TIMER,
            3 => FROM_EFFECT,
            4 => FROM_EXTERNAL,
            _ => return Err(invalid()),
        });
        match origin {
            1 => {
                state.put_ref(&mut meta, EDGE, key.fixed_field()?);
                let s = StampValue::raw(&mut key)?;
                stamp(&mut meta, &s, state, &mut seen, full);
            }
            2 | 3 => effect::encode(&mut meta, key.fixed_field()?, state, &mut seen, full)?,
            4 => {
                let value = key.fixed_field()?;
                let prefix = if full {
                    0
                } else {
                    state.external.get(&FROM_EXTERNAL).map_or(0, |b| {
                        b.iter().zip(value).take_while(|(a, b)| a == b).count()
                    })
                };
                uint(&mut meta, prefix as u64);
                field(&mut meta, &value[prefix..]);
                state.external.insert(FROM_EXTERNAL, value.to_vec());
                match key.byte()? {
                    0 => meta.push(ABSENT),
                    1 => {
                        meta.push(PRESENT);
                        state.put_ref(&mut meta, EDGE, key.fixed_field()?);
                    }
                    _ => return Err(invalid()),
                }
            }
            _ => unreachable!(),
        }
        if let Some(index) = parts.index {
            if !full && state.arrival.and_then(|n| n.checked_add(1)) == Some(index) {
                meta.push(NEXT);
            } else {
                meta.push(EXPLICIT);
                number(&mut meta, index, if full { None } else { state.arrival });
            }
            state.arrival = Some(index);
        }
        if let Some(time) = parts.observed {
            number(&mut meta, time, if full { None } else { state.observed });
            state.observed = Some(time);
        }
        let mut p = Reader::new(&parts.payload);
        let body_kind = p.byte()?;
        body.push(match body_kind {
            1 => OWNED,
            2 => EFFECT_BODY,
            4 => EMITTED,
            3 => {
                return Err(Error::UnsupportedRecordFormat {
                    vocabulary: "arrival_body",
                    found: 3,
                });
            }
            _ => return Err(invalid()),
        });
        if body_kind == 4 {
            state.put_ref(&mut body, ACTOR, p.fixed_field()?);
            uint(&mut body, p.u64()?);
        } else {
            field(&mut body, p.fixed_field()?);
        }
        let parents = p.u32()?;
        uint(&mut meta, parents.into());
        for _ in 0..parents {
            uint(&mut meta, p.u64()?);
            let s = StampValue::raw(&mut p)?;
            stamp(&mut meta, &s, state, &mut seen, full);
        }
        if body_kind == 2 {
            field(&mut body, p.fixed_field()?);
            field(&mut body, p.fixed_field()?);
        }
        let port = p.fixed_field()?;
        if body_kind != 2 {
            state.put_ref(&mut meta, PORT, port);
        } else if !port.is_empty() {
            return Err(invalid());
        }
        if body_kind != 2 {
            let result = p.fixed_field()?;
            ok =
                crate::arrival_result::decode(result) == Some(circular_runtime::EnvelopeResult::Ok);
            if !ok {
                field(&mut meta, result);
            }
        }
        p.done()?;
    } else {
        match (parts.class, parts.fact) {
            (BOUNDARY, RESERVATION) => {
                effect::encode(&mut meta, key.rest(), state, &mut seen, full)?
            }
            (BOUNDARY, EMISSION) | (STRUCTURE, MANIFEST) => {}
            (STRUCTURE, REVISION) => state.put_ref(&mut meta, SCOPE, key.rest()),
            (DISPLAY, DISPLAY_ITEM) => {
                meta.push(observation_name(key.byte()?)?);
                state.put_ref(&mut meta, ACTOR, key.fixed_field()?);
                match key.byte()? {
                    0 => meta.push(ABSENT),
                    1 => {
                        meta.push(PRESENT);
                        uint(&mut meta, key.u64()?);
                    }
                    _ => return Err(invalid()),
                }
            }
            (OBSERVATION, _) => {
                meta.push(match parts.key_tag {
                    1 => OBS_STREAM,
                    2 => OBS_GLOBAL,
                    3 => OBS_CHECKPOINT,
                    _ => return Err(invalid()),
                });
                meta.push(observation_name(key.byte()?)?);
                uint(&mut meta, key.u64()?);
            }
            _ => return Err(invalid()),
        }
        if let Some(bucket) = parts.bucket {
            uint(&mut meta, bucket);
        }
        body.extend_from_slice(&parts.payload);
    }
    key.done()?;
    for (producer, numbers) in seen {
        state.stamps.insert(producer, numbers);
    }
    Ok((parts.flags(ok), meta, body))
}

pub(super) fn decode(
    class: u8,
    fact: u8,
    flags: u64,
    r: &mut Reader<'_>,
    body: &[u8],
    state: &mut State,
    full: bool,
) -> Result<Parts, Error> {
    if flags & BASE == 0 || flags & !KNOWN_FLAGS != 0 {
        return Err(invalid());
    }
    let version = u16::try_from(r.uint()?).map_err(|_| invalid())?;
    let mut seen = Vec::new();
    let position = read_stamp(r, state, &mut seen, full)?;
    let attribution = if flags & ATTRIBUTED != 0 {
        Some(r.uint()?)
    } else {
        None
    };
    let mut key = Vec::new();
    let mut payload = Vec::new();
    let mut index = None;
    let mut observed = None;
    let mut bucket = None;
    let mut observation = None;
    let arrival = class == BOUNDARY && [ARRIVAL, ADMISSION].contains(&fact);
    let key_tag = match (class, fact) {
        (BOUNDARY, ARRIVAL) => 1,
        (BOUNDARY, RESERVATION) => 3,
        (BOUNDARY, ADMISSION) => 4,
        (BOUNDARY, EMISSION) => 5,
        (STRUCTURE, MANIFEST) => 1,
        (STRUCTURE, REVISION) => 2,
        (DISPLAY, DISPLAY_ITEM) => 1,
        (OBSERVATION, _) => {
            observation = Some(match fact {
                LIFECYCLE => 1,
                DIAGNOSTIC => 2,
                ACCOUNTING => 3,
                DEAD_LETTER => 4,
                REPLAY => 6,
                RESTART => 7,
                CHECKPOINT_FACT => 8,
                _ => return Err(invalid()),
            });
            0
        }
        _ => return Err(invalid()),
    };
    let mut key_tag = key_tag;
    if arrival {
        fixed_field(&mut key, &state.owner)?;
        let origin = match r.byte()? {
            FROM_EDGE => 1,
            FROM_TIMER => 2,
            FROM_EFFECT => 3,
            FROM_EXTERNAL => 4,
            _ => return Err(invalid()),
        };
        key.push(origin);
        match origin {
            1 => {
                fixed_field(&mut key, &state.get_ref(r, EDGE)?)?;
                read_stamp(r, state, &mut seen, full)?.put_raw(&mut key)?;
            }
            2 | 3 => fixed_field(&mut key, &effect::decode(r, state, &mut seen, full)?)?,
            4 => {
                let prefix = usize::try_from(r.uint()?).map_err(|_| invalid())?;
                let mut value = if prefix == 0 {
                    Vec::new()
                } else {
                    if full {
                        return Err(invalid());
                    }
                    state
                        .external
                        .get(&FROM_EXTERNAL)
                        .and_then(|b| b.get(..prefix))
                        .ok_or_else(invalid)?
                        .to_vec()
                };
                value.extend_from_slice(r.field()?);
                fixed_field(&mut key, &value)?;
                state.external.insert(FROM_EXTERNAL, value);
                match r.byte()? {
                    ABSENT => key.push(0),
                    PRESENT => {
                        key.push(1);
                        fixed_field(&mut key, &state.get_ref(r, EDGE)?)?;
                    }
                    _ => return Err(invalid()),
                }
            }
            _ => unreachable!(),
        }
        if flags & INDEX != 0 {
            let n = match r.byte()? {
                NEXT if !full => state
                    .arrival
                    .and_then(|v| v.checked_add(1))
                    .ok_or_else(invalid)?,
                EXPLICIT => read_number(r, if full { None } else { state.arrival })?,
                _ => return Err(invalid()),
            };
            state.arrival = Some(n);
            index = Some(n);
        }
        if flags & OBSERVED != 0 {
            let n = read_number(r, if full { None } else { state.observed })?;
            state.observed = Some(n);
            observed = Some(n);
        }
        let mut b = Reader::new(body);
        let body_kind = match b.byte()? {
            OWNED => 1,
            EFFECT_BODY => 2,
            EMITTED => 4,
            _ => return Err(invalid()),
        };
        payload.push(body_kind);
        if body_kind == 4 {
            fixed_field(&mut payload, &state.get_ref(&mut b, ACTOR)?)?;
            payload.extend_from_slice(&b.uint()?.to_be_bytes());
        } else {
            fixed_field(&mut payload, b.field()?)?;
        }
        let count = u32::try_from(r.uint()?).map_err(|_| invalid())?;
        payload.extend_from_slice(&count.to_be_bytes());
        for _ in 0..count {
            payload.extend_from_slice(&r.uint()?.to_be_bytes());
            read_stamp(r, state, &mut seen, full)?.put_raw(&mut payload)?;
        }
        if body_kind == 2 {
            fixed_field(&mut payload, b.field()?)?;
            fixed_field(&mut payload, b.field()?)?;
            fixed_field(&mut payload, &[])?;
        } else {
            fixed_field(&mut payload, &state.get_ref(r, PORT)?)?;
            if flags & OK != 0 {
                fixed_field(
                    &mut payload,
                    &crate::arrival_result::encode(&circular_runtime::EnvelopeResult::Ok)
                        .map_err(|_| invalid())?,
                )?;
            } else {
                fixed_field(&mut payload, r.field()?)?;
            }
        }
        b.done()?;
    } else {
        match (class, fact) {
            (BOUNDARY, RESERVATION) => key = effect::decode(r, state, &mut seen, full)?,
            (BOUNDARY, EMISSION) | (STRUCTURE, MANIFEST) => {}
            (STRUCTURE, REVISION) => key = state.get_ref(r, SCOPE)?,
            (DISPLAY, DISPLAY_ITEM) => {
                key.push(observation_name(r.byte()?)?);
                fixed_field(&mut key, &state.get_ref(r, ACTOR)?)?;
                match r.byte()? {
                    ABSENT => key.push(0),
                    PRESENT => {
                        key.push(1);
                        key.extend_from_slice(&r.uint()?.to_be_bytes());
                    }
                    _ => return Err(invalid()),
                }
            }
            (OBSERVATION, _) => {
                key_tag = match r.byte()? {
                    OBS_STREAM => 1,
                    OBS_GLOBAL => 2,
                    OBS_CHECKPOINT => 3,
                    _ => return Err(invalid()),
                };
                key.push(observation_name(r.byte()?)?);
                key.extend_from_slice(&r.uint()?.to_be_bytes());
            }
            _ => return Err(invalid()),
        }
        if flags & BUCKET != 0 {
            bucket = Some(r.uint()?);
        }
        payload.extend_from_slice(body);
    }
    for (producer, numbers) in seen {
        state.stamps.insert(producer, numbers);
    }
    let parts = Parts {
        version,
        class,
        fact,
        key_tag,
        observation,
        position,
        attribution,
        key,
        payload,
        index,
        observed,
        bucket,
    };
    if parts.owner()? != state.owner {
        return Err(invalid());
    }
    Ok(parts)
}

pub(super) fn encode_frame(
    parts: &Parts,
    state: &mut State,
    keyframe: bool,
    address: Address,
) -> Result<Vec<u8>, Error> {
    let before = state.clone();
    let old_count = state.definitions.len();
    let (flags, envelope, body) = encode(parts, state, keyframe)?;
    let mut metadata = Vec::new();
    definitions(
        &mut metadata,
        if keyframe {
            &state.definitions
        } else {
            &state.definitions[old_count..]
        },
    );
    if keyframe {
        bases(&mut metadata, &before);
    }
    metadata.extend_from_slice(&envelope);
    let mut item = vec![if keyframe { KEYFRAME } else { NORMAL }];
    address.write(&mut item);
    if keyframe {
        field(&mut item, &state.owner);
    }
    item.push(parts.class);
    item.push(parts.fact);
    uint(&mut item, flags);
    field(&mut item, &metadata);
    field(&mut item, &body);
    let mut out = Vec::new();
    field(&mut out, &item);
    state.since_keyframe = if keyframe {
        1
    } else {
        state.since_keyframe.checked_add(1).ok_or_else(invalid)?
    };
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Address {
    Current,
    Committed(u64, u32),
    Local(u32),
}
impl Address {
    fn write(self, out: &mut Vec<u8>) {
        match self {
            Self::Current => out.push(CURRENT),
            Self::Committed(c, o) => {
                out.push(COMMITTED);
                uint(out, c);
                uint(out, o.into());
            }
            Self::Local(o) => {
                out.push(LOCAL);
                uint(out, o.into());
            }
        }
    }
    fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(match r.byte()? {
            CURRENT => Self::Current,
            COMMITTED => {
                let c = r.uint()?;
                if c == 0 {
                    return Err(invalid());
                }
                Self::Committed(c, u32::try_from(r.uint()?).map_err(|_| invalid())?)
            }
            LOCAL => Self::Local(u32::try_from(r.uint()?).map_err(|_| invalid())?),
            _ => return Err(invalid()),
        })
    }
    pub fn position(self, at: (u64, u32)) -> Result<(u64, u32), Error> {
        match self {
            Self::Current => Ok(at),
            Self::Committed(c, o) if c < at.0 => Ok((c, o)),
            Self::Local(o) if o < at.1 => Ok((at.0, o)),
            _ => Err(invalid()),
        }
    }
}
pub(super) struct Frame<'a> {
    pub full: bool,
    pub address: Address,
    pub owner: Option<&'a [u8]>,
    class: u8,
    fact: u8,
    flags: u64,
    metadata: &'a [u8],
    body: &'a [u8],
    bytes: &'a [u8],
}
impl<'a> Frame<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let mut outer = Reader::new(bytes);
        let mut r = Reader::new(outer.field()?);
        outer.done()?;
        let full = match r.byte()? {
            NORMAL => false,
            KEYFRAME => true,
            _ => return Err(invalid()),
        };
        let address = Address::read(&mut r)?;
        if !full && address == Address::Current {
            return Err(invalid());
        }
        let owner = if full { Some(r.field()?) } else { None };
        let class = r.byte()?;
        let fact = r.byte()?;
        let flags = r.uint()?;
        let metadata = r.field()?;
        let body = r.field()?;
        r.done()?;
        Ok(Self {
            full,
            address,
            owner,
            class,
            fact,
            flags,
            metadata,
            body,
            bytes,
        })
    }
    pub fn decode(
        &self,
        state: &mut State,
        at: (u64, u32),
    ) -> Result<circular_core::EncodedPayload, Error> {
        let mut metadata = Reader::new(self.metadata);
        let mut next = if self.full {
            let owner = self.owner.ok_or_else(invalid)?;
            let mut next = State {
                owner: owner.to_vec(),
                address: Some(self.address.position(at)?),
                ..State::default()
            };
            read_definitions(&mut metadata, &mut next)?;
            read_bases(&mut metadata, &mut next)?;
            if self.address != Address::Current
                && state.address.is_some()
                && (state.owner != next.owner
                    || !next.definitions.starts_with(&state.definitions)
                    || next.stamps != state.stamps
                    || next.arrival != state.arrival
                    || next.observed != state.observed
                    || next.external != state.external)
            {
                return Err(invalid());
            }
            next
        } else {
            if state.address != Some(self.address.position(at)?) {
                return Err(invalid());
            }
            let mut next = state.clone();
            read_definitions(&mut metadata, &mut next)?;
            next
        };
        let mut canonical_before = next.clone();
        if !self.full {
            canonical_before.definitions = state.definitions.clone();
        }
        let parts = decode(
            self.class,
            self.fact,
            self.flags,
            &mut metadata,
            self.body,
            &mut next,
            self.full,
        )?;
        metadata.done()?;
        let rebuilt = encode_frame(&parts, &mut canonical_before, self.full, self.address)?;
        if rebuilt != self.bytes {
            return Err(invalid());
        }
        let raw = parts.raw()?;
        crate::decode_envelope(raw.body()).map_err(|_| invalid())?;
        next.since_keyframe = if self.full {
            1
        } else {
            state.since_keyframe.checked_add(1).ok_or_else(invalid)?
        };
        *state = next;
        Ok(raw)
    }
}
