use std::path::Path;

use circular_protocol::SessionTokenSource;
use circular_store::{
    ArrivalProjection, JournalProjection, ProductJournalCodec, ProductStore, Record, RunManifest,
    SqliteJournal, SqliteJournalSnapshot, SqliteTransactionCodec, StructureFact, StructureRecord,
};

fn manifest_payloads<'a>(
    payloads: impl Iterator<Item = &'a [u8]>,
) -> Result<Option<RunManifest<ProductStore>>, String> {
    manifest_record_payloads(payloads)?
        .as_ref()
        .map(run_manifest)
        .transpose()
}

fn run_manifest(
    record: &StructureRecord<ProductStore>,
) -> Result<RunManifest<ProductStore>, String> {
    let StructureFact::RunManifest(manifest) = record.fact() else {
        return Err("state manifest record is not a manifest".to_owned());
    };
    Ok(manifest.clone())
}

fn manifest_record_payloads<'a>(
    mut payloads: impl Iterator<Item = &'a [u8]>,
) -> Result<Option<StructureRecord<ProductStore>>, String> {
    let Some(payload) = payloads.next() else {
        return Ok(None);
    };
    let transaction = ProductJournalCodec
        .decode(payload)
        .map_err(|error| format!("state manifest transaction is invalid: {error:?}"))?;
    let records = ArrivalProjection::new()
        .project(&transaction)
        .map_err(|_| {
            format!(
                "refused state manifest code={}: record is invalid or uses the old format",
                circular_protocol::rejection_code::RejectionReason::JournalFormatRejected
                    .recorded_code()
            )
        })?;
    let Some(Record::Structure(record)) = records.first() else {
        return Err("first runtime record must be the manifest".to_owned());
    };
    run_manifest(record)?;
    Ok(Some(record.clone()))
}

/// Read without creating a journal or issuing a project identity.
pub fn read_state_manifest(path: &Path) -> Result<Option<RunManifest<ProductStore>>, String> {
    read_state_manifest_record(path)?
        .as_ref()
        .map(run_manifest)
        .transpose()
}

pub fn read_state_manifest_record(
    path: &Path,
) -> Result<Option<StructureRecord<ProductStore>>, String> {
    if !path
        .try_exists()
        .map_err(|error| format!("state manifest path: {error}"))?
    {
        return Ok(None);
    }
    let journal = SqliteJournal::open_read_only_namespace(
        path,
        crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
    )
    .map_err(|error| format!("state manifest read failed: {error}"))?;
    let Some(first) = journal
        .namespace_first_commit()
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let (_, bytes) = journal
        .namespace_payload_at(first)
        .map_err(|e| e.to_string())?;
    manifest_record_payloads(std::iter::once(bytes.as_slice()))
}

/// Reuse the already verified immutable physical prefix. Namespace selection
/// does not open a second connection or hash its payloads again.
pub fn state_manifest_in_prefix(
    snapshot: &SqliteJournalSnapshot,
) -> Result<Option<RunManifest<ProductStore>>, String> {
    let mut payloads = Vec::new();
    for entry in snapshot.entries() {
        if let Some((namespace, payload)) = entry.namespace_payload().map_err(|e| e.to_string())?
            && namespace == crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE
        {
            payloads.push(payload);
        }
    }
    manifest_payloads(payloads.into_iter())
}

pub fn issue_project_identity() -> Result<[u8; circular_protocol::SESSION_TOKEN_BYTES], String> {
    crate::session_tokens::OsSessionTokenSource
        .draw()
        .map_err(|_| "state project OS entropy unavailable".to_owned())
}

pub fn stream_in_prefix(
    prefix: &SqliteJournalSnapshot,
) -> Result<Option<circular_store::StreamId>, String> {
    Ok(state_manifest_in_prefix(prefix)?.map(|manifest| *manifest.stream()))
}
