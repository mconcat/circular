use std::borrow::Cow;

use circular_store::{
    ProductRecordCodec, ProductStore, Record, RecordEnvelope, StoredRow, ViewRow,
};

pub(crate) fn with_envelope<T>(
    row: &ViewRow<'_>,
    read: impl FnOnce(&RecordEnvelope<'_>) -> T,
) -> Result<Option<T>, String> {
    let envelope_error = |error: circular_store::RecordCodecError| {
        format!("record envelope at {}: {error:?}", row.position())
    };
    match row.stored() {
        StoredRow::Encoded(payload) => circular_store::decode_envelope(payload.body())
            .map(|envelope| Some(read(&envelope)))
            .map_err(envelope_error),
        StoredRow::Decoded(record) => {
            match circular_store::encode_record(record, &ProductRecordCodec) {
                Ok(bytes) => circular_store::decode_envelope(&bytes)
                    .map(|envelope| Some(read(&envelope)))
                    .map_err(envelope_error),
                Err(_) => Ok(None),
            }
        }
    }
}

pub(crate) fn keeps(
    row: &ViewRow<'_>,
    keep: impl FnOnce(&RecordEnvelope<'_>) -> bool,
) -> Result<bool, String> {
    Ok(with_envelope(row, keep)?.unwrap_or(true))
}

pub(crate) fn rebuild<'r>(row: &'r ViewRow<'_>) -> Cow<'r, Record<ProductStore>> {
    #[cfg(test)]
    REBUILDS.with(|count| count.set(count.get() + 1));
    row.record()
}

#[cfg(test)]
thread_local! {
    static REBUILDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn thread_rebuilds() -> u64 {
    REBUILDS.with(std::cell::Cell::get)
}

pub(crate) fn observation_kind(
    envelope: &RecordEnvelope<'_>,
) -> Option<circular_core::BuiltinObservationName> {
    if envelope.class != circular_store::Class::Observation {
        return None;
    }
    circular_core::BuiltinObservationName::from_tag(*envelope.class_key_body.first()?)
}

pub(crate) struct ArrivalEnvelope {
    pub(crate) index: u64,
    pub(crate) observed_at: circular_core::RecordedInstant,
}

pub(crate) fn arrival_envelope(row: &ViewRow<'_>) -> Result<Option<ArrivalEnvelope>, String> {
    if let Some(found) = with_envelope(row, |envelope| {
        match (
            envelope.arrival_origin_tag,
            envelope.arrival_index,
            envelope.observed_at,
        ) {
            (Some(_), Some(index), Some(observed_at)) => Some(ArrivalEnvelope {
                index: index.get(),
                observed_at,
            }),
            _ => None,
        }
    })? {
        return Ok(found);
    }
    let record = rebuild(row);
    let Record::Boundary(boundary) = &*record else {
        return Ok(None);
    };
    let circular_store::BoundaryFact::Arrival {
        arrival_index,
        observed_at,
        ..
    } = boundary.fact()
    else {
        return Ok(None);
    };
    Ok(Some(ArrivalEnvelope {
        index: arrival_index.get(),
        observed_at: *observed_at,
    }))
}

pub(crate) fn witness(row: &ViewRow<'_>) -> Result<circular_store::OpaqueWitness, String> {
    let witness = with_envelope(row, |envelope| {
        let stamped = envelope.class == circular_store::Class::Observation
            || (envelope.class == circular_store::Class::Boundary
                && envelope.class_key_body.is_empty());
        (!stamped).then(|| circular_store::OpaqueWitness::for_envelope(envelope))
    })?
    .flatten();
    match witness {
        Some(witness) => Ok(witness),
        None => crate::daemon::ledger::record_witness(rebuild(row).header()),
    }
}
