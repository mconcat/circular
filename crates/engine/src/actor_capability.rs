use circular_plan::ActorDecl;
use circular_runtime::{Capability, FoldedConfig, FsReadGrant, FsWriteGrant, PathScopes};

pub(crate) fn config(declaration: &ActorDecl) -> Result<circular_runtime::FoldedConfig, String> {
    crate::fold_config(
        *declaration.domain().actor_type(),
        declaration.domain().config(),
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn required(declaration: &ActorDecl, capability: Capability) -> bool {
    required_config(
        *declaration.domain().actor_type(),
        declaration.domain().config(),
        capability,
    )
}

pub(crate) fn required_config(
    actor_type: circular_core::ActorType,
    config: &circular_plan::Config,
    capability: Capability,
) -> bool {
    use circular_actors::capability_config::{PolicyDemand, demand};
    if demand(actor_type) != Some(PolicyDemand::AuthoredTools) {
        return true;
    }
    crate::fold_config(actor_type, config).is_ok_and(|folded| required_folded(&folded, capability))
}

pub(crate) fn required_folded(config: &FoldedConfig, capability: Capability) -> bool {
    circular_actors::capability_config::required_by_tools(
        config.actor_type(),
        capability,
        config.value(),
    )
}

pub(crate) fn roots(config: &FoldedConfig, capability: Capability) -> Result<PathScopes, String> {
    if !required_folded(config, capability) {
        return Ok(PathScopes::new([]));
    }
    circular_actors::capability_config::roots(config.value(), capability)
        .map_err(|error| error.to_string())
}

pub(crate) fn filesystem(config: &FoldedConfig) -> (FsReadGrant, FsWriteGrant) {
    let scope = |capability| roots(config, capability).unwrap_or_else(|_| PathScopes::new([]));
    (
        FsReadGrant::fs_read(scope(Capability::FsRead)),
        FsWriteGrant::fs_write(scope(Capability::FsWrite)),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use circular_core::{ActorType, Boundary, Ceilings, Value, encode};
    use circular_plan::{Config, ConfigValue, Name};

    fn capability_names(actor_type: ActorType) -> Option<&'static [&'static str]> {
        Some(match actor_type {
            ActorType::ToolExecutor => &["FsRead", "FsWrite", "ProcessSpawn"],
            ActorType::File | ActorType::FixtureTap => &["FsRead", "FsWrite"],
            ActorType::Listener => &["FsRead"],
            ActorType::Request => &["HttpFetch"],
            ActorType::Cli => &["ProcessSpawn"],
            ActorType::Notify => &["UserNotify"],
            _ => return None,
        })
    }

    fn fixture_roots(actor_type: ActorType, config: &Value) -> Vec<String> {
        let parent = |path: &str| {
            std::path::Path::new(path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| parent.to_string_lossy().into_owned())
        };
        let object = config.as_object();
        let mut roots = Vec::new();
        if let Some(path) = object
            .and_then(|root| root.get("path"))
            .and_then(Value::as_str)
        {
            roots.extend(parent(path));
        }
        if let Some(tools) = object
            .and_then(|root| root.get("tools"))
            .and_then(Value::as_object)
        {
            for (_, tool) in tools.iter() {
                if let Some(path) = tool
                    .as_object()
                    .and_then(|tool| tool.get("path"))
                    .and_then(Value::as_str)
                {
                    roots.extend(parent(path));
                }
            }
        }
        if let Some(glob) = object
            .and_then(|root| root.get("source"))
            .and_then(Value::as_object)
            .and_then(|source| source.get("value"))
            .and_then(Value::as_object)
            .and_then(|value| value.get("glob"))
            .and_then(Value::as_str)
        {
            roots.extend(parent(glob));
        }
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            roots.push(format!("/test-only/{}", actor_type.as_str()));
        }
        roots
    }

    fn fixture_policies(actor_type: ActorType, config: &Value) -> Option<Value> {
        let names = capability_names(actor_type)?;
        let roots = fixture_roots(actor_type, config);
        Some(
            Value::object(names.iter().map(|name| {
                let mut fields = vec![("approval", Value::string("none"))];
                if matches!(*name, "FsRead" | "FsWrite") {
                    fields.push((
                        "roots",
                        Value::array(roots.iter().map(|root| Value::string(root.clone()))),
                    ));
                }
                (*name, Value::object(fields).unwrap())
            }))
            .unwrap(),
        )
    }

    pub(crate) fn fixture_value(actor_type: &str, config: Value) -> Value {
        let kind = ActorType::ALL
            .into_iter()
            .find(|kind| kind.as_str() == actor_type)
            .expect("fixture actor kind");
        let Some(policies) = fixture_policies(kind, &config) else {
            return config;
        };
        let mut entries = match config {
            Value::Object(object) => object.into_map(),
            Value::Null => Default::default(),
            _ => return config,
        };
        entries.entry("capabilities".into()).or_insert(policies);
        Value::object(entries).unwrap()
    }

    pub(crate) fn fixture_config(actor_type: ActorType, config: Config) -> Config {
        if capability_names(actor_type).is_none() {
            return config;
        }
        let mut entries = config.record().entries().to_vec();
        if entries
            .iter()
            .any(|(key, _)| key.as_str() == "capabilities")
        {
            return config;
        }
        let folded = crate::fold_config(actor_type, &config);
        let declared = folded
            .as_ref()
            .map_or(Value::Null, |folded| folded.value().clone());
        let policies = fixture_policies(actor_type, &declared).expect("fixture capability names");
        fn value(input: Value) -> ConfigValue {
            match input {
                Value::Object(object) => ConfigValue::Record(
                    circular_plan::ConfigRecord::try_new(
                        object
                            .into_map()
                            .into_iter()
                            .map(|(key, item)| (Name::from_normalized(key), value(item)))
                            .collect(),
                    )
                    .unwrap(),
                ),
                Value::Array(items) => ConfigValue::List(items.into_iter().map(value).collect()),
                scalar => ConfigValue::Scalar {
                    tag: Name::from_normalized(crate::activation_config::CANONICAL_VALUE_TAG),
                    bytes: encode(&scalar, Ceilings::for_boundary(Boundary::Config))
                        .unwrap()
                        .into_boxed_slice(),
                },
            }
        }
        entries.push((Name::from_normalized("capabilities"), value(policies)));
        Config::try_new(entries).unwrap()
    }
}
