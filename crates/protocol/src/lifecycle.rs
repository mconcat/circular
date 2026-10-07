//! Published daemon health words. System outcomes and accepted control intents
//! are folded from the arrival journal; no separate lifecycle history is stored.

circular_core::closed_table! {
    pub enum LifecycleWord {
        Running => "running",
        Stopped => "stopped",
        ActivationFailed => "activation_failed",
        RevisionAdoptionFailed => "revision_adoption_failed",
        RecoveryFailed => "recovery_failed",
    }
}

pub const ALL_WORDS: &[LifecycleWord] = &LifecycleWord::ALL;

impl LifecycleWord {
    #[must_use]
    pub const fn pipeline_stands(self) -> bool {
        matches!(
            self,
            Self::Running | Self::Stopped | Self::RevisionAdoptionFailed
        )
    }
}
