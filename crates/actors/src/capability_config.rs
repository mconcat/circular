use crate::{ConfigPath, CreateInputAdmissionError};
use circular_core::{ActorType, Value};
use circular_runtime::{Capability, EffectCtor, NormalizedPath, PathScope, PathScopes};

pub const FIELD: &str = "capabilities";
/// Each grantable capability: its config key, and the words a form heads its policy
/// with — published as the `group` of that policy's inputs, the group they are drawn under.
pub const NAMES: [(Capability, &str, &str); 5] = [
    (Capability::FsRead, "FsRead", "Read files"),
    (Capability::FsWrite, "FsWrite", "Write files"),
    (Capability::HttpFetch, "HttpFetch", "Send HTTP requests"),
    (Capability::ProcessSpawn, "ProcessSpawn", "Run programs"),
    (Capability::UserNotify, "UserNotify", "Send notifications"),
];

pub fn name(capability: Capability) -> Option<&'static str> {
    NAMES
        .iter()
        .find_map(|(kind, name, _)| (*kind == capability).then_some(*name))
}

fn heading(capability: Capability) -> Option<&'static str> {
    NAMES
        .iter()
        .find_map(|(kind, _, heading)| (*kind == capability).then_some(*heading))
}

/// Policy inputs for the capabilities declared by this actor. These describe
/// each policy when needed; authored tools still determine actual demand.
pub fn create_inputs(
    actor_type: ActorType,
) -> Vec<(&'static str, Vec<(ConfigPath, crate::ConfigSlot)>)> {
    use crate::{BaseShape, ConfigSlot, ConfigSpace, Required, Shape};
    crate::registration(actor_type)
        .spec()
        .requires()
        .iter()
        .filter_map(|rule| {
            let capability = rule.capability();
            let name = name(capability)?;
            let heading = heading(capability)?;
            let approval = crate::approval_config::approval_space();
            let mandatory = |space| {
                ConfigSlot::try_new(space, Required::Mandatory, None)
                    .expect("mandatory policy inputs have no default")
            };
            let mut slots = vec![(
                path(capability, "approval"),
                mandatory(approval)
                    .with_text(
                        "Approval",
                        "Whether an effect under this grant waits for approval.",
                    )
                    .in_group(heading),
            )];
            if has_roots(capability) {
                slots.push((
                    path(capability, "roots"),
                    mandatory(ConfigSpace::unconstrained(Shape::Array(Box::new(
                        Shape::Base(BaseShape::String),
                    ))))
                    .with_text(
                        "Allowed directories",
                        "The absolute directories this grant covers.",
                    )
                    .in_group(heading),
                ));
            }
            Some((name, slots))
        })
        .collect()
}

fn has_roots(capability: Capability) -> bool {
    matches!(capability, Capability::FsRead | Capability::FsWrite)
}

pub fn for_effect(effect: EffectCtor) -> Option<Capability> {
    match effect {
        EffectCtor::FileRead => Some(Capability::FsRead),
        EffectCtor::FileWrite => Some(Capability::FsWrite),
        EffectCtor::Http => Some(Capability::HttpFetch),
        EffectCtor::Spawn => Some(Capability::ProcessSpawn),
        EffectCtor::Notify => Some(Capability::UserNotify),
        _ => None,
    }
}

/// What makes a declared grant's policy part of an authored config. A registration
/// declares it once, as the requirement of its `capabilities` slot (`registrations.rs`
/// `capabilities_slot`). `actor.create-inputs` publishes that slot as it stands, and acceptance
/// reads it back through [`demand`] — so a form filled with what the answer marks required is a
/// config that acceptance takes, and no second place knows which actors demand their grants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyDemand {
    /// Every declared policy, whatever else the config holds. The slot is mandatory.
    EveryDeclared,
    /// Only the policies whose effect an authored tool performs (`tool_executor`). The slot is
    /// optional: with `tools: {}` no grant is needed.
    AuthoredTools,
}

impl PolicyDemand {
    /// The requirement the `capabilities` slot carries for this demand. [`demand`] reads it back.
    pub(crate) fn requirement(self) -> crate::Required {
        match self {
            Self::EveryDeclared => crate::Required::Mandatory,
            Self::AuthoredTools => crate::Required::Optional {
                default: Value::object([] as [(&str, Value); 0])
                    .expect("the empty policy object has no keys"),
            },
        }
    }
}

/// The demand `actor_type`'s registration declares on its `capabilities` slot; `None` when it
/// has no such slot.
#[must_use]
pub fn demand(actor_type: ActorType) -> Option<PolicyDemand> {
    let slot = crate::registration(actor_type)
        .spec()
        .config()
        .top_level_slot(FIELD)?;
    Some(match slot.required() {
        crate::Required::Mandatory => PolicyDemand::EveryDeclared,
        _ => PolicyDemand::AuthoredTools,
    })
}

/// Whether `config` must carry `capability`'s policy. Under [`PolicyDemand::AuthoredTools`] the
/// tool registry describes possible effects and only authored tools require authority.
pub fn required_by_tools(actor_type: ActorType, capability: Capability, config: &Value) -> bool {
    if demand(actor_type) != Some(PolicyDemand::AuthoredTools) {
        return true;
    }
    let effect = match capability {
        Capability::FsRead => "file_read",
        Capability::FsWrite => "file_write",
        Capability::ProcessSpawn => "spawn",
        _ => return true,
    };
    config
        .as_object()
        .and_then(|root| root.get("tools"))
        .and_then(Value::as_object)
        .is_some_and(|tools| {
            tools.iter().any(|(_, tool)| {
                tool.as_object()
                    .and_then(|tool| tool.get("effect"))
                    .and_then(Value::as_str)
                    == Some(effect)
            })
        })
}

fn path(capability: Capability, key: &str) -> ConfigPath {
    ConfigPath::root()
        .join_key(FIELD)
        .join_key(name(capability).expect("every capability has a registered config name"))
        .join_key(key)
}

fn policy(config: &Value, capability: Capability) -> Option<&circular_core::ObjectValue> {
    config
        .as_object()?
        .get(FIELD)?
        .as_object()?
        .get(name(capability)?)?
        .as_object()
}

pub fn approval(config: &Value, capability: Capability) -> Result<bool, CreateInputAdmissionError> {
    let value = policy(config, capability)
        .and_then(|policy| policy.get("approval"))
        .ok_or_else(|| CreateInputAdmissionError::MissingMandatory(path(capability, "approval")))?;
    crate::approval_config::required(Some(value)).map_err(|_| {
        CreateInputAdmissionError::OutsideSpace {
            path: path(capability, "approval"),
            space: Some(crate::approval_config::approval_space()),
        }
    })
}

pub fn roots(
    config: &Value,
    capability: Capability,
) -> Result<PathScopes, CreateInputAdmissionError> {
    let value = policy(config, capability)
        .and_then(|policy| policy.get("roots"))
        .ok_or_else(|| CreateInputAdmissionError::MissingMandatory(path(capability, "roots")))?;
    let fail = || CreateInputAdmissionError::outside_space(path(capability, "roots"));
    let Value::Array(values) = value else {
        return Err(fail());
    };
    let paths = values
        .iter()
        .map(|value| {
            let text = value.as_str().ok_or_else(fail)?;
            if !std::path::Path::new(text).is_absolute() {
                return Err(fail());
            }
            NormalizedPath::new(text)
                .map(PathScope::new)
                .map_err(|_| fail())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PathScopes::new(paths))
}

pub fn validate(actor_type: ActorType, config: &Value) -> Result<(), CreateInputAdmissionError> {
    if let Some(value) = config.as_object().and_then(|root| root.get(FIELD)) {
        let Some(policies) = value.as_object() else {
            return Err(CreateInputAdmissionError::outside_space(
                ConfigPath::root().join_key(FIELD),
            ));
        };
        for (key, value) in policies.iter() {
            let Some((capability, _, _)) = NAMES.iter().find(|(_, name, _)| *name == key) else {
                return Err(CreateInputAdmissionError::UnknownField(format!(
                    "{FIELD}.{key}"
                )));
            };
            let Some(policy) = value.as_object() else {
                return Err(CreateInputAdmissionError::outside_space(
                    ConfigPath::root().join_key(FIELD).join_key(key),
                ));
            };
            for (field, _) in policy.iter() {
                if field != "approval" && !(field == "roots" && has_roots(*capability)) {
                    return Err(CreateInputAdmissionError::UnknownField(format!(
                        "{FIELD}.{key}.{field}"
                    )));
                }
            }
            approval(config, *capability)?;
            if has_roots(*capability) {
                roots(config, *capability)?;
            }
        }
    }
    for rule in crate::registration(actor_type).spec().requires() {
        let capability = rule.capability();
        if name(capability).is_some() && required_by_tools(actor_type, capability, config) {
            approval(config, capability)?;
            if has_roots(capability) {
                roots(config, capability)?;
            }
        }
    }
    Ok(())
}

