//! The exact peer Envelope subcodec.
//! No record version, outcome tag or carrier prefix is added here.
use super::*;
use crate::effect_term_codec::{
    TermReader, read_flag, read_sized_bytes, read_str, read_tag, read_u8, read_u64,
};
use crate::outcome_codec::{
    address, bytes, count, read_address, read_count, read_snapshot, rejected, snapshot, u64,
    unknown,
};
use crate::{OutcomeCodecError, TermCodecError};

pub fn encode_peer_envelope(value: &PeerEventEnvelope) -> Result<Vec<u8>, OutcomeCodecError> {
    let mut out = Vec::new();
    bytes(&mut out, value.binding().as_bytes(), "envelope.binding")?;
    u64(&mut out, value.cursor().get());
    match value.event() {
        PeerEvent::Inbound(message) => {
            out.push(1);
            bytes(&mut out, message.id().as_bytes(), "message.id")?;
            optional_bytes(
                &mut out,
                message.provider_id().map(ProviderMessageId::as_bytes),
            )?;
            address(&mut out, message.from())?;
            address(&mut out, message.to())?;
            out.push(u8::from(message.reply_to().is_some()));
            if let Some(reply) = message.reply_to() {
                address(&mut out, reply)?;
            }
            optional_bytes(&mut out, message.correlation().map(PeerMessageId::as_bytes))?;
            bytes(&mut out, message.body().as_str().as_bytes(), "message.body")?;
            out.push(message.provenance().tag());
            out.extend_from_slice(
                &count(message.provider_fields().iter().len(), "provider.fields")?.to_le_bytes(),
            );
            for (key, value) in message.provider_fields().iter() {
                bytes(&mut out, key.as_bytes(), "provider.key")?;
                bytes(&mut out, value, "provider.value")?;
            }
        }
        PeerEvent::PeerSnapshotChanged(value) => {
            out.push(2);
            snapshot(&mut out, value)?;
        }
        PeerEvent::BindingStateChanged { binding, state } => {
            out.push(3);
            bytes(&mut out, binding.as_bytes(), "event.binding")?;
            out.push(state.tag());
        }
        PeerEvent::DeliveryChanged { message, state } => {
            out.push(4);
            bytes(&mut out, message.as_bytes(), "delivery.message")?;
            out.push(state.tag());
        }
        PeerEvent::AdapterDiagnostic(value) => {
            out.push(5);
            bytes(&mut out, value.code().as_bytes(), "diagnostic.code")?;
            bytes(&mut out, value.detail().as_bytes(), "diagnostic.detail")?;
        }
    }
    Ok(out)
}

fn optional_bytes(out: &mut Vec<u8>, value: Option<&[u8]>) -> Result<(), OutcomeCodecError> {
    out.push(u8::from(value.is_some()));
    if let Some(value) = value {
        bytes(out, value, "message.optional_id")?;
    }
    Ok(())
}

/// This restores recorded data, not an AdmittedPeerEvent or an ack proof.
pub fn decode_peer_envelope(input: &[u8]) -> Result<PeerEventEnvelope, OutcomeCodecError> {
    let mut reader = TermReader::new(input);
    let binding = PeerBindingId::try_new(read_sized_bytes(&mut reader, "envelope.binding")?)
        .map_err(|_| rejected("envelope.binding"))?;
    let cursor = PeerCursor::new(read_u64(&mut reader, "envelope.cursor")?);
    let event = match read_u8(&mut reader, "envelope.event")? {
        1 => PeerEvent::Inbound(read_message(&mut reader)?),
        2 => PeerEvent::PeerSnapshotChanged(read_snapshot(&mut reader)?),
        3 => PeerEvent::BindingStateChanged {
            binding: PeerBindingId::try_new(read_sized_bytes(&mut reader, "event.binding")?)
                .map_err(|_| rejected("event.binding"))?,
            state: read_tag(&mut reader, "binding.state", PeerBindingState::from_tag)?,
        },
        4 => PeerEvent::DeliveryChanged {
            message: PeerMessageId::try_new(read_sized_bytes(&mut reader, "delivery.message")?)
                .map_err(|_| rejected("delivery.message"))?,
            state: read_tag(&mut reader, "delivery.state", DeliveryDisposition::from_tag)?,
        },
        5 => PeerEvent::AdapterDiagnostic(PeerDiagnostic::new(
            read_str(&mut reader, "diagnostic.code")?,
            read_str(&mut reader, "diagnostic.detail")?,
        )),
        found => return Err(unknown("envelope.event", found)),
    };
    if !reader.finished() {
        return Err(TermCodecError::TrailingBytes {
            remaining: reader.remaining(),
        }
        .into());
    }
    Ok(PeerEventEnvelope::new(binding, cursor, event))
}

fn read_message(reader: &mut TermReader<'_>) -> Result<PeerMessage, OutcomeCodecError> {
    let id = PeerMessageId::try_new(read_sized_bytes(reader, "message.id")?)
        .map_err(|_| rejected("message.id"))?;
    let provider = if read_flag(reader, "message.provider")? {
        Some(
            ProviderMessageId::try_new(read_sized_bytes(reader, "message.provider")?)
                .map_err(|_| rejected("message.provider"))?,
        )
    } else {
        None
    };
    let from = read_address(reader)?;
    let to = read_address(reader)?;
    let reply = if read_flag(reader, "message.reply")? {
        Some(read_address(reader)?)
    } else {
        None
    };
    let correlation = if read_flag(reader, "message.correlation")? {
        Some(
            PeerMessageId::try_new(read_sized_bytes(reader, "message.correlation")?)
                .map_err(|_| rejected("message.correlation"))?,
        )
    } else {
        None
    };
    let body = PeerBody::try_new(read_str(reader, "message.body")?)
        .map_err(|_| rejected("message.body"))?;
    let provenance = read_tag(reader, "message.provenance", PeerProvenance::from_tag)?;
    let count = read_count(reader, "provider.fields")?;
    let mut fields = Vec::new();
    let mut previous = None;
    for _ in 0..count {
        let key = read_str(reader, "provider.key")?;
        if previous.is_some_and(|previous| previous >= key) {
            return Err(rejected("provider.key.order"));
        }
        fields.push((key, read_sized_bytes(reader, "provider.value")?));
        previous = Some(key);
    }
    let fields = OpaqueProviderFields::try_new(fields).map_err(|_| rejected("provider.fields"))?;
    Ok(PeerMessage::new(
        id,
        provider,
        from,
        to,
        reply,
        correlation,
        body,
        provenance,
        fields,
    ))
}

