use circular_actors::{ProductActor, ProductPayload, ToolExecutorState};
use circular_plan::ActorDecl;
use circular_runtime::{ActorTypes, Capability, EffectCtor};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ApprovalDemand {
    folded: Result<Folded, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Folded {
    root: Result<bool, String>,
    tools: Option<BTreeMap<String, Result<bool, String>>>,
    capabilities: Vec<(Capability, Result<bool, String>)>,
}

impl ApprovalDemand {
    pub(crate) fn fold(declaration: &ActorDecl) -> Self {
        Self::from_config(crate::actor_capability::config(declaration).as_ref())
    }

    pub(crate) fn from_config(config: Result<&circular_runtime::FoldedConfig, &String>) -> Self {
        Self {
            folded: config.map_err(Clone::clone).map(Self::fold_inner),
        }
    }

    fn fold_inner(folded: &circular_runtime::FoldedConfig) -> Folded {
        let root = folded.value().as_object();
        Folded {
            root: circular_actors::approval_config::required(
                root.and_then(|r| r.get(circular_actors::approval_config::APPROVAL_FIELD)),
            )
            .map_err(str::to_owned),
            tools: root
                .and_then(|r| r.get("tools"))
                .and_then(|v| v.as_object())
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|(name, value)| {
                            let tool = value.as_object()?;
                            Some((
                                name.to_owned(),
                                circular_actors::approval_config::required(
                                    tool.get(circular_actors::approval_config::APPROVAL_FIELD),
                                )
                                .map_err(str::to_owned),
                            ))
                        })
                        .collect()
                }),
            capabilities: circular_actors::capability_config::NAMES
                .into_iter()
                .map(|(capability, _, _)| {
                    (
                        capability,
                        circular_actors::capability_config::approval(folded.value(), capability)
                            .map_err(|error| error.to_string()),
                    )
                })
                .collect(),
        }
    }
}

impl Folded {
    fn capability(&self, capability: Capability) -> Result<bool, String> {
        self.capabilities
            .iter()
            .find_map(|(kind, demand)| (*kind == capability).then(|| demand.clone()))
            .unwrap_or(Ok(false))
    }
}

pub(crate) fn required<T>(
    demand: &ApprovalDemand,
    actor: &ProductActor<T>,
    effect: EffectCtor,
) -> Result<bool, String>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: circular_runtime::InstanceAuthorityBearer,
{
    if matches!(
        effect,
        EffectCtor::Schedule | EffectCtor::MutateInstance | EffectCtor::RequestApproval
    ) {
        return Ok(false);
    }
    let folded = demand.folded.as_ref().map_err(Clone::clone)?;
    let effect_required = if let ProductActor::ToolExecutor(actor) = actor {
        let ToolExecutorState::Pending { call, .. } = actor.state() else {
            return Err("tool effect has no selected call".into());
        };
        folded
            .tools
            .as_ref()
            .and_then(|tools| tools.get(call.tool().as_str()))
            .ok_or("selected tool is absent from the event revision")?
            .clone()?
    } else {
        folded.root.clone()?
    };
    let capability_required = match circular_actors::capability_config::for_effect(effect) {
        Some(capability) => folded.capability(capability)?,
        None => false,
    };
    Ok(effect_required || capability_required)
}
