
use super::share::{InletWire, capacity_of};
use super::{Delivery, ProductEvent};
use circular_core::{ArrivalIndex, RecordedInstant, RevisionEpochId, Stamp, Tick};
use circular_plan::{ActorId, Delivery as Policy, EdgeId, PortId, Shed};
use std::collections::{BTreeMap, VecDeque};

pub(crate) struct Recorded {
    pub(crate) index: ArrivalIndex,
    pub(crate) at: Stamp<ActorId>,
    pub(crate) observed_at: RecordedInstant,
    pub(crate) route: Option<EdgeId>,
    pub(crate) origin: super::turn::InputOrigin,
    pub(crate) inlet: PortId,
    pub(crate) revision: RevisionEpochId,
    pub(crate) event: ProductEvent,
    /// Producer bytes retained only for forwarding a recorded wire to a cell.
    pub(crate) emission_body: Option<circular_core::EncodedPayload>,
    pub(crate) result: circular_runtime::EnvelopeResult,
    pub(crate) outcome: Option<Box<circular_runtime::EffectOutcome<circular_runtime::EffectId>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EdgeDepth {
    pub(crate) edge: EdgeId,
    pub(crate) depth: usize,
    pub(crate) queued: usize,
    pub(crate) capacity: Option<usize>,
}

struct Held {
    delivery: Delivery,
    until: Option<Tick>,
    admitted: bool,
}

pub(crate) enum Received {
    Admitted,
    Held,
    Shed(Box<Delivery>),
    Unknown(Box<Delivery>),
}

pub(crate) struct Inlets {
    cell: Option<RevisionEpochId>,
    wires: BTreeMap<EdgeId, BTreeMap<RevisionEpochId, InletWire>>,
    held: Vec<Held>,
    arrived: VecDeque<(Delivery, bool)>,
    mailbox: VecDeque<Recorded>,
    mailbox_capacity: usize,
    occupied: BTreeMap<EdgeId, usize>,
}

impl Inlets {
    pub(crate) fn new(mailbox_capacity: usize) -> Self {
        Self {
            cell: None,
            wires: BTreeMap::new(),
            held: Vec::new(),
            arrived: VecDeque::new(),
            mailbox: VecDeque::new(),
            mailbox_capacity,
            occupied: BTreeMap::new(),
        }
    }

    pub(crate) fn cell_revision(&mut self, revision: RevisionEpochId) {
        self.cell = Some(revision);
    }

    pub(crate) fn begin_life(&mut self, revision: RevisionEpochId) {
        if self.cell.is_some() {
            self.cell = Some(revision);
        }
    }

    pub(crate) fn install(
        &mut self,
        revision: RevisionEpochId,
        wires: &BTreeMap<EdgeId, InletWire>,
    ) {
        for (edge, wire) in wires {
            self.wires
                .entry(edge.clone())
                .or_default()
                .insert(revision, wire.clone());
        }
    }

    pub(crate) fn wire(&self, edge: &EdgeId, revision: RevisionEpochId) -> Option<&InletWire> {
        let wires = self.wires.get(edge)?;
        if let Some(minted) = self.cell {
            return wires.get(&minted);
        }
        wires.range(..=revision).next_back().map(|(_, wire)| wire)
    }

    fn policy(&self, delivery: &Delivery) -> Option<&InletWire> {
        self.wire(&delivery.edge, delivery.event.stamp().revision())
    }

    pub(crate) fn receive(
        &mut self,
        delivery: Delivery,
        now: Tick,
        ticks_per_second: u32,
    ) -> Received {
        let Some(wire) = self.policy(&delivery).cloned() else {
            return Received::Unknown(Box::new(delivery));
        };
        if let Policy::BestEffort { on_full } = wire.shape.policy.delivery {
            let capacity = capacity_of(&wire.shape.policy, self.mailbox_capacity);
            let occupied = self.occupied.get(&delivery.edge).copied().unwrap_or(0);
            if occupied >= capacity {
                match on_full {
                    Shed::DropNewest => return Received::Shed(Box::new(delivery)),
                    Shed::DropOldest => {
                        if let Some(oldest) = self.take_oldest(&delivery.edge) {
                            self.admit(delivery, &wire, now, ticks_per_second);
                            return Received::Shed(Box::new(oldest));
                        }
                        return Received::Shed(Box::new(delivery));
                    }
                }
            }
            *self.occupied.entry(delivery.edge.clone()).or_default() += 1;
        }
        self.admit(delivery, &wire, now, ticks_per_second)
    }

    fn admit(
        &mut self,
        delivery: Delivery,
        wire: &InletWire,
        now: Tick,
        ticks_per_second: u32,
    ) -> Received {
        let delay = wire.shape.delay;
        if delay.is_zero() {
            self.arrived.push_back((delivery, false));
            return Received::Admitted;
        }
        let ticks = delay_ticks(delay.numerator(), delay.denominator(), ticks_per_second);
        self.held.push(Held {
            delivery,
            until: Some(Tick::new(now.get().saturating_add(ticks))),
            admitted: false,
        });
        Received::Held
    }

    fn take_oldest(&mut self, edge: &EdgeId) -> Option<Delivery> {
        if let Some(position) = self
            .held
            .iter()
            .position(|held| &held.delivery.edge == edge)
        {
            return Some(self.held.remove(position).delivery);
        }
        let position = self
            .arrived
            .iter()
            .position(|(delivery, _)| &delivery.edge == edge)?;
        self.arrived.remove(position).map(|(delivery, _)| delivery)
    }

    pub(crate) fn release(&mut self, now: Tick, open: impl Fn(&PortId) -> bool) {
        let mut kept = Vec::with_capacity(self.held.len());
        for held in std::mem::take(&mut self.held) {
            let due = held.until.is_none_or(|until| until <= now);
            if due && open(&held.delivery.inlet) {
                self.arrived.push_back((held.delivery, held.admitted));
            } else {
                kept.push(held);
            }
        }
        self.held = kept;
    }

    pub(crate) fn admit_last(&mut self) -> Option<&Delivery> {
        let held = self.held.last_mut()?;
        held.admitted = true;
        Some(&held.delivery)
    }

    pub(crate) fn admit_unrecorded(&mut self, stopping: bool, mut admit: impl FnMut(&Delivery)) {
        if stopping {
            for (delivery, admitted) in &mut self.arrived {
                if !*admitted {
                    admit(delivery);
                    *admitted = true;
                }
            }
        }
        for held in &mut self.held {
            if !held.admitted {
                admit(&held.delivery);
                held.admitted = true;
            }
        }
    }

    pub(crate) fn drain_unrecorded(&mut self) -> Vec<Delivery> {
        let mut drained: Vec<Delivery> = self
            .arrived
            .drain(..)
            .map(|(delivery, _)| delivery)
            .collect();
        drained.extend(self.held.drain(..).map(|held| held.delivery));
        drained
    }

    pub(crate) fn drain_unadmitted(&mut self) -> Vec<Delivery> {
        let (unadmitted, admitted): (Vec<_>, Vec<_>) = std::mem::take(&mut self.arrived)
            .into_iter()
            .partition(|(_, admitted)| !*admitted);
        self.arrived = admitted.into();
        let (loose, held): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|held| !held.admitted);
        self.held = held;
        unadmitted
            .into_iter()
            .map(|(delivery, _)| delivery)
            .chain(loose.into_iter().map(|held| held.delivery))
            .collect()
    }

    pub(crate) fn rehold(&mut self, delivery: Delivery, until: Option<Tick>) {
        if let Some(wire) = self.wire(&delivery.edge, delivery.event.stamp().revision())
            && !super::share::holds_back(&wire.shape.policy)
        {
            *self.occupied.entry(delivery.edge.clone()).or_default() += 1;
        }
        self.held.push(Held {
            delivery,
            until,
            admitted: true,
        });
    }

    pub(crate) fn delay_of(&self, delivery: &Delivery, ticks_per_second: u32) -> Option<u64> {
        let delay = self
            .wire(&delivery.edge, delivery.event.stamp().revision())?
            .shape
            .delay;
        Some(delay_ticks(
            delay.numerator(),
            delay.denominator(),
            ticks_per_second,
        ))
    }

    pub(crate) fn pressure(&self, edge: &EdgeId) -> Option<(usize, usize)> {
        let wire = self
            .wires
            .get(edge)
            .and_then(|revisions| revisions.values().next_back())?;
        let capacity = capacity_of(&wire.shape.policy, self.mailbox_capacity).max(1);
        Some((self.unrecorded(edge), capacity))
    }

    fn unrecorded(&self, edge: &EdgeId) -> usize {
        self.held
            .iter()
            .map(|held| &held.delivery)
            .chain(self.arrived.iter().map(|(delivery, _)| delivery))
            .filter(|delivery| &delivery.edge == edge)
            .count()
    }

    pub(crate) fn depths(&self) -> Vec<EdgeDepth> {
        let current = self.cell.or_else(|| {
            self.wires
                .values()
                .filter_map(|revisions| revisions.keys().next_back().copied())
                .max()
        });
        self.wires
            .iter()
            .filter_map(|(edge, revisions)| {
                let wire = revisions.values().next_back()?;
                let depth = self.unrecorded(edge);
                let queued = self
                    .mailbox
                    .iter()
                    .filter(|arrival| arrival.route.as_ref() == Some(edge))
                    .count();
                let standing = current.is_some_and(|revision| revisions.contains_key(&revision));
                (standing || depth > 0 || queued > 0).then(|| EdgeDepth {
                    edge: edge.clone(),
                    depth,
                    queued,
                    capacity: wire.shape.policy.capacity.map(|capacity| capacity.get()),
                })
            })
            .collect()
    }

    pub(crate) fn next_release(&self) -> Option<Tick> {
        self.held.iter().filter_map(|held| held.until).min()
    }

    pub(crate) fn next_to_record(&mut self, open: impl Fn(&PortId) -> bool) -> Option<Delivery> {
        if self.mailbox.len() >= self.mailbox_capacity {
            return None;
        }
        while let Some((delivery, admitted)) = self.arrived.pop_front() {
            if open(&delivery.inlet) {
                return Some(delivery);
            }
            self.held.push(Held {
                delivery,
                until: None,
                admitted,
            });
        }
        None
    }

    pub(crate) fn recorded(&mut self, arrival: Recorded) {
        if let Some(edge) = &arrival.route {
            self.release_occupancy(edge);
        }
        self.mailbox.push_back(arrival);
    }

    pub(crate) fn folded(&mut self, edge: &EdgeId) {
        self.release_occupancy(edge);
    }

    fn release_occupancy(&mut self, edge: &EdgeId) {
        if let Some(count) = self.occupied.get_mut(edge) {
            *count = count.saturating_sub(1);
        }
    }

    pub(crate) fn next_arrival(&mut self) -> Option<Recorded> {
        self.mailbox.pop_front()
    }

    pub(crate) fn first_unconsumed(&self, next: ArrivalIndex) -> ArrivalIndex {
        self.mailbox.front().map_or(next, |arrival| arrival.index)
    }

    pub(crate) fn full(&self) -> bool {
        self.mailbox.len() >= self.mailbox_capacity
    }

    pub(crate) fn has_arrival(&self) -> bool {
        !self.mailbox.is_empty()
    }
}

pub(crate) fn delay_ticks(numerator: u64, denominator: u64, ticks_per_second: u32) -> u64 {
    let scaled = u128::from(numerator) * u128::from(ticks_per_second);
    let denominator = u128::from(denominator);
    u64::try_from(scaled.div_ceil(denominator)).unwrap_or(u64::MAX)
}
