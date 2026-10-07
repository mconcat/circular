
use std::fmt;

use circular_core::spelling::{ScalarText, ScopeText, SegmentText, SpelledSegment};

use crate::identity::{
    ActorId, InstanceKey, InstanceScalar, LocalKey, NamedActorId, ScopeId, ScopeSeg, ScopedActorId,
    SystemActor,
};

impl SpelledSegment for ScopeSeg {
    fn spelled(&self) -> SegmentText<'_> {
        fn scalar(value: &InstanceScalar) -> ScalarText<'_> {
            match value {
                InstanceScalar::Text(text) => ScalarText::Text(text),
                InstanceScalar::Int(number) => ScalarText::Int(*number),
                InstanceScalar::Bool(flag) => ScalarText::Bool(*flag),
            }
        }
        match self {
            Self::Child(name) => SegmentText::Child(name.as_str()),
            Self::Instance { of, key } => SegmentText::Instance {
                of: of.as_str(),
                key: match key {
                    InstanceKey::Scalar(value) => vec![scalar(value)],
                    InstanceKey::Tuple(values) => values.iter().map(scalar).collect(),
                },
            },
        }
    }
}

impl ScopeId {
    #[must_use]
    pub fn spelled(&self) -> ScopeText<'_, ScopeSeg> {
        ScopeText(self.segments())
    }
}

impl fmt::Display for NamedActorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(
            &circular_core::spelling::actor(self.scope().segments(), self.name()),
            formatter,
        )
    }
}

impl fmt::Display for ScopedActorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let segments = self.scope().segments();
        match self.local() {
            LocalKey::Named(name) => {
                fmt::Display::fmt(&circular_core::spelling::actor(segments, name), formatter)
            }
            LocalKey::Ephemeral(uuid) => {
                let prefix: String = uuid.as_bytes()[..4]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                fmt::Display::fmt(
                    &circular_core::spelling::actor(segments, format!("ephemeral-{prefix}")),
                    formatter,
                )
            }
        }
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scoped { scope, local } => {
                fmt::Display::fmt(&ScopedActorId::new(scope.clone(), local.clone()), formatter)
            }
            Self::System(SystemActor::Stream) => formatter.write_str("the stream"),
            Self::System(SystemActor::Heartbeat) => formatter.write_str("the heartbeat"),
            Self::System(SystemActor::Pipeline) => formatter.write_str("the pipeline"),
        }
    }
}
