//! Explicit fixture starting cut; never used by production issuance.
pub fn authoring_cut() -> crate::ProductAuthoringCut {
    crate::AuthoringCut {
        project: [0x10; 32],
        cursor: 1,
        environment: circular_protocol::declaration_payload::AuthoringEnvironment {
            declaration_schema: vec![0xa1],
            spec_set: vec![0xa2],
        },
        authoring_revision: circular_protocol::RevisionDigest::try_from_bytes(&[0xa3; 32]).unwrap(),
        topology_revision: circular_protocol::RevisionDigest::try_from_bytes(&[0xa4; 32]).unwrap(),
    }
}

pub fn manifest(
    run: crate::StreamId,
    cut: crate::ProductAuthoringCut,
) -> crate::RunManifest<crate::ProductStore> {
    manifest_with_start(run, crate::RevisionStart::Fresh(cut))
}

pub fn manifest_with_start(
    run: crate::StreamId,
    start: crate::RevisionStart<crate::ProductStore>,
) -> crate::RunManifest<crate::ProductStore> {
    use crate::*;
    use circular_core::{
        EncodedPayload, NonZeroTicks, PayloadVersionTag, TicksPerSecond, TimeSourceKind,
        TimeSourcePlan,
    };
    let payload = || EncodedPayload::new(PayloadVersionTag::FIRST, &[]);
    RunManifest::new(
        run,
        ManifestGroups::new(
            TimeParams::new(
                TicksPerSecond::new(1000).unwrap(),
                NonZeroTicks::new(1).unwrap(),
                payload(),
                TimeSourcePlan::Single(TimeSourceKind::Manual),
                OpaqueId::new(0),
            ),
            PlacementParams::from_validated(payload()),
            FailureParams::from_validated(payload()),
            payload(),
            RevisionContext::try_new(start, vec![]).unwrap(),
            RunInputs::from_primary_data(payload()),
        ),
    )
}
