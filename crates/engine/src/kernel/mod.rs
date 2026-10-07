
pub(crate) mod actor;
pub(crate) mod assemble;
pub(crate) mod column;
pub(crate) mod effect;
pub(crate) mod inlet;
pub(crate) mod outlet;
pub(crate) mod peer;
pub(crate) mod record;
pub(crate) mod share;
pub(crate) mod snapshot;
pub(crate) mod source;
pub(crate) mod system;
pub(crate) mod turn;

use circular_actors::ProductPayload;
use circular_plan::{ActorId, EdgeId, PortId};
use circular_store::{OperationId, StreamId};

pub(crate) type ProductEvent = circular_core::Event<StreamId, ActorId, ProductPayload, OperationId>;

#[derive(Clone, Debug)]
pub(crate) struct Address(tokio::sync::mpsc::UnboundedSender<Message>);

impl Address {
    pub(crate) fn channel() -> (Self, tokio::sync::mpsc::UnboundedReceiver<Message>) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        (Self(sender), receiver)
    }

    pub(crate) fn same_channel(&self, other: &Self) -> bool {
        self.0.same_channel(&other.0)
    }

    pub(crate) fn send(&self, message: Message) -> Result<(), Message> {
        self.0.send(message).map_err(|refused| refused.0)
    }

    pub(crate) fn deliver(&self, delivery: Delivery) -> Result<(), Box<Delivery>> {
        self.0
            .send(Message::Deliver(delivery))
            .map_err(|refused| match refused.0 {
                Message::Deliver(delivery) => Box::new(delivery),
                _ => panic!("the refused message is the delivery sent"),
            })
    }
}

pub(crate) enum Message {
    Deliver(Delivery),
    Inject(actor::Injection),
    Inlets(Box<share::Share>),
    Share(Box<share::Share>, Option<Box<actor::Incarnation>>),
    Settled(Box<circular_runtime::EffectOutcome<circular_runtime::EffectId>>),
    ApprovalUnavailable(String),
    Refused(Box<source::Refused>),
    Fell(crate::activation_detail::RegistrationFailure),
    CellStood(Box<system::CellStood>),
    Depths(DepthsReply),
    Control(Control),
}

pub(crate) struct DepthsReply {
    answer: Option<tokio::sync::oneshot::Sender<Vec<inlet::EdgeDepth>>>,
    wake: std::sync::Arc<crate::wake::Wake>,
}

impl DepthsReply {
    pub(crate) fn new(
        answer: tokio::sync::oneshot::Sender<Vec<inlet::EdgeDepth>>,
        wake: std::sync::Arc<crate::wake::Wake>,
    ) -> Self {
        Self {
            answer: Some(answer),
            wake,
        }
    }

    pub(crate) fn answer(mut self, depths: Vec<inlet::EdgeDepth>) {
        if let Some(answer) = self.answer.take() {
            let _ = answer.send(depths);
        }
    }
}

impl Drop for DepthsReply {
    fn drop(&mut self) {
        drop(self.answer.take());
        self.wake.notify();
    }
}

pub(crate) struct Delivery {
    pub(crate) edge: EdgeId,
    pub(crate) inlet: PortId,
    pub(crate) event: ProductEvent,
    /// Same-journal wire emission: the producer submitted this body before sending.
    /// Lifecycle inputs and external injections never construct a Delivery.
    pub(crate) encoded: circular_core::EncodedPayload,
    pub(crate) result: circular_runtime::EnvelopeResult,
    pub(crate) credit: Option<tokio::sync::OwnedSemaphorePermit>,
}

pub(crate) enum Control {
    Retire,
    Stop,
    Pause { force: bool },
    Resume,
    Harness {
        harness: circular_runtime::AgentHarnessName,
        executor: Option<crate::execution_profile::AgentExecutorFactory>,
    },
}
