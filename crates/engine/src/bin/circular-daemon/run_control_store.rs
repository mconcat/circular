//! The stream identity comes from the state manifest. Lifecycle is System's column.

pub(crate) fn state_stream(
    state_directory: Option<&std::path::Path>,
) -> Result<circular_store::StreamId, String> {
    if let Some(directory) = state_directory
        && let Some(manifest) = engine::state_manifest::read_state_manifest(
            &engine::state_journal::state_journal_path(directory),
        )?
    {
        return Ok(*manifest.stream());
    }
    Ok(circular_store::StreamId::new(1))
}
