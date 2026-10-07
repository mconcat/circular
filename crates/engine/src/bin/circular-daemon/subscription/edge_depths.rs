
use super::{
    FlowFsm, PumpResult, Subscription, SubscriptionSource, credit_flow, write_subscription_end_at,
    write_subscription_frame_at,
};
use crate::daemon::subscription_catalog::EDGE_DEPTHS_TARGET;
use circular_core::Value;
use circular_protocol::Partition;
use circular_protocol::SubscriptionEndReason as FsmEndReason;
use circular_protocol::declaration_payload::{Accepted, CommandResult};
use circular_protocol::rejection_code::RejectionReason;
use circular_protocol::subscription_payload::{
    FrameOrigin, SubscriptionEndReason, SubscriptionEnded, SubscriptionFrame,
};
use circular_transport::LocalByteStream;
use std::collections::VecDeque;
use tokio::sync::oneshot::error::TryRecvError;

pub(crate) struct Feed {
    asked: Vec<crate::kernel::system::Asked>,
    ready: VecDeque<Value>,
    delivered: u64,
}

impl Feed {
    fn new(asked: Vec<crate::kernel::system::Asked>) -> Self {
        Self {
            asked,
            ready: VecDeque::new(),
            delivered: 0,
        }
    }

    fn collect(&mut self) {
        let mut waiting = Vec::with_capacity(self.asked.len());
        for mut asked in self.asked.drain(..) {
            match asked.answer.try_recv() {
                Ok(depths) => self.ready.extend(depths.iter().map(edge_row)),
                Err(TryRecvError::Closed) => self.ready.push_back(ended_row(&asked.actor)),
                Err(TryRecvError::Empty) => waiting.push(asked),
            }
        }
        self.asked = waiting;
    }
}

pub(crate) fn open(
    open: &circular_protocol::subscription_payload::Subscribe,
    correlation: u32,
    system: Option<&crate::daemon::ledger::SystemRuntime>,
    wake: &std::sync::Arc<engine::wake::Wake>,
    live: &mut Option<Subscription>,
) -> CommandResult {
    if open.lens.is_some() {
        return CommandResult::Rejected(RejectionReason::Malformed.reject(
            Partition::Subscription,
            format!("{EDGE_DEPTHS_TARGET} does not read under a replay lens"),
        ));
    }
    if open.args != Value::Null {
        return CommandResult::Rejected(RejectionReason::Malformed.reject(
            Partition::Subscription,
            format!("{EDGE_DEPTHS_TARGET} args are not Null"),
        ));
    }
    let asked = match system {
        None => Vec::new(),
        Some(system) => match system.pipeline.edge_depths(wake.clone()) {
            Ok(asked) => asked,
            Err(_) => {
                return CommandResult::Rejected(RejectionReason::Unresolved.reject(
                    Partition::Subscription,
                    "the actor kernel stopped before it was asked".to_owned(),
                ));
            }
        },
    };
    *live = Some(Subscription {
        source: SubscriptionSource::EdgeDepths {
            feed: Feed::new(asked),
        },
        flow: credit_flow(0),
        correlation,
        lens: None,
    });
    CommandResult::Accepted(Accepted::Nothing)
}

fn edge_row(row: &crate::kernel::inlet::EdgeDepth) -> Value {
    let count = |value: usize| Value::UInt(value as u64);
    Value::object([
        (
            "edge",
            circular_store::edge_value(&row.edge).expect("an edge identity always has its carrier"),
        ),
        ("depth", count(row.depth)),
        ("queued", count(row.queued)),
        ("capacity", row.capacity.map_or(Value::Null, count)),
    ])
    .expect("four distinct keys")
}

fn ended_row(actor: &circular_plan::NamedActorId) -> Value {
    Value::object([
        (
            "actor",
            circular_store::named_actor_value(actor)
                .expect("a standing member's name has its carrier"),
        ),
        (
            "code",
            Value::UInt(u64::from(
                RejectionReason::EndedBeforeAnswering.number_in(Partition::Subscription),
            )),
        ),
    ])
    .expect("two distinct keys")
}

pub(super) fn pump(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    feed: &mut Feed,
    flow: &mut FlowFsm,
    correlation: u32,
) -> PumpResult {
    feed.collect();
    while let Some(row) = feed.ready.front() {
        let anchor = feed.delivered + 1;
        let frame = SubscriptionFrame::Credit {
            origin: FrameOrigin::Live,
            payload: row.clone(),
            pending_after: feed.ready.len() as u64 - 1,
        };
        if !write_subscription_frame_at(stream, correlation, &frame, flow, anchor) {
            return PumpResult::Open {
                pending: feed.ready.len() as u64,
            };
        }
        feed.ready.pop_front();
        feed.delivered = anchor;
    }
    if !feed.asked.is_empty() {
        return PumpResult::Open { pending: 0 };
    }
    let _ = flow.end(FsmEndReason::Complete, 0);
    write_subscription_end_at(
        stream,
        correlation,
        &SubscriptionEnded {
            reason: SubscriptionEndReason::Complete,
            code: 0,
            anchor: feed.delivered.to_be_bytes().to_vec(),
        },
    );
    PumpResult::Ended
}
