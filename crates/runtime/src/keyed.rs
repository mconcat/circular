
use crate::{AdmittedTemplate, CellDerivationError, Endpoint, InstanceKey, NamedActorId};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyedEmission<E> {
    key: InstanceKey,
    event: E,
}

impl<E> KeyedEmission<E> {
    pub const fn new(key: InstanceKey, event: E) -> Self {
        Self { key, event }
    }

    #[must_use]
    pub const fn key(&self) -> &InstanceKey {
        &self.key
    }

    #[must_use]
    pub const fn event(&self) -> &E {
        &self.event
    }

    #[must_use]
    pub fn into_event(self) -> E {
        self.event
    }

    pub fn map<F, T>(self, transform: F) -> KeyedEmission<T>
    where
        F: FnOnce(E) -> T,
    {
        KeyedEmission {
            key: self.key,
            event: transform(self.event),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyedInlet {
    template: AdmittedTemplate,
    receiver: Endpoint,
}

impl KeyedInlet {
    pub fn new(
        template: AdmittedTemplate,
        receiver: Endpoint,
    ) -> Result<Self, CellDerivationError> {
        template.cell_actor(receiver.actor(), &probe_key())?;
        Ok(Self { template, receiver })
    }

    #[must_use]
    pub const fn template(&self) -> &AdmittedTemplate {
        &self.template
    }

    #[must_use]
    pub const fn receiver(&self) -> &Endpoint {
        &self.receiver
    }

    #[must_use]
    pub fn route<E>(&self, emission: KeyedEmission<E>) -> KeyedDelivery<E> {
        let actor = self
            .template
            .cell_actor(self.receiver.actor(), emission.key())
            .expect("the derivation was already confirmed to hold at binding time");
        KeyedDelivery {
            target: Endpoint::new(actor, self.receiver.port().clone()),
            key: emission.key,
            event: emission.event,
        }
    }

    #[must_use]
    pub fn cell_receiver(&self, key: &InstanceKey) -> NamedActorId {
        self.template
            .cell_actor(self.receiver.actor(), key)
            .expect("the derivation was already confirmed to hold at binding time")
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyedDelivery<E> {
    target: Endpoint,
    key: InstanceKey,
    event: E,
}

impl<E> KeyedDelivery<E> {
    #[must_use]
    pub const fn target(&self) -> &Endpoint {
        &self.target
    }

    #[must_use]
    pub const fn key(&self) -> &InstanceKey {
        &self.key
    }

    #[must_use]
    pub fn into_event(self) -> E {
        self.event
    }
}

fn probe_key() -> InstanceKey {
    InstanceKey::Scalar(crate::InstanceScalar::Bool(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActorArrivals, ArrivalOrigin, ArrivalStep, EdgeId, IngressEdges, Name, RecordStep, ScopeId,
        ScopeSeg,
    };
    use circular_core::{
        BaseShape, GroundShape, Payload, PortId, ProducerIdentity, Sequence, Shape, Stamp, Tick,
    };
    use circular_plan::{InstanceScalar, ScopeRole, admit_template};

    struct Opaque;
    fn fleet_roles() -> circular_plan::ScopeRoleTable {
        let mut table = circular_plan::ScopeRoleTable::new();
        let at = |segments: &[&str]| {
            circular_plan::ScopeId::from_segments(
                segments
                    .iter()
                    .map(|s| {
                        circular_plan::ScopeSeg::Child(circular_plan::Name::from_normalized(*s))
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        };
        table.declare(at(&["fleet"]), circular_plan::ScopeRole::Concrete);
        table.declare(at(&["fleet", "cell"]), circular_plan::ScopeRole::Template);
        table
    }

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn key(value: &str) -> InstanceKey {
        InstanceKey::Scalar(InstanceScalar::normalized_text(value))
    }

    fn port(value: &str) -> PortId {
        PortId::try_new(value).unwrap()
    }

    fn fleet_scope() -> ScopeId {
        ScopeId::from_segments(vec![ScopeSeg::Child(name("fleet"))]).unwrap()
    }

    fn intake() -> NamedActorId {
        NamedActorId::new(
            ScopeId::from_segments(vec![
                ScopeSeg::Child(name("fleet")),
                ScopeSeg::Child(name("cell")),
            ])
            .unwrap(),
            name("intake"),
        )
    }

    fn inlet(plan: &circular_plan::ScopeRoleTable) -> KeyedInlet {
        let template = admit_template(&fleet_roles(), &fleet_scope(), &name("cell")).unwrap();
        KeyedInlet::new(template, Endpoint::new(intake(), port("event"))).unwrap()
    }

    #[test]
    fn delivery_never_opens_the_payload() {
        let plan = fleet_roles();
        let inlet = inlet(&plan);

        let delivery = inlet.route(KeyedEmission::new(key("s1"), Opaque));

        assert_eq!(delivery.key(), &key("s1"));
        assert_eq!(delivery.target().port(), &port("event"));
        assert_eq!(delivery.target().actor().name(), &name("intake"));
        let Opaque = delivery.into_event();
    }

    #[test]
    fn an_inlet_outside_its_template_is_not_bound() {
        let plan = fleet_roles();
        let template = admit_template(&fleet_roles(), &fleet_scope(), &name("cell")).unwrap();
        let outside = NamedActorId::new(fleet_scope(), name("elsewhere"));
        assert!(matches!(
            KeyedInlet::new(template, Endpoint::new(outside, port("event"))),
            Err(CellDerivationError::NotUnderTemplate { .. })
        ));
    }

    #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct TestProducer(NamedActorId);

    impl ProducerIdentity for TestProducer {
        type EventProducer = NamedActorId;
        fn from_event_producer(producer: Self::EventProducer) -> Self {
            Self(producer)
        }
    }
}
