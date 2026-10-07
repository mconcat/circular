//! Built-in observation names retained by the product record/query contract.

crate::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum BuiltinObservationName: u8 {
        DeliveryOccurrence = 0 => "DeliveryOccurrence",
        DeliveryException = 1 => "DeliveryException",
        ProcessingDisposition = 2 => "ProcessingDisposition",
        AdmissionDelay = 3 => "AdmissionDelay",
        DeadLetterEntry = 4 => "DeadLetterEntry",
        EffectItem = 5 => "EffectItem",
        EffectFailed = 6 => "EffectFailed",
        IncarnationTransition = 7 => "IncarnationTransition",
        ActivationOutcome = 8 => "ActivationOutcome",
        ApprovalSettlement = 9 => "ApprovalSettlement",
        OperationTransition = 10 => "OperationTransition",
        CausalChain = 11 => "CausalChain",
        BoundaryDelayExceeded = 12 => "BoundaryDelayExceeded",
        EditAttribution = 14 => "EditAttribution",
        RoutingDecision = 15 => "RoutingDecision",
        CustodyClaim = 16 => "CustodyClaim",
        SessionTransition = 17 => "SessionTransition",
        SubscriptionTransition = 18 => "SubscriptionTransition",
        StoreAppend = 19 => "StoreAppend",
        RegistrationLoaded = 20 => "RegistrationLoaded",
        DiagnosticOccurrence = 21 => "DiagnosticOccurrence",
        Actor = 22 => "actor",
        Incarnation = 23 => "incarnation",
        Scope = 24 => "scope",
        Operation = 25 => "operation",
        ReplaySession = 26 => "replay_session",
        Tally = 27 => "tally",
        Plane = 28 => "plane",
        Progress = 29 => "progress",
        Density = 30 => "density",
        Floor = 31 => "floor",
        Edge = 32 => "edge",
        Binding = 33 => "binding",
        Storage = 34 => "storage",
        BoundaryGrowth = 35 => "boundary_growth",
        InstanceTransition = 36 => "instance_transition",
        Restart = 38 => "restart",
        DaemonShutdown = 39 => "daemon_shutdown",
        StreamStart = 40 => "stream_start",
        SystemActivationOutcome = 41 => "system_activation_outcome",
        SystemRevisionAdoptionOutcome = 42 => "system_revision_adoption_outcome",
        SystemRecoveryOutcome = 43 => "system_recovery_outcome",
        SystemPauseAccepted = 44 => "system_pause_accepted",
        SystemResumeAccepted = 45 => "system_resume_accepted",
    }
    retired: [13, 37];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_catalog_is_complete_and_unique() {
        let names = BuiltinObservationName::ALL
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), BuiltinObservationName::ALL.len());
        assert!(names.contains(&BuiltinObservationName::DaemonShutdown));
        let spellings = BuiltinObservationName::ALL
            .iter()
            .map(|name| name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(spellings.len(), BuiltinObservationName::ALL.len());
        assert_eq!(BuiltinObservationName::Tally.as_str(), "tally");
        assert_eq!(
            BuiltinObservationName::DaemonShutdown.as_str(),
            "daemon_shutdown"
        );
    }

    #[test]
    fn a_retired_catalog_tag_stays_vacant_instead_of_moving_the_names_behind_it() {
        assert_eq!(BuiltinObservationName::from_tag(37), None);
        assert_eq!(BuiltinObservationName::Restart.tag(), 38);
        assert_eq!(BuiltinObservationName::DaemonShutdown.tag(), 39);
        for name in BuiltinObservationName::ALL {
            assert_eq!(BuiltinObservationName::from_tag(name.tag()), Some(name));
        }
    }
}
