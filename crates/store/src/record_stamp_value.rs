//! The existing five-coordinate RecordStamp Value used by public record projections.
use circular_core::{Hlc, LogicalCounter, RevisionEpochId, Sequence, Stamp, Tick, Value};
use circular_runtime::ActorId;

pub fn record_stamp_value(stamp: &Stamp<ActorId>) -> Result<Value, String> {
    Ok(Value::array([
        Value::UInt(stamp.hlc().l().get()),
        Value::UInt(stamp.hlc().c().get()),
        crate::record_actor_value(stamp.producer()).map_err(|e| e.to_string())?,
        Value::UInt(stamp.sequence().get()),
        Value::UInt(stamp.revision().get()),
    ]))
}

pub fn record_stamp_from_value(value: &Value) -> Result<Stamp<ActorId>, String> {
    let Some(
        [
            Value::UInt(l),
            Value::UInt(c),
            producer,
            Value::UInt(sequence),
            Value::UInt(revision),
        ],
    ) = value.as_array()
    else {
        return Err("record stamp requires five coordinates".into());
    };
    crate::record_stamp(
        Hlc::new(Tick::new(*l), LogicalCounter::new(*c)),
        crate::record_actor_from_value(producer).map_err(|e| e.to_string())?,
        Sequence::new(*sequence).map_err(|e| e.to_string())?,
        RevisionEpochId::new(*revision).ok_or("zero record stamp revision")?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use crate::{
        ArrivalProjection, ObservationFact, ProductRecordCodec, ProductStore, Record, encode_record,
    };
    use circular_core::{Boundary, Ceilings};
    const DEAD_LETTER: &str = "010000018405000000000000000064000000000000000200220800000002000000056c6f63616c0500000001730000000573636f706507000000000000000000000009000000000000000101000000080000000000000007010400000009040000000000000000000000000000006400000116000108000000050000000764726f7070656407000000050900000000000000050900000000000000010800000002000000056c6f63616c0500000001700000000573636f70650700000000090000000000000003090000000000000001000000066f726967696e0800000002000000056163746f720800000002000000056c6f63616c0500000001730000000573636f7065070000000000000004706f72740100000006726561736f6e080000000200000004636f6465050000001064657374696e6174696f6e5f676f6e650000000664657461696c010000000573636f70650700000000000000077375626a656374080000000200000005736861706507000000010300000000000000010000000576616c756501";

    fn literal(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }
    fn read(hex: &str) -> Record<ProductStore> {
        ArrivalProjection::new().record(&literal(hex)).unwrap()
    }
    #[test]
    fn dead_letter_preserves_recorder_and_dropped_coordinates_separately() {
        let record = read(DEAD_LETTER);
        assert_eq!(record.header().at().sequence().get(), 9);
        assert_eq!(record.header().at().physical_time().get(), 100);
        let Record::Observation(observation) = &record else {
            panic!("observation")
        };
        let ObservationFact::DeadLetter(payload) = observation.fact() else {
            panic!("dead letter")
        };
        let body = circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
            .unwrap();
        let dropped_value = body.as_object().unwrap().get("dropped").unwrap();
        let dropped = crate::record_stamp_from_value(dropped_value).unwrap();
        assert_eq!(dropped.physical_time().get(), 5);
        assert_eq!(dropped.hlc().c().get(), 1);
        assert_eq!(dropped.sequence().get(), 3);
        assert_ne!(dropped.producer(), record.header().at().producer());
        assert_eq!(&crate::record_stamp_value(&dropped).unwrap(), dropped_value);
        assert_eq!(
            encode_record(&record, &ProductRecordCodec).unwrap(),
            literal(DEAD_LETTER)
        );
    }
}
