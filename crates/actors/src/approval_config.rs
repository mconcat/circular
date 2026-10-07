use crate::{ConfigPath, CreateInputAdmissionError};
use circular_core::{ActorType, Value};

pub const APPROVAL_FIELD: &str = "approval";
pub const APPROVAL_VALUES: [&str; 2] = ["none", "required"];

#[must_use]
pub fn approval_space() -> crate::ConfigSpace {
    crate::ConfigSpace::try_new(
        crate::Shape::Base(crate::BaseShape::String),
        crate::ConfigConstraint::ClosedTags(
            crate::ClosedTags::try_from_members(APPROVAL_VALUES)
                .expect("the approval values are a non-empty closed tag set"),
        ),
    )
    .expect("approval constrains strings")
}

pub fn required(value: Option<&Value>) -> Result<bool, &'static str> {
    match value {
        None => Ok(false),
        Some(Value::String(value)) if value == APPROVAL_VALUES[0] => Ok(false),
        Some(Value::String(value)) if value == APPROVAL_VALUES[1] => Ok(true),
        _ => Err("approval must be none or required"),
    }
}

pub(crate) fn admit_beyond_schema(
    actor_type: ActorType,
    config: &Value,
) -> Result<(), crate::RegisteredCreateAdmissionError> {
    let Some(root) = config.as_object() else {
        return Ok(());
    };
    match actor_type {
        ActorType::ToolExecutor => {
            if let Some(tools) = root.get("tools").and_then(Value::as_object) {
                for (name, tool) in tools.iter() {
                    if let Some(tool) = tool.as_object() {
                        required(tool.get(APPROVAL_FIELD)).map_err(|_| {
                            crate::RegisteredCreateAdmissionError::Config(
                                CreateInputAdmissionError::OutsideSpace {
                                    path: ConfigPath::root()
                                        .join_key("tools")
                                        .join_key(name)
                                        .join_key(APPROVAL_FIELD),
                                    space: Some(approval_space()),
                                },
                            )
                        })?;
                    }
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

