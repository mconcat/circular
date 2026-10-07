use circular_core::{
    Boundary, Ceilings, EncodedPayload, PayloadVersionTag, RevisionEpochId, Value,
};
use circular_runtime::{EffectId, ScheduleCorrelation};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductScheduleReservation {
    pub effect: EffectId,
    pub birth: RevisionEpochId,
    pub correlation: ScheduleCorrelation,
    pub deadline_unix_millis: u64,
}
impl ProductScheduleReservation {
    pub fn encode(&self) -> Result<EncodedPayload, String> {
        let effect = circular_core::decode(
            &circular_runtime::EffectId::encode(&self.effect).map_err(|e| e.to_string())?,
            Ceilings::for_boundary(Boundary::Identity),
        )
        .map_err(|e| e.to_string())?;
        let bytes = circular_core::encode(
            &Value::array([
                effect,
                Value::UInt(self.birth.get()),
                Value::UInt(self.correlation.get()),
                Value::UInt(self.deadline_unix_millis),
            ]),
            Ceilings::for_boundary(Boundary::Journal),
        )
        .map_err(|e| e.to_string())?;
        Ok(EncodedPayload::new(PayloadVersionTag::FIRST, &bytes))
    }
    pub fn decode(payload: &EncodedPayload) -> Result<Self, String> {
        if payload.version_tag() != PayloadVersionTag::FIRST {
            return Err("unsupported schedule reservation version".into());
        }
        let value =
            circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
                .map_err(|e| e.to_string())?;
        let Some(
            [
                effect,
                Value::UInt(birth),
                Value::UInt(correlation),
                Value::UInt(deadline),
            ],
        ) = value.as_array()
        else {
            return Err("schedule reservation requires four canonical fields".into());
        };
        let effect = circular_runtime::EffectId::decode(
            &circular_core::encode(effect, Ceilings::for_boundary(Boundary::Identity))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            effect,
            birth: RevisionEpochId::new(*birth).ok_or("zero actor birth")?,
            correlation: ScheduleCorrelation::new(*correlation),
            deadline_unix_millis: *deadline,
        })
    }
}
pub fn read_schedule_reservation(
    record: &crate::BoundaryRecord<crate::ProductStore>,
) -> Result<ProductScheduleReservation, String> {
    let crate::BoundaryFact::ScheduleReservation { payload } = record.fact() else {
        return Err("not a schedule reservation".into());
    };
    let crate::ClassKey::Boundary(crate::BoundaryKey::ScheduleReservation { effect }) =
        record.header().key()
    else {
        return Err("schedule reservation has another key".into());
    };
    let row = ProductScheduleReservation::decode(payload)?;
    if &row.effect != effect || row.birth > record.header().at().revision() {
        return Err("schedule reservation key or birth differs from its record".into());
    }
    Ok(row)
}

