//! Executable authority is read from the declaration of the causal revision.
//! The daemon allowlist can restrict this set; it cannot add actor authority.
use circular_actors::ActorType;
#[cfg(test)]
use circular_plan::ActorDecl;
use circular_runtime::{FoldedConfig, ProcessTargets, ProgramName};

#[cfg(test)]
pub(crate) fn declared_targets(declaration: &ActorDecl) -> ProcessTargets {
    if *declaration.domain().actor_type() != ActorType::ToolExecutor {
        return ProcessTargets::default();
    }
    let Ok(config) = crate::fold_config(ActorType::ToolExecutor, declaration.domain().config())
    else {
        return ProcessTargets::default();
    };
    declared_targets_folded(&config)
}

pub(crate) fn declared_targets_folded(config: &FoldedConfig) -> ProcessTargets {
    if config.actor_type() != ActorType::ToolExecutor {
        return ProcessTargets::default();
    }
    let Some(tools) = config
        .value()
        .as_object()
        .and_then(|root| root.get("tools"))
        .and_then(|value| value.as_object())
    else {
        return ProcessTargets::default();
    };
    ProcessTargets::exact(tools.iter().filter_map(|(_, value)| {
        let template = value.as_object()?;
        if template.get("effect")?.as_str()? != "spawn" {
            return None;
        }
        let program = template.get("program")?.as_str()?;
        std::path::Path::new(program)
            .is_absolute()
            .then(|| ProgramName::from_normalized(program))
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use circular_core::Value;
    use circular_plan::{ActorDomain, ActorFlags, Config, ConfigValue, Name};

    pub(crate) fn declaration(program: &str) -> ActorDecl {
        let tools = Value::object([(
            "run".to_owned(),
            Value::object([
                ("effect".to_owned(), Value::string("spawn")),
                ("program".to_owned(), Value::string(program)),
                ("arguments".to_owned(), Value::array([])),
            ])
            .unwrap(),
        )])
        .unwrap();
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
                    tag: Name::from_normalized(crate::CANONICAL_VALUE_TAG),
                    bytes: circular_core::encode(
                        &scalar,
                        circular_core::Ceilings::for_boundary(circular_core::Boundary::Config),
                    )
                    .unwrap()
                    .into_boxed_slice(),
                },
            }
        }
        let config = Config::try_new(vec![(Name::from_normalized("tools"), value(tools))]).unwrap();
        ActorDecl::new(
            ActorDomain::new(ActorType::ToolExecutor, config),
            ActorFlags::default(),
        )
    }

    #[test]
    fn process_authority_is_only_the_exact_declared_absolute_program() {
        let targets = declared_targets(&declaration("/usr/bin/env"));
        assert_eq!(
            targets.iter().map(ProgramName::as_str).collect::<Vec<_>>(),
            ["/usr/bin/env"]
        );
        assert!(!targets.allows(&ProgramName::from_normalized("/bin/sh")));
        assert!(declared_targets(&declaration("env")).is_empty());
        assert!(
            declared_targets(&ActorDecl::new(
                ActorDomain::new(ActorType::ToolExecutor, Config::default()),
                ActorFlags::default()
            ))
            .is_empty()
        );
    }
}
