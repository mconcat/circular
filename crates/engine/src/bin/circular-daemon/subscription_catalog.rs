
use circular_protocol::DeliveryDiscipline;

pub(crate) const ACTOR_EVENTS_TARGET: &str = "actor.events";

pub(crate) const DISPLAY_FRAMES_TARGET: &str = "display.frames";

pub(crate) const AUTHORING_COMMITS_TARGET: &str = "authoring-commits";

pub(crate) const EDGE_DEPTHS_TARGET: &str = "edge.depths";

pub(crate) const RECORDS_TARGET: &str = "records";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionTarget {
    ActorEvents,
    AuthoringCommits,
    DisplayFrames,
    Records,
    EdgeDepths,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArgumentSpec {
    Null,
    ScopeAndAfter,
    RecordsObject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AnsweredBy {
    StandingPipeline,
    AuthoringState,
    StateJournal,
    Actors,
}

#[derive(Clone, Copy)]
pub(crate) struct DescriptorSpec {
    pub(crate) id: SubscriptionTarget,
    pub(crate) name: &'static str,
    pub(crate) argument: ArgumentSpec,
    pub(crate) delivery: DeliveryDiscipline,
    pub(crate) answered_by: AnsweredBy,
}

const DESCRIPTORS: &[DescriptorSpec] = &[
    DescriptorSpec {
        id: SubscriptionTarget::ActorEvents,
        name: ACTOR_EVENTS_TARGET,
        argument: ArgumentSpec::Null,
        delivery: DeliveryDiscipline::Credit,
        answered_by: AnsweredBy::StandingPipeline,
    },
    DescriptorSpec {
        id: SubscriptionTarget::AuthoringCommits,
        name: AUTHORING_COMMITS_TARGET,
        argument: ArgumentSpec::ScopeAndAfter,
        delivery: DeliveryDiscipline::Credit,
        answered_by: AnsweredBy::AuthoringState,
    },
    DescriptorSpec {
        id: SubscriptionTarget::DisplayFrames,
        name: DISPLAY_FRAMES_TARGET,
        argument: ArgumentSpec::Null,
        delivery: DeliveryDiscipline::Credit,
        answered_by: AnsweredBy::StandingPipeline,
    },
    DescriptorSpec {
        id: SubscriptionTarget::EdgeDepths,
        name: EDGE_DEPTHS_TARGET,
        argument: ArgumentSpec::Null,
        delivery: DeliveryDiscipline::Credit,
        answered_by: AnsweredBy::Actors,
    },
    DescriptorSpec {
        id: SubscriptionTarget::Records,
        name: RECORDS_TARGET,
        argument: ArgumentSpec::RecordsObject,
        delivery: DeliveryDiscipline::Credit,
        answered_by: AnsweredBy::StateJournal,
    },
];

pub(crate) fn descriptors() -> &'static [DescriptorSpec] {
    DESCRIPTORS
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn wire_rows() -> impl Iterator<Item = (String, &'static str, String)> {
    DESCRIPTORS.iter().map(|descriptor| {
        (
            format!("{:?}", descriptor.id),
            descriptor.name,
            format!("{:?}", descriptor.delivery),
        )
    })
}

pub(crate) fn resolve(name: &str) -> Option<SubscriptionTarget> {
    DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.name == name)
        .map(|descriptor| descriptor.id)
}

fn descriptor(id: SubscriptionTarget) -> &'static DescriptorSpec {
    DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.id == id)
        .expect("one descriptor per registration identity")
}

impl SubscriptionTarget {
    pub(crate) fn name(self) -> &'static str {
        descriptor(self).name
    }

    pub(crate) fn answered_by(self) -> AnsweredBy {
        descriptor(self).answered_by
    }

    pub(crate) fn delivery(self) -> DeliveryDiscipline {
        descriptor(self).delivery
    }

    pub(crate) fn argument(self) -> ArgumentSpec {
        descriptor(self).argument
    }
}

fn validate_descriptor_order(descriptors: &[DescriptorSpec]) -> Result<(), String> {
    for pair in descriptors.windows(2) {
        if pair[0].name >= pair[1].name {
            return Err(format!(
                "subscription descriptors are not in unique canonical order at {:?}, {:?}",
                pair[0].name, pair[1].name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_NAMES: [&str; 5] = [
        "actor.events",
        "authoring-commits",
        "display.frames",
        "edge.depths",
        "records",
    ];

    #[test]
    fn duplicate_or_unsorted_registry_rows_are_rejected() {
        let row = DESCRIPTORS[0];
        assert!(validate_descriptor_order(&[row, row]).is_err());
        assert!(validate_descriptor_order(&[DESCRIPTORS[1], DESCRIPTORS[0]]).is_err());
    }

    #[test]
    fn every_registered_name_resolves_and_nothing_else_does() {
        for descriptor in DESCRIPTORS {
            assert_eq!(resolve(descriptor.name), Some(descriptor.id));
            assert_eq!(descriptor.id.name(), descriptor.name);
        }
        assert_eq!(resolve("not-listed"), None);
        assert_eq!(
            resolve("display.rollup"),
            None,
            "a query name is not a target"
        );
    }

    #[test]
    fn the_answering_producer_comes_from_the_registration() {
        let answered = DESCRIPTORS
            .iter()
            .map(|descriptor| (descriptor.name, descriptor.id.answered_by()))
            .collect::<Vec<_>>();
        assert_eq!(
            answered,
            vec![
                ("actor.events", AnsweredBy::StandingPipeline),
                ("authoring-commits", AnsweredBy::AuthoringState),
                ("display.frames", AnsweredBy::StandingPipeline),
                ("edge.depths", AnsweredBy::Actors),
                ("records", AnsweredBy::StateJournal),
            ]
        );
    }

    #[test]
    fn the_argument_shape_class_comes_from_the_registration() {
        assert_eq!(
            SubscriptionTarget::ActorEvents.argument(),
            ArgumentSpec::Null
        );
        assert_eq!(
            SubscriptionTarget::DisplayFrames.argument(),
            ArgumentSpec::Null
        );
        assert_eq!(
            SubscriptionTarget::AuthoringCommits.argument(),
            ArgumentSpec::ScopeAndAfter
        );
        assert_eq!(
            SubscriptionTarget::Records.argument(),
            ArgumentSpec::RecordsObject
        );
    }
}
