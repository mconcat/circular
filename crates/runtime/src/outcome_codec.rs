use crate::effect_term_codec::{
    TermReader, decode_effect_failure, encode_effect_failure, read_flag, read_i32,
    read_sized_bytes, read_str, read_tag, read_u8, read_u16, read_u32, read_u64,
};
use crate::{
    AgentHarnessName, AgentPayload, AgentProgressRecord, AgentSessionId, AgentStepNext,
    AgentStepResult, AgentToolCall, AgentToolCallId, ApprovalRequestOutcome, ApprovalTicket,
    BindingLease, DeliveryDisposition, EffectFailure, HttpResponse, NotificationChannel,
    NotificationReceipt, OutcomePayload, Peer, PeerActorIncarnation, PeerAdapterName, PeerAddress,
    PeerAvailability, PeerBinding, PeerBindingCapabilities, PeerBindingId, PeerCapabilities,
    PeerDisplayName, PeerId, PeerKind, PeerMessageId, PeerRealmId, PeerSnapshot, PeerStamp,
    ProcessResult, ProviderMessageId, ScheduleCorrelation, SubmissionReceipt, TermCodecError,
    ToolName, UnbindReceipt,
};

pub const OUTCOME_CODEC_VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomeCodecError {
    Malformed(TermCodecError),
    LengthOverflow { position: &'static str },
}

impl From<TermCodecError> for OutcomeCodecError {
    fn from(error: TermCodecError) -> Self {
        Self::Malformed(error)
    }
}
impl std::fmt::Display for OutcomeCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "outcome codec: {self:?}")
    }
}
impl std::error::Error for OutcomeCodecError {}

pub(crate) fn count(value: usize, position: &'static str) -> Result<u32, OutcomeCodecError> {
    u32::try_from(value).map_err(|_| OutcomeCodecError::LengthOverflow { position })
}
pub(crate) fn bytes(
    out: &mut Vec<u8>,
    value: &[u8],
    position: &'static str,
) -> Result<(), OutcomeCodecError> {
    out.extend_from_slice(&count(value.len(), position)?.to_le_bytes());
    out.extend_from_slice(value);
    Ok(())
}
pub(crate) fn u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
pub(crate) fn address(out: &mut Vec<u8>, value: &PeerAddress) -> Result<(), OutcomeCodecError> {
    bytes(out, value.adapter().as_str().as_bytes(), "address.adapter")?;
    bytes(out, value.realm().as_bytes(), "address.realm")?;
    bytes(out, value.peer().as_bytes(), "address.peer")
}

pub(crate) fn snapshot(out: &mut Vec<u8>, value: &PeerSnapshot) -> Result<(), OutcomeCodecError> {
    bytes(out, value.realm().as_bytes(), "snapshot.realm")?;
    out.extend_from_slice(&count(value.peers().len(), "snapshot.peers")?.to_le_bytes());
    for peer in value.peers() {
        address(out, peer.address())?;
        bytes(
            out,
            peer.display_name().as_str().as_bytes(),
            "peer.display_name",
        )?;
        out.push(peer.kind().tag());
        out.push(peer.availability().tag());
        out.extend_from_slice(&[
            u8::from(peer.capabilities().receive_text()),
            u8::from(peer.capabilities().idle_wakeup()),
            u8::from(peer.capabilities().active_turn_inject()),
        ]);
    }
    u64(out, value.observed_at().get());
    Ok(())
}

pub(crate) fn binding(out: &mut Vec<u8>, value: &PeerBinding) -> Result<(), OutcomeCodecError> {
    bytes(out, value.id().as_bytes(), "binding.id")?;
    bytes(out, value.actor().actor(), "binding.actor")?;
    bytes(out, value.actor().incarnation(), "binding.incarnation")?;
    address(out, value.address())?;
    bytes(
        out,
        value.effective_name().as_str().as_bytes(),
        "binding.name",
    )?;
    match value.lease() {
        BindingLease::Process => out.push(1),
        BindingLease::Until(stamp) => {
            out.push(2);
            u64(out, stamp.get());
        }
    }
    out.extend_from_slice(&[
        u8::from(value.capabilities().receive_text()),
        u8::from(value.capabilities().provider_ack()),
    ]);
    Ok(())
}

fn progress_column(
    out: &mut Vec<u8>,
    records: &[AgentProgressRecord],
    [column, payload]: [&'static str; 2],
) -> Result<(), OutcomeCodecError> {
    out.extend_from_slice(&count(records.len(), column)?.to_le_bytes());
    for record in records {
        bytes(out, record.payload().as_bytes(), payload)?;
    }
    Ok(())
}

fn read_progress_column(
    reader: &mut TermReader<'_>,
    [column, payload]: [&'static str; 2],
) -> Result<Vec<AgentProgressRecord>, OutcomeCodecError> {
    let count = read_count(reader, column)?;
    let mut records = Vec::new();
    for _ in 0..count {
        records.push(AgentProgressRecord::new(AgentPayload::new(
            read_sized_bytes(reader, payload)?,
        )));
    }
    Ok(records)
}

const AGENT_PROGRESS: [&str; 2] = ["agent.progress", "agent.progress.payload"];
const FAILURE_PROGRESS: [&str; 2] = ["failure.progress", "failure.progress.payload"];

pub fn encode_outcome(
    value: &Result<OutcomePayload, EffectFailure>,
    failure_progress: &[AgentProgressRecord],
) -> Result<Vec<u8>, OutcomeCodecError> {
    let mut out = vec![OUTCOME_CODEC_VERSION];
    let payload = match value {
        Err(failure) if failure_progress.is_empty() => {
            out.push(2);
            encode_effect_failure(&mut out, failure);
            return Ok(out);
        }
        Err(failure) => {
            out.push(3);
            encode_effect_failure(&mut out, failure);
            progress_column(&mut out, failure_progress, FAILURE_PROGRESS)?;
            return Ok(out);
        }
        Ok(_) if !failure_progress.is_empty() => return Err(rejected("failure.progress")),
        Ok(payload) => {
            out.push(1);
            payload
        }
    };
    match payload {
        OutcomePayload::PeerEnvelope(_) => {
            return Err(rejected("peer envelope requires Receive term context"));
        }
        OutcomePayload::HttpResponse(value) => {
            out.push(1);
            out.extend_from_slice(&value.status().to_le_bytes());
            bytes(&mut out, value.body(), "http.body")?;
            out.push(u8::from(value.truncated()));
            match value.retry_after_seconds() {
                None => out.push(0),
                Some(value) => {
                    out.push(1);
                    u64(&mut out, value);
                }
            }
        }
        OutcomePayload::FileBytes(value) => {
            out.push(2);
            bytes(&mut out, value, "file.bytes")?;
        }
        OutcomePayload::WrittenLength(value) => {
            out.push(3);
            u64(&mut out, *value);
        }
        OutcomePayload::ProcessResult(value) => {
            out.push(4);
            out.extend_from_slice(&value.exit_code().to_le_bytes());
            bytes(&mut out, value.stdout(), "process.stdout")?;
            bytes(&mut out, value.stderr(), "process.stderr")?;
        }
        OutcomePayload::NotificationDelivered(value) => {
            out.push(5);
            bytes(
                &mut out,
                value.channel().as_str().as_bytes(),
                "notification.channel",
            )?;
        }
        OutcomePayload::AgentStepResult(value) => {
            out.push(6);
            bytes(
                &mut out,
                value.session().harness().as_str().as_bytes(),
                "agent.harness",
            )?;
            bytes(&mut out, value.session().opaque(), "agent.session")?;
            progress_column(&mut out, value.progress(), AGENT_PROGRESS)?;
            match value.next() {
                AgentStepNext::Final { output, metadata } => {
                    out.push(1);
                    bytes(&mut out, output.as_bytes(), "agent.output")?;
                    bytes(&mut out, metadata.as_bytes(), "agent.metadata")?;
                }
                AgentStepNext::ToolRequest { call } => {
                    out.push(2);
                    bytes(&mut out, call.id().as_bytes(), "agent.call")?;
                    bytes(&mut out, call.tool().as_str().as_bytes(), "agent.tool")?;
                    bytes(&mut out, call.arguments().as_bytes(), "agent.arguments")?;
                }
            }
        }
        OutcomePayload::Approval(value) => {
            out.push(7);
            match value {
                ApprovalRequestOutcome::Approved(ticket) => {
                    out.push(1);
                    bytes(
                        &mut out,
                        &crate::EffectId::encode(ticket.ledger_item())
                            .map_err(|_| rejected("approval.ledger"))?,
                        "approval.ledger",
                    )?;
                    bytes(
                        &mut out,
                        &crate::EffectId::encode(ticket.target_effect())
                            .map_err(|_| rejected("approval.effect"))?,
                        "approval.effect",
                    )?;
                }
                ApprovalRequestOutcome::Denied => out.push(2),
            }
        }
        OutcomePayload::ScheduleArmed(value) => {
            out.push(8);
            u64(&mut out, value.get());
        }
        OutcomePayload::PeerSnapshot(value) => {
            out.push(9);
            snapshot(&mut out, value)?;
        }
        OutcomePayload::PeerBinding(value) => {
            out.push(10);
            binding(&mut out, value)?;
        }
        OutcomePayload::SubmissionReceipt(value) => {
            out.push(11);
            bytes(&mut out, value.message().as_bytes(), "receipt.message")?;
            match value.provider_id() {
                None => out.push(0),
                Some(id) => {
                    out.push(1);
                    bytes(&mut out, id.as_bytes(), "receipt.provider")?;
                }
            }
            out.push(value.disposition().tag());
            u64(&mut out, value.accepted_at().get());
        }
        OutcomePayload::UnbindReceipt(value) => {
            out.push(12);
            bytes(&mut out, value.binding().as_bytes(), "unbind.binding")?;
            address(&mut out, value.address())?;
            u64(&mut out, value.closed_at().get());
        }
    }
    Ok(out)
}

pub(crate) fn rejected(position: &'static str) -> OutcomeCodecError {
    TermCodecError::RejectedByConstructor { position }.into()
}
pub(crate) fn unknown(position: &'static str, found: u8) -> OutcomeCodecError {
    TermCodecError::UnknownTag { position, found }.into()
}

pub(crate) fn read_count(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<usize, TermCodecError> {
    let count = read_u32(reader, position)? as usize;
    if count > reader.remaining() / 4 {
        return Err(TermCodecError::Truncated { position });
    }
    Ok(count)
}

pub(crate) fn read_address(reader: &mut TermReader<'_>) -> Result<PeerAddress, OutcomeCodecError> {
    Ok(PeerAddress::new(
        PeerAdapterName::try_new(read_str(reader, "address.adapter")?)
            .map_err(|_| rejected("address.adapter"))?,
        PeerRealmId::try_new(read_sized_bytes(reader, "address.realm")?)
            .map_err(|_| rejected("address.realm"))?,
        PeerId::try_new(read_sized_bytes(reader, "address.peer")?)
            .map_err(|_| rejected("address.peer"))?,
    ))
}
pub(crate) fn read_binding(reader: &mut TermReader<'_>) -> Result<PeerBinding, OutcomeCodecError> {
    let id = PeerBindingId::try_new(read_sized_bytes(reader, "binding.id")?)
        .map_err(|_| rejected("binding.id"))?;
    let actor = PeerActorIncarnation::try_new(
        read_sized_bytes(reader, "binding.actor")?,
        read_sized_bytes(reader, "binding.incarnation")?,
    )
    .map_err(|_| rejected("binding.actor_incarnation"))?;
    let address = read_address(reader)?;
    let name = PeerDisplayName::try_new(read_str(reader, "binding.name")?)
        .map_err(|_| rejected("binding.name"))?;
    let lease = match read_u8(reader, "binding.lease")? {
        1 => BindingLease::Process,
        2 => BindingLease::Until(PeerStamp::new(read_u64(reader, "binding.until")?)),
        found => return Err(unknown("binding.lease", found)),
    };
    let caps = PeerBindingCapabilities::new(
        read_flag(reader, "binding.receive_text")?,
        read_flag(reader, "binding.provider_ack")?,
    );
    Ok(PeerBinding::new(id, actor, address, name, lease, caps))
}

pub(crate) fn read_snapshot(
    reader: &mut TermReader<'_>,
) -> Result<PeerSnapshot, OutcomeCodecError> {
    let realm = PeerRealmId::try_new(read_sized_bytes(reader, "snapshot.realm")?)
        .map_err(|_| rejected("snapshot.realm"))?;
    let count = read_count(reader, "snapshot.peers")?;
    let mut peers = Vec::new();
    for _ in 0..count {
        let address = read_address(reader)?;
        let display = PeerDisplayName::try_new(read_str(reader, "peer.display_name")?)
            .map_err(|_| rejected("peer.display_name"))?;
        let kind = read_tag(reader, "peer.kind", PeerKind::from_tag)?;
        let availability = read_tag(reader, "peer.availability", PeerAvailability::from_tag)?;
        let caps = PeerCapabilities::new(
            read_flag(reader, "peer.receive_text")?,
            read_flag(reader, "peer.idle_wakeup")?,
            read_flag(reader, "peer.active_turn_inject")?,
        );
        peers.push(Peer::new(address, display, kind, availability, caps));
    }
    Ok(PeerSnapshot::new(
        realm,
        peers,
        PeerStamp::new(read_u64(reader, "snapshot.observed_at")?),
    ))
}
fn read_success(reader: &mut TermReader<'_>) -> Result<OutcomePayload, OutcomeCodecError> {
    Ok(match read_u8(reader, "outcome.success")? {
        1 => {
            let status = read_u16(reader, "http.status")?;
            let body = read_sized_bytes(reader, "http.body")?;
            let truncated = read_flag(reader, "http.truncated")?;
            let retry = if read_flag(reader, "http.retry")? {
                Some(read_u64(reader, "http.retry.seconds")?)
            } else {
                None
            };
            OutcomePayload::HttpResponse(HttpResponse::new(status, body, truncated, retry))
        }
        2 => OutcomePayload::FileBytes(read_sized_bytes(reader, "file.bytes")?.into()),
        3 => OutcomePayload::WrittenLength(read_u64(reader, "file.length")?),
        4 => {
            let exit = read_i32(reader, "process.exit")?;
            OutcomePayload::ProcessResult(ProcessResult::direct(
                exit,
                read_sized_bytes(reader, "process.stdout")?,
                read_sized_bytes(reader, "process.stderr")?,
            ))
        }
        5 => OutcomePayload::NotificationDelivered(NotificationReceipt::delivered(
            NotificationChannel::from_normalized(read_str(reader, "notification.channel")?),
        )),
        6 => {
            let harness = AgentHarnessName::try_from_normalized(read_str(reader, "agent.harness")?)
                .map_err(|_| rejected("agent.harness"))?;
            let session =
                AgentSessionId::new(harness.clone(), read_sized_bytes(reader, "agent.session")?);
            let progress = read_progress_column(reader, AGENT_PROGRESS)?;
            let next = match read_u8(reader, "agent.next")? {
                1 => AgentStepNext::Final {
                    output: AgentPayload::new(read_sized_bytes(reader, "agent.output")?),
                    metadata: AgentPayload::new(read_sized_bytes(reader, "agent.metadata")?),
                },
                2 => AgentStepNext::ToolRequest {
                    call: AgentToolCall::new(
                        AgentToolCallId::try_from_bytes(read_sized_bytes(reader, "agent.call")?)
                            .map_err(|_| rejected("agent.call"))?,
                        ToolName::try_from_normalized(read_str(reader, "agent.tool")?)
                            .map_err(|_| rejected("agent.tool"))?,
                        AgentPayload::new(read_sized_bytes(reader, "agent.arguments")?),
                    ),
                },
                found => return Err(unknown("agent.next", found)),
            };
            OutcomePayload::AgentStepResult(
                AgentStepResult::try_new(&harness, session, progress, next)
                    .map_err(|_| rejected("agent.session"))?,
            )
        }
        7 => OutcomePayload::Approval(match read_u8(reader, "approval.result")? {
            1 => ApprovalRequestOutcome::Approved(ApprovalTicket::restore(
                crate::EffectId::decode(read_sized_bytes(reader, "approval.ledger")?)
                    .map_err(|_| rejected("approval.ledger"))?,
                crate::EffectId::decode(read_sized_bytes(reader, "approval.effect")?)
                    .map_err(|_| rejected("approval.effect"))?,
            )),
            2 => ApprovalRequestOutcome::Denied,
            found => return Err(unknown("approval.result", found)),
        }),
        8 => OutcomePayload::ScheduleArmed(ScheduleCorrelation::new(read_u64(
            reader,
            "schedule.correlation",
        )?)),
        9 => OutcomePayload::PeerSnapshot(read_snapshot(reader)?),
        10 => OutcomePayload::PeerBinding(read_binding(reader)?),
        11 => {
            let message = PeerMessageId::try_new(read_sized_bytes(reader, "receipt.message")?)
                .map_err(|_| rejected("receipt.message"))?;
            let provider = if read_flag(reader, "receipt.provider")? {
                Some(
                    ProviderMessageId::try_new(read_sized_bytes(reader, "receipt.provider.id")?)
                        .map_err(|_| rejected("receipt.provider.id"))?,
                )
            } else {
                None
            };
            let disposition =
                read_tag(reader, "receipt.disposition", DeliveryDisposition::from_tag)?;
            OutcomePayload::SubmissionReceipt(
                SubmissionReceipt::try_new(
                    message,
                    provider,
                    disposition,
                    PeerStamp::new(read_u64(reader, "receipt.accepted_at")?),
                )
                .map_err(|_| rejected("receipt.disposition"))?,
            )
        }
        12 => OutcomePayload::UnbindReceipt(UnbindReceipt::new(
            PeerBindingId::try_new(read_sized_bytes(reader, "unbind.binding")?)
                .map_err(|_| rejected("unbind.binding"))?,
            read_address(reader)?,
            PeerStamp::new(read_u64(reader, "unbind.closed_at")?),
        )),
        found => return Err(unknown("outcome.success", found)),
    })
}

pub fn decode_outcome(
    bytes: &[u8],
) -> Result<
    (
        Result<OutcomePayload, EffectFailure>,
        Vec<AgentProgressRecord>,
    ),
    OutcomeCodecError,
> {
    let Some(&version) = bytes.first() else {
        return Err(TermCodecError::Empty.into());
    };
    if version != OUTCOME_CODEC_VERSION {
        return Err(TermCodecError::UnknownVersion { found: version }.into());
    }
    let mut reader = TermReader::new(&bytes[1..]);
    let settled = match read_u8(&mut reader, "outcome.result")? {
        1 => (Ok(read_success(&mut reader)?), Vec::new()),
        2 => (Err(decode_effect_failure(&mut reader)?), Vec::new()),
        3 => {
            let failure = decode_effect_failure(&mut reader)?;
            let progress = read_progress_column(&mut reader, FAILURE_PROGRESS)?;
            if progress.is_empty() {
                return Err(rejected("failure.progress"));
            }
            (Err(failure), progress)
        }
        found => return Err(unknown("outcome.result", found)),
    };
    if !reader.finished() {
        return Err(TermCodecError::TrailingBytes {
            remaining: reader.remaining(),
        }
        .into());
    }
    Ok(settled)
}

