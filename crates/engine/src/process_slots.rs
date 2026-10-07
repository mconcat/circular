use circular_plan::{ActorId, EdgeId};
use circular_runtime::{EffectId, EffectOccasion};
use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueueScope(ActorId, Option<EdgeId>);
impl QueueScope {
    fn of(id: &EffectId) -> Self {
        Self(
            id.actor().clone(),
            match id.occasion() {
                EffectOccasion::Delivery(edge, _) => edge.clone(),
                EffectOccasion::Poll(..) => None,
            },
        )
    }
}
#[derive(Debug)]
struct Ticket {
    ready: AtomicBool,
    scope: QueueScope,
    wake: Mutex<Option<std::task::Waker>>,
}
#[derive(Debug, Default)]
struct State {
    running: usize,
    queued: VecDeque<Arc<Ticket>>,
}
#[derive(Debug)]
pub(crate) struct ProcessSlots {
    max: NonZeroUsize,
    queue_capacity: Option<NonZeroUsize>,
    state: Mutex<State>,
}
#[derive(Debug)]
pub(crate) struct ProcessPermit {
    owner: Arc<ProcessSlots>,
    ticket: Arc<Ticket>,
}
impl ProcessSlots {
    pub(crate) fn new(max: NonZeroUsize, queue_capacity: Option<NonZeroUsize>) -> Arc<Self> {
        Arc::new(Self {
            max,
            queue_capacity,
            state: Mutex::new(State::default()),
        })
    }
    pub(crate) fn reserve(
        self: &Arc<Self>,
        id: &EffectId,
        peer_capacity: usize,
    ) -> Option<Arc<ProcessPermit>> {
        let mut state = self.state.lock().expect("process slots");
        let scope = QueueScope::of(id);
        let ready = state.running < self.max.get() && state.queued.is_empty();
        if !ready {
            let (depth, capacity) = match self.queue_capacity {
                Some(capacity) => (state.queued.len(), capacity.get()),
                None => (
                    state
                        .queued
                        .iter()
                        .filter(|ticket| ticket.scope == scope)
                        .count(),
                    peer_capacity,
                ),
            };
            if depth >= capacity {
                return None;
            }
        }
        let ticket = Arc::new(Ticket {
            ready: AtomicBool::new(ready),
            scope,
            wake: Mutex::new(None),
        });
        if ready {
            state.running += 1;
        } else {
            state.queued.push_back(ticket.clone());
        }
        Some(Arc::new(ProcessPermit {
            owner: self.clone(),
            ticket,
        }))
    }
}
impl ProcessPermit {
    pub(crate) fn set_waker(&self, wake: std::task::Waker) {
        *self.ticket.wake.lock().expect("process ticket wake") = Some(wake.clone());
        if self.ready() {
            wake.wake();
        }
    }
    pub(crate) fn ready(&self) -> bool {
        self.ticket.ready.load(Ordering::Acquire)
    }
}
impl Drop for ProcessPermit {
    fn drop(&mut self) {
        let mut state = self.owner.state.lock().expect("process slots");
        if self.ready() {
            state.running -= 1;
        } else {
            state.queued.retain(|item| !Arc::ptr_eq(item, &self.ticket));
        }
        let mut promoted = Vec::new();
        while state.running < self.owner.max.get() {
            let Some(next) = state.queued.pop_front() else {
                break;
            };
            state.running += 1;
            next.ready.store(true, Ordering::Release);
            promoted.push(next);
        }
        drop(state);
        for ticket in promoted {
            let wake = ticket.wake.lock().expect("process ticket wake").clone();
            if let Some(wake) = wake {
                wake.wake();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fifo_promotion_wakes_the_waiting_effect_owner_without_polling() {
        struct Ready(std::sync::mpsc::Sender<()>);
        impl std::task::Wake for Ready {
            fn wake(self: Arc<Self>) {
                self.0.send(()).unwrap();
            }
        }
        let slots = ProcessSlots::new(NonZeroUsize::new(1).unwrap(), None);
        let active = slots.reserve(&crate::test_effect(0), 1).unwrap();
        let next = slots.reserve(&crate::test_effect(1), 1).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        next.set_waker(std::task::Waker::from(Arc::new(Ready(send))));
        assert!(receive.try_recv().is_err());
        drop(active);
        receive
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(next.ready());
        assert!(receive.try_recv().is_err());
        let last = slots.reserve(&crate::test_effect(2), 1).unwrap();
        drop(next);
        let (send, receive) = std::sync::mpsc::channel();
        last.set_waker(std::task::Waker::from(Arc::new(Ready(send))));
        receive
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(last.ready());
    }
}
