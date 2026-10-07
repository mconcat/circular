
use super::share::OutWire;
use super::{Delivery, ProductEvent};
use circular_core::{RevisionEpochId, Sequence};
use circular_plan::{ActorId, EdgeId, PortId};
use circular_store::{ArrivalOrigin, BoundaryFact, ProductStore, Record};
use std::collections::{BTreeMap, VecDeque};

pub(crate) struct Gone(pub(crate) Box<Delivery>);

#[derive(Debug, Default)]
pub(crate) struct Settled {
    edges: BTreeMap<EdgeId, Sequence>,
    bodies: BTreeMap<u64, u64>,
}

impl Settled {
    fn settle(&mut self, edge: EdgeId, sequence: Sequence) {
        let last = self.edges.entry(edge).or_insert(sequence);
        *last = (*last).max(sequence);
    }

    fn body(&mut self, sequence: Sequence) {
        let at = sequence.get();
        if self.recorded(at) {
            return;
        }
        let mut start = at;
        let mut end = at;
        if let Some((&before, &until)) = self.bodies.range(..at).next_back()
            && until.checked_add(1) == Some(at)
        {
            self.bodies.remove(&before);
            start = before;
        }
        if let Some(after) = at.checked_add(1)
            && let Some(until) = self.bodies.remove(&after)
        {
            end = until;
        }
        self.bodies.insert(start, end);
    }

    fn recorded(&self, at: u64) -> bool {
        self.bodies
            .range(..=at)
            .next_back()
            .is_some_and(|(_, until)| *until >= at)
    }

    pub(crate) fn pending(&self, edge: &EdgeId, sequence: Sequence) -> bool {
        self.edges.get(edge).is_none_or(|last| sequence > *last) && self.recorded(sequence.get())
    }

    pub(crate) fn covers(&self, edge: &EdgeId, sequence: Sequence) -> bool {
        self.edges.get(edge).is_some_and(|last| *last >= sequence)
    }
}

pub(crate) fn settle(
    settled: &mut BTreeMap<ActorId, Settled>,
    record: &Record<ProductStore>,
) -> Result<(), String> {
    match record {
        Record::Boundary(boundary) => match boundary.fact() {
            BoundaryFact::EmissionBody { .. } => {
                let at = record.header().at();
                settled
                    .entry(at.producer().clone())
                    .or_default()
                    .body(at.sequence());
            }
            BoundaryFact::Arrival {
                route_edge: Some(edge),
                origin,
                ..
            }
            | BoundaryFact::Admission {
                route_edge: Some(edge),
                origin,
                ..
            } => {
                if let ArrivalOrigin::EdgeDelivery { sender, .. } = origin.as_ref() {
                    settled
                        .entry(sender.producer().clone())
                        .or_default()
                        .settle(edge.clone(), sender.sequence());
                }
            }
            _ => {}
        },
        Record::Observation(_) => {
            let Some(letter) =
                crate::dead_letter_writer::dead_letter_value(record).map_err(|e| e.to_string())?
            else {
                return Ok(());
            };
            let field = |name: &str| letter.as_object().and_then(|fields| fields.get(name));
            let Some(target) = field("target").filter(|target| target.as_object().is_none()) else {
                return Ok(());
            };
            let edge = circular_runtime::product_identity::edge_from_value(target)
                .map_err(|error| format!("dead-letter target edge: {error}"))?;
            let dropped = circular_store::record_stamp_from_value(
                field("dropped").ok_or("a dead letter names no dropped stamp")?,
            )?;
            settled
                .entry(dropped.producer().clone())
                .or_default()
                .settle(edge, dropped.sequence());
        }
        _ => {}
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct Outlets {
    cell: Option<RevisionEpochId>,
    wires: BTreeMap<RevisionEpochId, BTreeMap<PortId, Vec<OutWire>>>,
    waiting: VecDeque<(OutWire, Delivery)>,
}

impl Outlets {
    pub(crate) fn new(revision: RevisionEpochId, wires: BTreeMap<PortId, Vec<OutWire>>) -> Self {
        Self {
            cell: None,
            wires: BTreeMap::from([(revision, wires)]),
            waiting: VecDeque::new(),
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
        wires: BTreeMap<PortId, Vec<OutWire>>,
    ) {
        self.wires.insert(revision, wires);
    }

    pub(crate) fn wires(&self, port: &PortId, revision: RevisionEpochId) -> Option<&[OutWire]> {
        let wires = if let Some(minted) = self.cell {
            self.wires.get(&minted)?
        } else {
            self.wires.range(..=revision).next_back()?.1
        };
        Some(wires.get(port).map(Vec::as_slice).unwrap_or_default())
    }

    pub(crate) fn emit(
        &mut self,
        wires: Vec<OutWire>,
        event: &ProductEvent,
        encoded: &circular_core::EncodedPayload,
        result: &circular_runtime::EnvelopeResult,
        closed: bool,
    ) -> Vec<Gone> {
        let mut gone = Vec::new();
        for wire in wires {
            let delivery = Delivery {
                edge: wire.edge.clone(),
                inlet: wire.to.port().clone(),
                event: event.clone(),
                encoded: encoded.clone(),
                result: result.clone(),
                credit: None,
            };
            if let Some(refused) = self.offer(wire, delivery, closed) {
                gone.push(refused);
            }
        }
        gone
    }

    pub(crate) fn forward(
        &mut self,
        wire: OutWire,
        event: &ProductEvent,
        encoded: &circular_core::EncodedPayload,
        result: &circular_runtime::EnvelopeResult,
        closed: bool,
    ) -> Option<Gone> {
        let delivery = Delivery {
            edge: wire.edge.clone(),
            inlet: wire.to.port().clone(),
            event: event.clone(),
            encoded: encoded.clone(),
            result: result.clone(),
            credit: None,
        };
        self.offer(wire, delivery, closed)
    }

    pub(crate) fn resend(
        &mut self,
        wire: OutWire,
        event: &ProductEvent,
        encoded: &circular_core::EncodedPayload,
        result: &circular_runtime::EnvelopeResult,
    ) {
        let delivery = Delivery {
            edge: wire.edge.clone(),
            inlet: wire.to.port().clone(),
            event: event.clone(),
            encoded: encoded.clone(),
            result: result.clone(),
            credit: None,
        };
        self.waiting.push_back((wire, delivery));
    }

    fn offer(&mut self, wire: OutWire, mut delivery: Delivery, closed: bool) -> Option<Gone> {
        if let Some(credit) = &wire.credit {
            if !self.waiting.is_empty() {
                self.waiting.push_back((wire, delivery));
                return None;
            }
            match credit.clone().try_acquire_owned() {
                Ok(permit) => delivery.credit = Some(permit),
                Err(_) => {
                    self.waiting.push_back((wire, delivery));
                    return None;
                }
            }
        }
        match wire.address.deliver(delivery) {
            Ok(()) => None,
            Err(mut delivery) if closed => {
                delivery.credit = None;
                self.waiting.push_back((wire, *delivery));
                None
            }
            Err(delivery) => Some(Gone(delivery)),
        }
    }

    pub(crate) fn blocked(&self) -> bool {
        !self.waiting.is_empty()
    }

    pub(crate) async fn flush(&mut self, door: &super::actor::Door) -> Vec<Gone> {
        let mut gone = Vec::new();
        loop {
            let Some((wire, _)) = self.waiting.front() else {
                break;
            };
            let acquired = match wire.credit.clone() {
                Some(credit) => Some(credit.acquire_owned().await),
                None => None,
            };
            if door.paused() {
                break;
            }
            let (wire, mut delivery) = self
                .waiting
                .pop_front()
                .expect("the delivery that waited is still at the front");
            match acquired {
                Some(Ok(permit)) => delivery.credit = Some(permit),
                Some(Err(_)) => {
                    gone.push(Gone(Box::new(delivery)));
                    continue;
                }
                None => {}
            }
            if let Err(refused) = wire.address.deliver(delivery) {
                gone.push(Gone(refused));
            }
        }
        gone
    }

    pub(crate) fn abandon(&mut self) -> Vec<Gone> {
        self.waiting
            .drain(..)
            .map(|(_, delivery)| Gone(Box::new(delivery)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Settled;
    use circular_core::Sequence;
    use circular_plan::{EdgeId, Name, NamedActorId, ScopeId};
    use std::collections::{BTreeMap, BTreeSet};

    fn edge(name: &str) -> EdgeId {
        EdgeId::outcome(NamedActorId::new(
            ScopeId::root(),
            Name::from_normalized(name),
        ))
    }

    #[test]
    fn the_fold_answers_like_the_set_model_for_generated_records() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |bound: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % bound
        };
        let edges = [edge("a"), edge("b"), edge("c")];
        for _ in 0..500 {
            let mut fold = Settled::default();
            let mut bodies = BTreeSet::new();
            let mut last = BTreeMap::new();
            for _ in 0..next(60) {
                let sequence = next(40);
                if next(3) == 0 {
                    let edge = &edges[usize::try_from(next(3)).unwrap()];
                    fold.settle(edge.clone(), Sequence::new(sequence).unwrap());
                    let slot = last.entry(edge.clone()).or_insert(sequence);
                    *slot = (*slot).max(sequence);
                } else {
                    fold.body(Sequence::new(sequence).unwrap());
                    bodies.insert(sequence);
                }
            }
            for edge in &edges {
                for sequence in 0..42 {
                    let at = Sequence::new(sequence).unwrap();
                    let unsettled = last.get(edge).is_none_or(|last| sequence > *last);
                    assert_eq!(
                        fold.pending(edge, at),
                        unsettled && bodies.contains(&sequence),
                        "pending {sequence} on {edge:?}: bodies {bodies:?}, settled {last:?}"
                    );
                    assert_eq!(
                        fold.covers(edge, at),
                        last.get(edge).is_some_and(|last| *last >= sequence),
                    );
                }
            }
        }
    }
}
