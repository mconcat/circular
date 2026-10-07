use crate::product_identity::{
    ProductIdentityError, actor_parts_from_value, actor_value, edge_from_value, edge_value,
    identity_bytes, identity_value,
};
use crate::{EffectId, EffectOccasion};
use circular_core::{Hlc, LogicalCounter, RevisionEpochId, Sequence, Stamp, Tick, Value};
use circular_plan::{ActorId, Generation, NamedActorId};

type Result<T> = std::result::Result<T, ProductIdentityError>;
fn malformed() -> ProductIdentityError {
    ProductIdentityError::Malformed("effect identity")
}
fn uint(v: &Value) -> Result<u64> {
    if let Value::UInt(n) = v {
        Ok(*n)
    } else {
        Err(malformed())
    }
}

pub fn encode_effect_stamp(stamp: &Stamp<ActorId>) -> Result<Vec<u8>> {
    let producer = identity_bytes(&crate::product_identity::record_actor_value(
        stamp.producer(),
    )?)?;
    let mut out = Vec::new();
    out.extend_from_slice(&stamp.hlc().l().get().to_be_bytes());
    out.extend_from_slice(&stamp.hlc().c().get().to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(producer.len())
            .map_err(|_| malformed())?
            .to_be_bytes(),
    );
    out.extend_from_slice(&producer);
    out.extend_from_slice(&stamp.sequence().get().to_be_bytes());
    out.extend_from_slice(&stamp.revision().get().to_be_bytes());
    Ok(out)
}

pub fn decode_effect_stamp(bytes: &[u8]) -> Result<Stamp<ActorId>> {
    let read = |at: usize| -> Result<u64> {
        Ok(u64::from_be_bytes(
            bytes
                .get(at..at + 8)
                .ok_or_else(malformed)?
                .try_into()
                .map_err(|_| malformed())?,
        ))
    };
    let l = read(0)?;
    let c = read(8)?;
    let len = u32::from_be_bytes(
        bytes
            .get(16..20)
            .ok_or_else(malformed)?
            .try_into()
            .map_err(|_| malformed())?,
    ) as usize;
    let end = 20usize.checked_add(len).ok_or_else(malformed)?;
    if bytes.len() != end.checked_add(16).ok_or_else(malformed)? {
        return Err(malformed());
    }
    let producer = crate::product_identity::record_actor_from_value(&identity_value(
        bytes.get(20..end).ok_or_else(malformed)?,
    )?)?;
    let stamp = crate::product_identity::record_stamp(
        Hlc::new(Tick::new(l), LogicalCounter::new(c)),
        producer,
        Sequence::new(read(end)?).map_err(|_| malformed())?,
        RevisionEpochId::new(read(end + 8)?).ok_or_else(malformed)?,
    )?;
    if encode_effect_stamp(&stamp)? != bytes {
        return Err(malformed());
    }
    Ok(stamp)
}

impl EffectId {
    /// Canonical bytes: `[actor, generations, occasion, index]`.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let occasion = match self.occasion() {
            EffectOccasion::Delivery(edge, stamp) => Value::array([
                Value::UInt(1),
                edge.as_ref().map_or(Ok(Value::Null), edge_value)?,
                Value::bytes(encode_effect_stamp(stamp)?),
            ]),
            EffectOccasion::Poll(tick, index) => {
                Value::array([Value::UInt(3), Value::UInt(tick.get()), Value::UInt(*index)])
            }
        };
        identity_bytes(&Value::array([
            actor_value(self.actor())?,
            Value::array(self.generations().iter().map(|g| Value::UInt(g.get()))),
            occasion,
            Value::UInt(self.index()),
        ]))
    }

    /// The inverse of [`EffectId::encode`]; any other byte string is rejected,
    /// including the retired tick-wake occasion.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = identity_value(bytes)?;
        let [actor, generations, occasion, index] = value.as_array().ok_or_else(malformed)? else {
            return Err(malformed());
        };
        let (scope, name) = actor_parts_from_value(actor)?;
        let generations = generations
            .as_array()
            .ok_or_else(malformed)?
            .iter()
            .map(|g| uint(g).map(Generation::new))
            .collect::<Result<Vec<_>>>()?;
        let occasion = match occasion.as_array().ok_or_else(malformed)? {
            [Value::UInt(1), edge, stamp] => EffectOccasion::Delivery(
                match edge {
                    Value::Null => None,
                    edge => Some(edge_from_value(edge)?),
                },
                decode_effect_stamp(stamp.as_bytes().ok_or_else(malformed)?)?,
            ),
            [Value::UInt(3), tick, index] => {
                EffectOccasion::Poll(Tick::new(uint(tick)?), uint(index)?)
            }
            _ => return Err(malformed()),
        };
        let key = Self::from_components(
            NamedActorId::new(scope, name).into(),
            generations.into_boxed_slice(),
            occasion,
            uint(index)?,
        )
        .map_err(|_| malformed())?;
        if key.encode()? != bytes {
            return Err(malformed());
        }
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::{Name, ScopeId};
    const ARRIVAL: &str = concat!(
        "0700000004",
        "0800000002000000056c6f63616c0500000001730000000573636f70650700000000",
        "0700000001090000000000000002",
        "0700000003090000000000000001",
        "01",
        "0600000046",
        "00000000000000050000000000000002000000220800000002000000056c6f63616c0500000001730000000573636f7065070000000000000000000000070000000000000001",
        "090000000000000003",
    );
    const RETIRED_WAKE: &str = "07000000040800000002000000056c6f63616c0500000001730000000573636f7065070000000007000000010900000000000000020700000002090000000000000002090000000000000005090000000000000003";
    const POLL: &str = "07000000040800000002000000056c6f63616c0500000001730000000573636f7065070000000007000000010900000000000000020700000003090000000000000003090000000000000005090000000000000001090000000000000003";
    const DELIVERY: &str = "07000000040800000002000000056c6f63616c0500000001730000000573636f706507000000000700000001090000000000000002070000000309000000000000000107000000020300000000000000020800000002000000056c6f63616c0500000001730000000573636f70650700000000060000004600000000000000050000000000000002000000220800000002000000056c6f63616c0500000001730000000573636f7065070000000000000000000000070000000000000001090000000000000003";
    const STAMP: &str = "00000000000000050000000000000002000000220800000002000000056c6f63616c0500000001730000000573636f7065070000000000000000000000070000000000000001";
    fn literal(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn two_occasions_match_independent_golden_bytes() {
        let actor = NamedActorId::new(ScopeId::root(), Name::from_normalized("s"));
        let stamp = Stamp::from_event_producer_at(
            Hlc::new(Tick::new(5), LogicalCounter::new(2)),
            actor.clone(),
            Sequence::new(7).unwrap(),
            RevisionEpochId::new(1).unwrap(),
        );
        for (occasion, bytes) in [
            (EffectOccasion::Delivery(None, stamp.clone()), ARRIVAL),
            (EffectOccasion::Poll(Tick::new(5), 1), POLL),
            (
                EffectOccasion::Delivery(
                    Some(crate::EdgeId::outcome(actor.clone())),
                    stamp.clone(),
                ),
                DELIVERY,
            ),
        ] {
            let key = EffectId::from_components(
                actor.clone().into(),
                vec![Generation::new(2)].into_boxed_slice(),
                occasion,
                3,
            )
            .unwrap();
            assert_eq!(key.encode().unwrap(), literal(bytes));
            assert_eq!(EffectId::decode(&literal(bytes)).unwrap(), key);
        }
        assert_eq!(encode_effect_stamp(&stamp).unwrap(), literal(STAMP));
        assert_eq!(decode_effect_stamp(&literal(STAMP)).unwrap(), stamp);
    }

    #[test]
    fn old_numeric_keys_truncation_trailing_and_unknown_occasion_are_rejected() {
        assert!(EffectId::decode(&7u64.to_be_bytes()).is_err());
        assert!(EffectId::decode(&literal(RETIRED_WAKE)).is_err());
        for text in [ARRIVAL, POLL, DELIVERY] {
            let bytes = literal(text);
            for end in 0..bytes.len() {
                assert!(EffectId::decode(&bytes[..end]).is_err());
            }
            let mut trailing = bytes;
            trailing.push(0);
            assert!(EffectId::decode(&trailing).is_err());
        }
        let mut value = identity_value(&literal(ARRIVAL)).unwrap();
        let Value::Array(ref mut fields) = value else {
            unreachable!()
        };
        fields[2] = Value::array([Value::UInt(4), Value::UInt(5)]);
        assert!(EffectId::decode(&identity_bytes(&value).unwrap()).is_err());
    }
}
