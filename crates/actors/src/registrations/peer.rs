//! Peer actor registration source.
//!
//! The stable registration keeps provider-native details behind the runtime
//! adapter while exposing the same graph contract for every harness.

use super::*;

pub(crate) fn peer_source() -> SpecSource<ExternalEffect> {
    let fixed = PortSet::try_new(
        vec![
            required_primary_inlet("send", Flow::Stream(open_object_shape()), "Send"),
            required_inlet("refresh", Flow::Stream(Shape::Any), false, "Refresh"),
        ],
        vec![
            primary_outlet("message", Flow::Stream(open_object_shape()), "Message"),
            outlet("peers", Flow::Stream(open_object_shape()), false, "Peers"),
            outlet(
                "delivery",
                Flow::Stream(open_object_shape()),
                false,
                "Delivery",
            ),
            outlet(
                "binding",
                Flow::Stream(open_object_shape()),
                false,
                "Binding",
            ),
        ],
    )
    .expect("peer roles are unique in each direction");
    let requires = RequireRules::external(
        RequireRule::new(Capability::PeerDiscover, Condition::Always),
        [
            RequireRule::new(Capability::PeerSend, Condition::Always),
            RequireRule::new(Capability::PeerAdvertise, Condition::Always),
            RequireRule::new(Capability::PeerReceive, Condition::Always),
        ],
    );
    let effect = ExternalEffect::new(
        Durability::Durable,
        failed_stand_ins(
            EffectCtor::PeerDiscover,
            [
                EffectCtor::PeerBind,
                EffectCtor::PeerSend,
                EffectCtor::PeerUnbind,
                EffectCtor::PeerReceive,
            ],
        ),
    );

    published_external_source(
        actor_label("Peer"),
        Description::from_static(
            "Send and receive messages with other agent sessions.",
        ),
        PortRule::new(fixed, Box::new([])),
        requires,
        effect,
        create_config_schema([
            described(
                (
                    config_path("adapter"),
                    mandatory_slot_in(
                        crate::config::SlotKind::space(&crate::config::PeerAdapter),
                        None,
                    ),
                ),
                "Adapter",
                "The peer adapter name.",
            ),
            described(
                (
                    config_path("realm"),
                    mandatory_slot(Shape::Base(BaseShape::String), None),
                ),
                "Realm",
                "The realm this actor binds into.",
            ),
            described(
                (
                    config_path("name"),
                    mandatory_slot(Shape::Base(BaseShape::String), None),
                ),
                "Binding name",
                "The requested binding name.",
            ),
            described(
                (
                    config_path("inbound_policy"),
                    mandatory_slot(open_object_shape(), None),
                ),
                "Inbound policy",
                "Who may send to this actor: any_known_peer set to true for every sender on this adapter and realm, or exact with the list of allowed addresses.",
            ),
            starting(
                described(
                    mandatory_typed(&crate::peer_actor::INBOX_CAPACITY),
                    "Inbox capacity",
                    "How many incoming messages can wait before this actor records them.",
                ),
                Value::Int(64),
            ),
        ]),
    )
    .with_view_config(view_defaults([(
        "heading",
        Value::string("Peer connection"),
    )]))
}
