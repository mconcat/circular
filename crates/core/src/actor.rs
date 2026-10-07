
use std::error::Error;
use std::fmt;

crate::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum ActorType: usize {
        Route = 0 => "route",
        FixtureInput = 1 => "fixture_input",
        FixtureMap = 2 => "fixture_map",
        FixtureFilter = 3 => "fixture_filter",
        FixtureTap = 4 => "fixture_tap",
        FixtureProjectOutput = 5 => "fixture_project_output",
        EditableCounter = 6 => "editable_counter",
        EditableAux = 7 => "editable_aux",
        EditableScopeProbe = 8 => "editable_scope_probe",
        PipelineActor = 9 => "pipeline_actor",
        Debounce = 10 => "debounce",
        Throttle = 11 => "throttle",
        Alert = 12 => "alert",
        Filter = 13 => "filter",
        Dedup = 16 => "dedup",
        Map = 19 => "map",
        Parse = 23 => "parse",
        Tap = 25 => "tap",
        Input = 26 => "input",
        Output = 27 => "output",
        Replicator = 28 => "replicator",
        Agent = 29 => "agent",
        TokenCostMeter = 30 => "token_cost_meter",
        Counter = 34 => "counter",
        Ema = 35 => "ema",
        WindowedReduce = 36 => "windowed_reduce",
        Timer = 39 => "timer",
        ToolExecutor = 42 => "tool_executor",
        Notify = 45 => "notify",
        Cli = 49 => "cli",
        Bang = 50 => "bang",
        Peer = 51 => "peer",
        Listener = 54 => "listener",
        KeyedReduce = 55 => "keyed_reduce",
        FixturePanic = 56 => "fixture_panic",
        Request = 57 => "request",
        File = 58 => "file",
        Json = 59 => "json",
        Otlp = 60 => "otlp",
        Match = 61 => "match",
        Assemble = 62 => "assemble",
        Join = 63 => "join",
        Form = 64 => "form",
    }
    retired: [
        14, 15, 17, 18, 20, 21, 22, 24, 31, 32, 33, 37, 38, 40, 41, 43, 44, 46, 47, 48, 52, 53,
    ];
}

impl ActorType {
    #[doc(hidden)]
    #[must_use]
    pub const fn registration_index(self) -> usize {
        let tag = self.tag();
        let mut retired = 0;
        let mut cell = 0;
        while cell < Self::RETIRED.len() {
            if Self::RETIRED[cell] < tag {
                retired += 1;
            }
            cell += 1;
        }
        tag - retired
    }

    #[must_use]
    pub const fn is_container(self) -> bool {
        matches!(self, Self::PipelineActor | Self::Replicator)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PortId(Box<str>);

impl PortId {
    pub fn try_new(value: impl Into<String>) -> Result<Self, PortIdError> {
        let value = value.into();
        validate_port_id(&value, true)?;
        Ok(Self(value.into_boxed_str()))
    }

    pub fn try_authored_static(value: &'static str) -> Result<AuthoredPortId, PortIdError> {
        validate_port_id(value, false)?;
        Ok(AuthoredPortId(Self(value.into())))
    }

    pub fn try_authored(value: String) -> Result<AuthoredPortId, PortIdError> {
        validate_port_id(&value, false)?;
        Ok(AuthoredPortId(Self(value.into_boxed_str())))
    }

    pub fn try_derived(value: String) -> Result<Self, PortIdError> {
        Self::try_new(value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthoredPortId(PortId);

impl AuthoredPortId {
    #[must_use]
    pub const fn as_port_id(&self) -> &PortId {
        &self.0
    }

    #[must_use]
    pub fn into_port_id(self) -> PortId {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortIdError {
    Empty,
    TooLong,
    InvalidFirstByte,
    InvalidByte,
    Reserved,
}

impl fmt::Display for PortIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "port id must not be empty",
            Self::TooLong => "port id must be at most 32 bytes",
            Self::InvalidFirstByte => "port id must start with lowercase ASCII",
            Self::InvalidByte => "port id contains a non-canonical byte",
            Self::Reserved => "authored port id must not use the reserved underscore prefix",
        };
        formatter.write_str(message)
    }
}

impl Error for PortIdError {}

fn validate_port_id(value: &str, derived: bool) -> Result<(), PortIdError> {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first().copied() else {
        return Err(PortIdError::Empty);
    };
    if bytes.len() > 32 {
        return Err(PortIdError::TooLong);
    }
    if first == b'_' {
        if !derived {
            return Err(PortIdError::Reserved);
        }
    } else if !first.is_ascii_lowercase() {
        return Err(PortIdError::InvalidFirstByte);
    }
    if bytes
        .iter()
        .any(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && *byte != b'_')
    {
        return Err(PortIdError::InvalidByte);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_pipeline_actor_owns_the_canonical_container_name() {
        assert_eq!(ActorType::PipelineActor.as_str(), "pipeline_actor");
        assert_eq!(
            ActorType::from_str("pipeline_actor"),
            Some(ActorType::PipelineActor)
        );
        assert_eq!(ActorType::from_str("fixture_pipeline_actor"), None);
    }

    #[test]
    fn canonical_references_include_reserved_ports_but_authored_ids_do_not() {
        let reserved = PortId::try_new("_error").expect("reserved references are canonical");
        assert_eq!(reserved.as_str(), "_error");
        assert_eq!(
            PortId::try_authored_static("_error"),
            Err(PortIdError::Reserved)
        );
        assert_eq!(PortId::try_new("Event"), Err(PortIdError::InvalidFirstByte));
    }
}
