
use crate::{InstanceKey, Name, NamedActorId, ScopeId, ScopeRole, ScopeRoleTable, ScopeSeg};
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstanceBinding {
    container: ScopeId,
    template: Name,
    key: InstanceKey,
}

impl InstanceBinding {
    #[must_use]
    pub const fn container(&self) -> &ScopeId {
        &self.container
    }

    #[must_use]
    pub const fn template(&self) -> &Name {
        &self.template
    }

    #[must_use]
    pub const fn key(&self) -> &InstanceKey {
        &self.key
    }

    #[must_use]
    pub fn segment(&self) -> ScopeSeg {
        ScopeSeg::Instance {
            of: self.template.clone(),
            key: self.key.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedScope {
    declared: ScopeId,
    instances: Vec<InstanceBinding>,
}

impl AdmittedScope {
    #[must_use]
    pub const fn declared(&self) -> &ScopeId {
        &self.declared
    }

    #[must_use]
    pub fn instances(&self) -> &[InstanceBinding] {
        &self.instances
    }

    #[must_use]
    pub fn is_instantiated(&self) -> bool {
        !self.instances.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScopeAdmissionError {
    UnknownScope { container: ScopeId, name: Name },
    InstanceOfNonTemplate { container: ScopeId, name: Name },
    TemplateAddressedAsChild { container: ScopeId, name: Name },
}

impl ScopeAdmissionError {
    #[must_use]
    pub const fn container(&self) -> &ScopeId {
        match self {
            Self::UnknownScope { container, .. }
            | Self::InstanceOfNonTemplate { container, .. }
            | Self::TemplateAddressedAsChild { container, .. } => container,
        }
    }

    #[must_use]
    pub const fn name(&self) -> &Name {
        match self {
            Self::UnknownScope { name, .. }
            | Self::InstanceOfNonTemplate { name, .. }
            | Self::TemplateAddressedAsChild { name, .. } => name,
        }
    }
}

impl fmt::Display for ScopeAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::UnknownScope { .. } => "no child scope with that name",
            Self::InstanceOfNonTemplate { .. } => {
                "scope is not a template and cannot create instances"
            }
            Self::TemplateAddressedAsChild { .. } => {
                "template scopes can be instantiated only through instance segments"
            }
        };
        write!(
            formatter,
            "{} child `{}`: {reason}",
            self.container(),
            self.name()
        )
    }
}

impl std::error::Error for ScopeAdmissionError {}

pub fn admit_runtime_scope(
    roles: &ScopeRoleTable,
    address: &ScopeId,
) -> Result<AdmittedScope, ScopeAdmissionError> {
    walk(roles, address)
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AdmittedTemplate {
    container: ScopeId,
    template: Name,
}

impl AdmittedTemplate {
    #[must_use]
    pub const fn container(&self) -> &ScopeId {
        &self.container
    }

    #[must_use]
    pub const fn template(&self) -> &Name {
        &self.template
    }

    #[must_use]
    pub fn segment(&self, key: InstanceKey) -> ScopeSeg {
        ScopeSeg::Instance {
            of: self.template.clone(),
            key,
        }
    }

    #[must_use]
    pub fn declared(&self) -> ScopeId {
        let mut segments = self.container.segments().to_vec();
        segments.push(ScopeSeg::Child(self.template.clone()));
        ScopeId::from_segments(segments).expect("the container already kept the depth ceiling")
    }

    pub fn cell_actor(
        &self,
        template_actor: &NamedActorId,
        key: &InstanceKey,
    ) -> Result<NamedActorId, CellDerivationError> {
        let declared = self.declared();
        if !declared.is_ancestor_of(template_actor.scope()) {
            return Err(CellDerivationError::NotUnderTemplate {
                template: declared,
                actor: template_actor.clone(),
            });
        }
        let depth = declared.depth();
        let mut segments = self.container.segments().to_vec();
        segments.push(self.segment(key.clone()));
        segments.extend_from_slice(&template_actor.scope().segments()[depth..]);
        let scope = ScopeId::from_segments(segments)
            .expect("substitution does not change the segment count, so depth is preserved");
        Ok(NamedActorId::new(scope, template_actor.name().clone()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CellDerivationError {
    NotUnderTemplate {
        template: ScopeId,
        actor: NamedActorId,
    },
}

impl fmt::Display for CellDerivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUnderTemplate { template, actor } => {
                write!(
                    formatter,
                    "{} actor `{}` is not under template {template}",
                    actor.scope(),
                    actor.name()
                )
            }
        }
    }
}

impl std::error::Error for CellDerivationError {}

pub fn admit_template(
    roles: &ScopeRoleTable,
    container: &ScopeId,
    template: &Name,
) -> Result<AdmittedTemplate, ScopeAdmissionError> {
    let admitted = admit_runtime_scope(roles, container)?;
    let child = admitted
        .declared()
        .clone()
        .append_segment(ScopeSeg::Child(template.clone()))
        .map_err(|_| ScopeAdmissionError::UnknownScope {
            container: container.clone(),
            name: template.clone(),
        })?;
    match roles.role_at(&child) {
        Some(ScopeRole::Template) => Ok(AdmittedTemplate {
            container: container.clone(),
            template: template.clone(),
        }),
        Some(ScopeRole::Concrete) => Err(ScopeAdmissionError::InstanceOfNonTemplate {
            container: container.clone(),
            name: template.clone(),
        }),
        None => Err(ScopeAdmissionError::UnknownScope {
            container: container.clone(),
            name: template.clone(),
        }),
    }
}

fn walk(roles: &ScopeRoleTable, address: &ScopeId) -> Result<AdmittedScope, ScopeAdmissionError> {
    let mut declared = ScopeId::root();
    let mut running = ScopeId::root();
    let mut instances = Vec::new();

    for segment in address.segments() {
        let (name, key) = match segment {
            ScopeSeg::Child(name) => (name, None),
            ScopeSeg::Instance { of, key } => (of, Some(key)),
        };
        let child_declared = declared
            .clone()
            .append_segment(ScopeSeg::Child(name.clone()))
            .map_err(|_| ScopeAdmissionError::UnknownScope {
                container: declared.clone(),
                name: name.clone(),
            })?;
        let child_role =
            roles
                .role_at(&child_declared)
                .ok_or_else(|| ScopeAdmissionError::UnknownScope {
                    container: declared.clone(),
                    name: name.clone(),
                })?;
        match (child_role, key) {
            (ScopeRole::Concrete, None) => {}
            (ScopeRole::Template, Some(key)) => instances.push(InstanceBinding {
                container: running.clone(),
                template: name.clone(),
                key: key.clone(),
            }),
            (ScopeRole::Concrete, Some(_)) => {
                return Err(ScopeAdmissionError::InstanceOfNonTemplate {
                    container: declared.clone(),
                    name: name.clone(),
                });
            }
            (ScopeRole::Template, None) => {
                return Err(ScopeAdmissionError::TemplateAddressedAsChild {
                    container: declared.clone(),
                    name: name.clone(),
                });
            }
        }
        declared = child_declared;
        running = running
            .append_segment(segment.clone())
            .expect("the address already kept the depth ceiling");
    }

    Ok(AdmittedScope {
        declared,
        instances,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InstanceScalar;

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn key(value: &str) -> InstanceKey {
        InstanceKey::Scalar(InstanceScalar::normalized_text(value))
    }

    fn child(value: &str) -> ScopeSeg {
        ScopeSeg::Child(name(value))
    }

    fn instance(of: &str, at: &str) -> ScopeSeg {
        ScopeSeg::Instance {
            of: name(of),
            key: key(at),
        }
    }

    fn address(segments: Vec<ScopeSeg>) -> ScopeId {
        ScopeId::from_segments(segments).expect("the test address is shallow")
    }

    fn roles(entries: &[(&[ScopeSeg], ScopeRole)]) -> ScopeRoleTable {
        let mut table = ScopeRoleTable::new();
        for (segments, role) in entries {
            table.declare(address(segments.to_vec()), *role);
        }
        table
    }

    fn fleet_roles() -> ScopeRoleTable {
        roles(&[
            (&[child("fleet")], ScopeRole::Concrete),
            (&[child("fleet"), child("cell")], ScopeRole::Template),
            (&[child("fleet"), child("plain")], ScopeRole::Concrete),
        ])
    }

    #[test]
    fn instance_of_a_concrete_scope_is_rejected() {
        let plan = fleet_roles();
        let error = admit_runtime_scope(
            &plan,
            &address(vec![child("fleet"), instance("plain", "k")]),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ScopeAdmissionError::InstanceOfNonTemplate {
                container: address(vec![child("fleet")]),
                name: name("plain"),
            }
        );
    }

    #[test]
    fn instance_of_an_undeclared_name_is_rejected() {
        let plan = fleet_roles();
        let error = admit_runtime_scope(
            &plan,
            &address(vec![child("fleet"), instance("ghost", "k")]),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ScopeAdmissionError::UnknownScope {
                container: address(vec![child("fleet")]),
                name: name("ghost"),
            }
        );
    }

    #[test]
    fn instance_of_a_template_carries_the_authority_coordinates() {
        let plan = fleet_roles();
        let admitted = admit_runtime_scope(
            &plan,
            &address(vec![child("fleet"), instance("cell", "s1")]),
        )
        .unwrap();

        assert_eq!(
            admitted.declared(),
            &address(vec![child("fleet"), child("cell")])
        );
        assert!(admitted.is_instantiated());
        assert_eq!(admitted.instances().len(), 1);
        let binding = &admitted.instances()[0];
        assert_eq!(binding.container(), &address(vec![child("fleet")]));
        assert_eq!(binding.template(), &name("cell"));
        assert_eq!(binding.key(), &key("s1"));
        assert_eq!(binding.segment(), instance("cell", "s1"));
        assert_eq!(plan.role_at(admitted.declared()), Some(ScopeRole::Template));
    }

    #[test]
    fn every_key_of_one_template_admits_to_the_same_declaration() {
        let plan = fleet_roles();
        let left =
            admit_runtime_scope(&plan, &address(vec![child("fleet"), instance("cell", "a")]))
                .unwrap();
        let right =
            admit_runtime_scope(&plan, &address(vec![child("fleet"), instance("cell", "b")]))
                .unwrap();
        assert_eq!(left.declared(), right.declared());
        assert_eq!(plan.role_at(left.declared()), Some(ScopeRole::Template));
        assert_ne!(left.instances()[0].key(), right.instances()[0].key());
    }

    #[test]
    fn a_template_is_not_a_runtime_scope() {
        let plan = fleet_roles();
        let template = address(vec![child("fleet"), child("cell")]);

        assert_eq!(
            admit_runtime_scope(&plan, &template).unwrap_err(),
            ScopeAdmissionError::TemplateAddressedAsChild {
                container: address(vec![child("fleet")]),
                name: name("cell"),
            }
        );
    }

    #[test]
    fn the_root_address_is_admitted() {
        let plan = fleet_roles();
        let admitted = admit_runtime_scope(&plan, &ScopeId::root()).unwrap();
        assert_eq!(admitted.declared(), &ScopeId::root());
        assert!(!admitted.is_instantiated());
        assert_eq!(plan.role_at(admitted.declared()), Some(ScopeRole::Concrete));
    }

    #[test]
    fn nested_templates_bind_from_the_root_downward() {
        let plan = roles(&[
            (&[child("outer")], ScopeRole::Template),
            (&[child("outer"), child("inner")], ScopeRole::Template),
        ]);

        let admitted = admit_runtime_scope(
            &plan,
            &address(vec![instance("outer", "o"), instance("inner", "i")]),
        )
        .unwrap();

        let bindings = admitted.instances();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].container(), &ScopeId::root());
        assert_eq!(bindings[0].template(), &name("outer"));
        assert_eq!(
            bindings[1].container(),
            &address(vec![instance("outer", "o")])
        );
        assert_eq!(bindings[1].template(), &name("inner"));

        let other = admit_runtime_scope(
            &plan,
            &address(vec![instance("outer", "other"), instance("inner", "i")]),
        )
        .unwrap();
        assert_ne!(
            other.instances()[1].container(),
            bindings[1].container(),
            "different cells give different coordinates for the inner instance set"
        );
    }

    #[test]
    fn admission_stops_at_the_first_unresolved_segment() {
        let plan = fleet_roles();
        let error = admit_runtime_scope(
            &plan,
            &address(vec![
                child("fleet"),
                instance("plain", "k"),
                child("deeper"),
            ]),
        )
        .unwrap_err();
        assert_eq!(error.container(), &address(vec![child("fleet")]));
        assert_eq!(error.name(), &name("plain"));
    }

    #[test]
    fn a_scope_below_an_instance_resolves_in_the_template() {
        let nested = roles(&[
            (&[child("cell")], ScopeRole::Template),
            (&[child("cell"), child("part")], ScopeRole::Concrete),
        ]);

        let admitted = admit_runtime_scope(
            &nested,
            &address(vec![instance("cell", "1"), child("part")]),
        )
        .unwrap();
        assert_eq!(
            admitted.declared(),
            &address(vec![child("cell"), child("part")])
        );
        assert_eq!(admitted.instances().len(), 1);
        assert_eq!(
            nested.role_at(admitted.declared()),
            Some(ScopeRole::Concrete)
        );
    }

    #[test]
    fn a_template_is_admitted_as_an_authority_coordinate() {
        let plan = fleet_roles();
        let admitted =
            admit_template(&plan, &address(vec![child("fleet")]), &name("cell")).unwrap();
        assert_eq!(admitted.container(), &address(vec![child("fleet")]));
        assert_eq!(admitted.template(), &name("cell"));
        assert_eq!(admitted.segment(key("s1")), instance("cell", "s1"));
    }

    #[test]
    fn a_concrete_child_is_not_an_authority_coordinate() {
        let plan = fleet_roles();
        assert_eq!(
            admit_template(&plan, &address(vec![child("fleet")]), &name("plain")).unwrap_err(),
            ScopeAdmissionError::InstanceOfNonTemplate {
                container: address(vec![child("fleet")]),
                name: name("plain"),
            }
        );
        assert_eq!(
            admit_template(&plan, &address(vec![child("fleet")]), &name("ghost")).unwrap_err(),
            ScopeAdmissionError::UnknownScope {
                container: address(vec![child("fleet")]),
                name: name("ghost"),
            }
        );
    }

    #[test]
    fn an_authority_coordinate_under_a_cell_names_that_cell() {
        let plan = roles(&[
            (&[child("outer")], ScopeRole::Template),
            (&[child("outer"), child("inner")], ScopeRole::Template),
        ]);

        let left = admit_template(
            &plan,
            &address(vec![instance("outer", "a")]),
            &name("inner"),
        )
        .unwrap();
        let right = admit_template(
            &plan,
            &address(vec![instance("outer", "b")]),
            &name("inner"),
        )
        .unwrap();
        assert_ne!(left, right);
        assert_eq!(left.template(), right.template());

        assert_eq!(
            admit_template(&plan, &address(vec![child("outer")]), &name("inner")).unwrap_err(),
            ScopeAdmissionError::TemplateAddressedAsChild {
                container: ScopeId::root(),
                name: name("outer"),
            }
        );
    }
}
