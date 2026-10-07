//! Daemon-owned authoring capability projection.
//!
//! Registry requirements and product-profile decisions are joined here, while
//! the client sees only a closed typed carrier. Config bodies, display labels,
//! and actor-type-specific policy never cross the query boundary.

use std::collections::BTreeMap;

use circular_core::{ActorType, Boundary, Ceilings, Value, encode};
use circular_plan::NamedActorId;
use circular_protocol::authoring_snapshot::scope_identity_value;
use circular_protocol::declaration_payload::{PlanActorKey, ScopeSegment};
use engine::execution_profile::{
    ProductCapabilityAccess, ProductCapabilityAuthority, ProductCapabilityCondition,
    ProductCapabilityDecision, ProductCapabilitySubject, ProductExecutionProfile,
};

use engine::authoring_assembly::fold::EpochCandidate;

pub(crate) const AUTHORING_ACTOR_ACCESS_QUERY: &str = "authoring.actor-access";

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthoringActorAccessFact {
    actor: PlanActorKey,
    requirements: Vec<ProductCapabilityAccess>,
}

pub(crate) fn authoring_access_items(
    current: &EpochCandidate,
    execution: &ProductExecutionProfile,
    target: &[ScopeSegment],
) -> Result<Vec<Value>, String> {
    let plan = current.assemble().map_err(|error| error.to_string())?;
    let decisions = execution
        .actor_capability_access(&plan)
        .into_iter()
        .map(|access| (access.actor, access.requirements))
        .collect::<BTreeMap<_, _>>();
    let mut facts = Vec::new();
    for (actor, declaration) in current.tables().actors() {
        if !actor.scope.starts_with(target) {
            continue;
        }
        let actor_type = ActorType::from_str(&declaration.actor_type)
            .ok_or_else(|| format!("unknown authored actor type {:?}", declaration.actor_type))?;
        let key = plan_actor_id(actor)?;
        let requirements = match decisions.get(&key) {
            Some(requirements) => requirements.clone(),
            None if circular_actors::registration(actor_type)
                .spec()
                .requires()
                .is_empty() =>
            {
                Vec::new()
            }
            None => {
                return Err(format!(
                    "product access projection omitted capability-bearing authored actor {:?}",
                    actor.local
                ));
            }
        };
        facts.push(AuthoringActorAccessFact {
            actor: actor.clone(),
            requirements,
        });
    }
    access_items(facts)
}

fn access_items(facts: Vec<AuthoringActorAccessFact>) -> Result<Vec<Value>, String> {
    let items = facts
        .into_iter()
        .map(|fact| {
            let actor = Value::object([
                ("local", Value::String(fact.actor.local.as_str().to_owned())),
                ("scope", scope_identity_value(&fact.actor.scope)),
            ])
            .map_err(|error| format!("authoring access actor identity: {error:?}"))?;
            let requirements = fact
                .requirements
                .into_iter()
                .map(access_value)
                .collect::<Result<Vec<_>, _>>()?;
            Value::object([
                ("actor", Value::array([Value::Int(1), actor])),
                ("requirements", Value::Array(requirements)),
            ])
            .map_err(|error| format!("authoring access item: {error:?}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut encoded_items = items
        .into_iter()
        .map(|item| {
            let encoded = encode(&item, Ceilings::for_boundary(Boundary::Wire))
                .map_err(|error| format!("authoring access item does not encode: {error:?}"))?;
            Ok((encoded, item))
        })
        .collect::<Result<Vec<_>, String>>()?;
    encoded_items.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok(encoded_items.into_iter().map(|(_, item)| item).collect())
}

fn access_value(access: ProductCapabilityAccess) -> Result<Value, String> {
    let condition = match access.condition {
        ProductCapabilityCondition::Always => Value::Int(1),
        ProductCapabilityCondition::ConfigPresent => Value::Int(2),
        ProductCapabilityCondition::ConfigEquals => Value::Int(3),
    };
    let authority = match access.authority {
        ProductCapabilityAuthority::ProductExecutionProfile => Value::Int(1),
        ProductCapabilityAuthority::UnconditionalRequirementResolver => Value::Int(2),
    };
    let decision = match access.decision {
        ProductCapabilityDecision::Allowed => Value::array([Value::Int(1)]),
        ProductCapabilityDecision::Denied { reason } if !reason.is_empty() => {
            Value::array([Value::Int(2), Value::String(reason)])
        }
        ProductCapabilityDecision::Denied { .. } => {
            return Err("denied capability access has an empty reason".to_owned());
        }
    };
    let subject = match access.subject {
        None => Value::Null,
        Some(ProductCapabilitySubject::AgentHarness(harness)) => {
            Value::array([Value::Int(1), Value::String(harness.as_str().to_owned())])
        }
    };
    Value::object([
        ("authority", authority),
        (
            "capability",
            Value::String(capability_name(access.capability).to_owned()),
        ),
        ("condition", condition),
        ("decision", decision),
        ("rule_index", Value::UInt(access.rule_index)),
        ("subject", subject),
    ])
    .map_err(|error| format!("authoring access requirement: {error:?}"))
}

fn capability_name(capability: circular_runtime::Capability) -> &'static str {
    use circular_runtime::Capability;
    match capability {
        Capability::HttpFetch => "http_fetch",
        Capability::NetworkOutbound => "network_outbound",
        Capability::NetworkListen => "network_listen",
        Capability::FsRead => "fs_read",
        Capability::FsWrite => "fs_write",
        Capability::ProcessSpawn => "process_spawn",
        Capability::UserNotify => "user_notify",
        Capability::PeerConnect => "peer_connect",
        Capability::ApprovalRequest => "approval_request",
        Capability::CivilTime => "civil_time",
        Capability::HostedDelegation => "hosted_delegation",
        Capability::AgentHarness => "agent_harness",
        Capability::ModelProvider => "model_provider",
        Capability::PeerDiscover => "peer_discover",
        Capability::PeerSend => "peer_send",
        Capability::PeerAdvertise => "peer_advertise",
        Capability::PeerReceive => "peer_receive",
    }
}

fn plan_actor_id(actor: &PlanActorKey) -> Result<NamedActorId, String> {
    circular_runtime::product_identity::named_actor_from_wire(actor)
        .map_err(|error| format!("authored actor scope is too deep: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::execution_profile::{
        CONDITIONAL_CAPABILITY_REQUIREMENT_REASON, ProductCapabilityAccess,
    };

    fn key(local: &str) -> PlanActorKey {
        PlanActorKey {
            scope: Vec::new(),
            local: circular_protocol::declaration_payload::AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        }
    }

    #[test]
    fn empty_allowed_and_resolver_denied_rows_encode_without_opaque_config() {
        let facts = vec![
            AuthoringActorAccessFact {
                actor: key("empty"),
                requirements: Vec::new(),
            },
            AuthoringActorAccessFact {
                actor: key("conditional"),
                requirements: vec![ProductCapabilityAccess {
                    rule_index: 0,
                    capability: circular_runtime::Capability::PeerConnect,
                    condition: ProductCapabilityCondition::ConfigEquals,
                    decision: ProductCapabilityDecision::Denied {
                        reason: CONDITIONAL_CAPABILITY_REQUIREMENT_REASON.to_owned(),
                    },
                    authority: ProductCapabilityAuthority::UnconditionalRequirementResolver,
                    subject: None,
                }],
            },
        ];
        let items = access_items(facts).expect("access items");
        assert_eq!(items.len(), 2);
        let encoded = items
            .iter()
            .map(|item| encode(item, Ceilings::for_boundary(Boundary::Wire)).expect("encodes"))
            .collect::<Vec<_>>();
        assert!(encoded.windows(2).all(|pair| pair[0] < pair[1]));
        for item in &items {
            let Value::Object(fields) = item else {
                panic!("access item is an object")
            };
            assert_eq!(fields.keys().collect::<Vec<_>>(), ["actor", "requirements"]);
            let Value::Array(requirements) = fields.get("requirements").expect("requirements")
            else {
                panic!("requirements is an array")
            };
            for requirement in requirements {
                let Value::Object(requirement) = requirement else {
                    panic!("requirement is an object")
                };
                assert_eq!(
                    requirement.keys().collect::<Vec<_>>(),
                    [
                        "authority",
                        "capability",
                        "condition",
                        "decision",
                        "rule_index",
                        "subject",
                    ]
                );
            }
        }
    }

    #[test]
    fn all_runtime_capabilities_have_one_closed_wire_name() {
        let names = circular_runtime::Capability::ALL.map(capability_name);
        let unique = names.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), circular_runtime::Capability::COUNT);
    }
}
