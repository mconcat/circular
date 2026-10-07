//! Occurrence identities are inline components, never persistent definitions.
//! Only their reusable actors/edges and producer stamp bases belong to a column.
use super::*;
use circular_core::Value;
use circular_runtime::product_identity::{actor_value, edge_value, identity_bytes, identity_value};
use circular_runtime::{EffectId, EffectOccasion};

pub(super) fn encode(
    out: &mut Vec<u8>,
    bytes: &[u8],
    state: &mut State,
    seen: &mut Vec<(u64, [u64; 4])>,
    full: bool,
) -> Result<(), Error> {
    let effect = EffectId::decode(bytes).map_err(|_| invalid())?;
    let actor = identity_bytes(&actor_value(effect.actor()).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    state.put_ref(out, ACTOR, &actor);
    uint(out, effect.generations().len() as u64);
    for generation in effect.generations() {
        uint(out, generation.get());
    }
    match effect.occasion() {
        EffectOccasion::Delivery(edge, at) => {
            out.push(EFFECT_DELIVERY);
            out.push(if edge.is_some() { PRESENT } else { ABSENT });
            if let Some(edge) = edge {
                let edge = identity_bytes(&edge_value(edge).map_err(|_| invalid())?)
                    .map_err(|_| invalid())?;
                state.put_ref(out, EDGE, &edge);
            }
            let bytes = circular_runtime::encode_effect_stamp(at).map_err(|_| invalid())?;
            let at = StampValue::raw(&mut Reader::new(&bytes))?;
            stamp(out, &at, state, seen, full);
        }
        EffectOccasion::Poll(tick, index) => {
            out.push(EFFECT_POLL);
            uint(out, tick.get());
            uint(out, *index);
        }
    }
    uint(out, effect.index());
    Ok(())
}

pub(super) fn decode(
    r: &mut Reader<'_>,
    state: &State,
    seen: &mut Vec<(u64, [u64; 4])>,
    full: bool,
) -> Result<Vec<u8>, Error> {
    let actor = identity_value(&state.get_ref(r, ACTOR)?).map_err(|_| invalid())?;
    let mut generations = Vec::new();
    for _ in 0..r.uint()? {
        generations.push(Value::UInt(r.uint()?));
    }
    let occasion = match r.byte()? {
        EFFECT_DELIVERY => {
            let edge = match r.byte()? {
                ABSENT => Value::Null,
                PRESENT => identity_value(&state.get_ref(r, EDGE)?).map_err(|_| invalid())?,
                _ => return Err(invalid()),
            };
            let mut at = Vec::new();
            read_stamp(r, state, seen, full)?.put_raw(&mut at)?;
            Value::array([Value::UInt(1), edge, Value::bytes(at)])
        }
        EFFECT_POLL => Value::array([
            Value::UInt(3),
            Value::UInt(r.uint()?),
            Value::UInt(r.uint()?),
        ]),
        _ => return Err(invalid()),
    };
    let bytes = identity_bytes(&Value::array([
        actor,
        Value::array(generations),
        occasion,
        Value::UInt(r.uint()?),
    ]))
    .map_err(|_| invalid())?;
    EffectId::decode(&bytes).map_err(|_| invalid())?;
    Ok(bytes)
}
