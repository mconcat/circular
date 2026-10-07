use crate::ProductStore;
use circular_core::{
    Boundary, BuiltinObservationName, Ceilings, EncodedPayload, PayloadVersionTag, Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductStreamStartBody {
    pub boot_id: [u8; 16],
    pub wall_millis: u64,
}

impl ProductStreamStartBody {
    pub fn encode(&self) -> Result<EncodedPayload, String> {
        let value = Value::array([Value::bytes(self.boot_id), Value::UInt(self.wall_millis)]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|error| error.to_string())?;
        Ok(EncodedPayload::new(PayloadVersionTag::FIRST, &bytes))
    }

    pub fn decode(payload: &EncodedPayload) -> Result<Self, String> {
        if payload.version_tag() != PayloadVersionTag::FIRST {
            return Err("unsupported stream start body version".into());
        }
        let value =
            circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
                .map_err(|error| error.to_string())?;
        let Some([Value::Bytes(boot), Value::UInt(wall_millis)]) = value.as_array() else {
            return Err("stream start requires [boot Bytes16, wall UInt]".into());
        };
        Ok(Self {
            boot_id: boot
                .as_slice()
                .try_into()
                .map_err(|_| "stream start boot id must be 16 bytes")?,
            wall_millis: *wall_millis,
        })
    }
}

pub fn stream_start_record(
    at: circular_core::Stamp<circular_runtime::ActorId>,
    body: &ProductStreamStartBody,
) -> Result<crate::Record<ProductStore>, String> {
    if !matches!(
        at.producer(),
        circular_runtime::ActorId::System(circular_runtime::SystemActor::Pipeline)
    ) {
        return Err("stream start is a System(Pipeline) record".into());
    }
    let identity = crate::OpaqueId::new(at.sequence().get());
    Ok(crate::Record::Observation(
        crate::ObservationRecord::lifecycle(
            at,
            crate::ObservationBucket::from_millis(body.wall_millis),
            crate::RecordOrigin::Stream,
            crate::ObservationItemKey::new(BuiltinObservationName::StreamStart, identity),
            body.encode()?,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_start_body_independent_golden_and_all_truncations() {
        let mut bytes = vec![7, 0, 0, 0, 2, 6, 0, 0, 0, 16];
        bytes.extend_from_slice(&[0x42; 16]);
        bytes.extend_from_slice(&[9, 0, 0, 0, 0, 0, 0, 0, 9]);
        let body = ProductStreamStartBody {
            boot_id: [0x42; 16],
            wall_millis: 9,
        };
        let encoded = EncodedPayload::new(PayloadVersionTag::FIRST, &bytes);
        assert_eq!(body.encode().unwrap(), encoded);
        assert_eq!(ProductStreamStartBody::decode(&encoded).unwrap(), body);
        for end in 0..bytes.len() {
            assert!(
                ProductStreamStartBody::decode(&EncodedPayload::new(
                    PayloadVersionTag::FIRST,
                    &bytes[..end]
                ))
                .is_err()
            );
        }
        assert!(
            ProductStreamStartBody::decode(&EncodedPayload::new(
                PayloadVersionTag::new(2).unwrap(),
                &bytes
            ))
            .is_err()
        );
        for value in [
            Value::array([Value::bytes([0; 15]), Value::UInt(9)]),
            Value::array([Value::bytes([0; 16]), Value::Int(9)]),
            Value::array([Value::bytes([0; 16])]),
            Value::array([Value::bytes([0; 16]), Value::UInt(9), Value::Null]),
        ] {
            let bytes =
                circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal)).unwrap();
            assert!(
                ProductStreamStartBody::decode(&EncodedPayload::new(
                    PayloadVersionTag::FIRST,
                    &bytes
                ))
                .is_err()
            );
        }
    }
}
