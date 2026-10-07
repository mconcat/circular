
use circular_core::{Ceilings, Value};

use crate::declaration_payload::{PlanExportKey, decode_plan_export_key};
use crate::wire_value::{PayloadRejection, bytes_of, exhausted, object, take};

#[derive(Clone, Debug, PartialEq)]
pub struct Inject {
    pub mount: PlanExportKey,
    pub payload: Value,
    pub idempotency: Vec<u8>,
}

pub fn decode_inject(bytes: &[u8], ceilings: Ceilings) -> Result<Inject, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let idempotency = bytes_of(take(&mut fields, "idempotency")?, "idempotency")?;
    if idempotency.is_empty() {
        return Err(PayloadRejection::WrongCarrier { key: "idempotency" });
    }
    let mount = decode_plan_export_key(take(&mut fields, "mount")?)?;
    let payload = take(&mut fields, "payload")?;
    exhausted(fields)?;
    Ok(Inject {
        mount,
        payload,
        idempotency,
    })
}

