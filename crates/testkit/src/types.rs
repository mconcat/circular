
use circular_core::StreamIdentity;
use circular_runtime::ActorTypes;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TestRun(pub u16);

impl StreamIdentity for TestRun {}

pub struct TestTypes<P, G = ()>(PhantomData<fn() -> (P, G)>);

impl<P, G> ActorTypes for TestTypes<P, G> {
    type Stream = TestRun;
    type Event = P;
    type Payload = P;
    type EffectId = u64;
    type StateVersion = u16;
    type Observation = ();
    type Grants = G;

    fn payload(event: &Self::Event) -> &Self::Payload {
        event
    }
}
