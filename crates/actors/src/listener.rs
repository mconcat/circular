
use circular_core::Value;

pub const OP: &str = "op";

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum ControlPulse {
        ReplayFromStart => "replay_from_start",
    }
}

impl ControlPulse {
    #[must_use]
    pub fn to_value(self) -> Value {
        Value::object([(OP.to_owned(), Value::string(self.as_str()))])
            .expect("one field has no duplicate")
    }

    pub fn from_value(value: &Value) -> Result<Self, ControlPulseError> {
        let Some(object) = value.as_object() else {
            return Err(ControlPulseError::NotAnObject);
        };
        let op = object
            .get(OP)
            .and_then(Value::as_str)
            .ok_or(ControlPulseError::MissingOp)?;
        Self::from_str(op).ok_or(ControlPulseError::UnknownOp)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlPulseError {
    NotAnObject,
    MissingOp,
    UnknownOp,
}

impl core::fmt::Display for ControlPulseError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("control pulse is not an object"),
            Self::MissingOp => formatter.write_str("control pulse is missing op"),
            Self::UnknownOp => formatter.write_str("control pulse op is not a supported variant"),
        }
    }
}

impl std::error::Error for ControlPulseError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListenerSource {
    FileTail {
        glob: Box<str>,
        poll: circular_runtime::NonZeroMillis,
    },
}

impl ListenerSource {
    pub const KIND: &'static str = "kind";
    pub const VALUE: &'static str = "value";
    pub const KINDS: [&'static str; 1] = ["file_tail"];

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::FileTail { .. } => "file_tail",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListenerConfig {
    source: ListenerSource,
}

impl ListenerConfig {
    pub const SOURCE: &'static str = "source";

    pub fn from_value(value: &Value) -> Result<Self, ListenerConfigError> {
        let Some(root) = value.as_object() else {
            return Err(ListenerConfigError::NotAnObject);
        };
        let source = root
            .get(Self::SOURCE)
            .ok_or(ListenerConfigError::MissingSource)?;
        let Some(source) = source.as_object() else {
            return Err(ListenerConfigError::SourceIsNotAnObject);
        };
        let kind = source
            .get(ListenerSource::KIND)
            .and_then(Value::as_str)
            .ok_or(ListenerConfigError::MissingKind)?;
        if kind != "file_tail" {
            return Err(ListenerConfigError::UnknownKind);
        }
        let arm = source
            .get(ListenerSource::VALUE)
            .and_then(Value::as_object)
            .ok_or(ListenerConfigError::ArmIsNotAnObject)?;

        let glob = arm
            .get("glob")
            .and_then(Value::as_str)
            .ok_or(ListenerConfigError::MissingGlob)?;
        if glob.is_empty() {
            return Err(ListenerConfigError::EmptyGlob);
        }
        let poll = match arm
            .get("poll")
            .map(crate::actor_support::read_config_unsigned)
        {
            Some(crate::actor_support::ConfigUnsigned::Value(value)) => value,
            Some(crate::actor_support::ConfigUnsigned::Negative(_)) => {
                return Err(ListenerConfigError::PollNegative);
            }
            Some(crate::actor_support::ConfigUnsigned::NotAnInteger) | None => {
                return Err(ListenerConfigError::PollIsNotAnInteger);
            }
        };
        let poll = circular_runtime::NonZeroMillis::new(poll)
            .map_err(|_| ListenerConfigError::PollZero)?;

        Ok(Self {
            source: ListenerSource::FileTail {
                glob: glob.into(),
                poll,
            },
        })
    }

    #[must_use]
    pub const fn source(&self) -> &ListenerSource {
        &self.source
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListenerConfigError {
    NotAnObject,
    MissingSource,
    SourceIsNotAnObject,
    MissingKind,
    UnknownKind,
    ArmIsNotAnObject,
    MissingGlob,
    EmptyGlob,
    PollIsNotAnInteger,
    PollNegative,
    PollZero,
}

impl core::fmt::Display for ListenerConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::NotAnObject => "listener config is not an object",
            Self::MissingSource => "listener config is missing source",
            Self::SourceIsNotAnObject => "source is not an object",
            Self::MissingKind => "source is missing kind",
            Self::UnknownKind => "source kind is not a supported variant",
            Self::ArmIsNotAnObject => "source value is not an object",
            Self::MissingGlob => "file_tail is missing glob",
            Self::EmptyGlob => "glob is empty; this is different from matching no files",
            Self::PollIsNotAnInteger => "poll interval is not an integer",
            Self::PollNegative => "poll interval is negative",
            Self::PollZero => "poll interval is zero",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for ListenerConfigError {}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum ListenerRefusal {
        NotUtf8 => "not_utf8",
        FileUnreadable => "file_unreadable",
        RootUnreadable => "root_unreadable",
    }
}

impl ListenerRefusal {
    #[must_use]
    pub fn declared(self) -> circular_runtime::DeclaredReason {
        LISTENER_DEAD_LETTER_REASONS
            .resolve(self.as_str())
            .expect("the listener declares every arm of its refusal sum")
    }
}

pub static LISTENER_DEAD_LETTER_REASONS: std::sync::LazyLock<
    circular_runtime::ReasonDecl<circular_runtime::DeadLettering>,
> = std::sync::LazyLock::new(|| {
    circular_runtime::ReasonDecl::try_from_names(ListenerRefusal::ALL.map(ListenerRefusal::as_str))
        .expect("the listener refusal names are distinct")
});

