struct InjectionCustody {
    ledger: InjectionLedger<PlanExportKey, circular_runtime::ExternalOrigin, (), (), u64>,
    sequence: u64,
}
impl InjectionCustody {
    fn new() -> Self {
        Self {
            ledger: InjectionLedger::new(std::num::NonZeroUsize::new(usize::MAX).unwrap()),
            sequence: 0,
        }
    }
}

struct MountBinding {
    emitter: circular_plan::NamedActorId,
    outlet: circular_plan::PortId,
    declared: Option<circular_actors::GroundShape>,
    input: crate::kernel::system::Entrance,
}

struct MountEntrance {
    binding: Result<MountBinding, String>,
    custody: Arc<Mutex<InjectionCustody>>,
}

#[derive(Clone, Default)]
pub(crate) struct MountEntrances(Arc<BTreeMap<PlanExportKey, Arc<MountEntrance>>>);

impl MountEntrances {
    fn activate(
        previous: &Self,
        plan: &AuthoredProjection,
        ports: &engine::ResolvedRevisionPorts,
        pipeline: &crate::kernel::system::Pipeline,
    ) -> Self {
        let custody = |address: &PlanExportKey| {
            previous.0.get(address).map_or_else(
                || Arc::new(Mutex::new(InjectionCustody::new())),
                |old| old.custody.clone(),
            )
        };
        let mut entrances = BTreeMap::new();
        fold_projection::<()>(plan, |layer| {
            let scope = layer.scope();
            for (name, export) in layer.exports() {
                let address = PlanExportKey {
                    scope: circular_runtime::product_identity::wire_scope(scope),
                    local: name.name().as_str().to_owned(),
                };
                let binding = if export.request_boundary_ref().is_none() {
                    Err(format!(
                        "export `{address}` has no request role bound — there is no injection destination"
                    ))
                } else {
                    match ports.request(scope, name) {
                        None => Err(format!(
                            "admitted revision has no resolved request mount: {address}"
                        )),
                        Some(RequestExportIngress::BoundarySource { emitter, outlet }) => {
                            match pipeline.entrance(emitter) {
                                Ok(Some(input)) => Ok(MountBinding {
                                    declared: ports.shape(emitter, outlet, false).ok().flatten(),
                                    input,
                                    emitter: emitter.clone(),
                                    outlet: outlet.clone(),
                                }),
                                Ok(None) => Err(format!(
                                    "the request boundary of `{address}` is not standing"
                                )),
                                Err(_) => Err("the actor kernel stopped".to_owned()),
                            }
                        }
                    }
                };
                entrances.insert(
                    address.clone(),
                    Arc::new(MountEntrance {
                        binding,
                        custody: custody(&address),
                    }),
                );
            }
        });
        for (address, old) in previous.0.iter() {
            entrances.entry(address.clone()).or_insert_with(|| {
                Arc::new(MountEntrance {
                    binding: Err(NOT_IN_EXPORT_SECTION.to_owned()),
                    custody: old.custody.clone(),
                })
            });
        }
        Self(Arc::new(entrances))
    }
}

const NOT_IN_EXPORT_SECTION: &str = "not present in this plan's export section";

#[derive(Clone)]
pub(crate) struct ServerIngress {
    entrances: MountEntrances,
    pipeline: crate::kernel::system::Pipeline,
    journal: Option<engine::ProductDurableArrivalJournal>,
    limits: crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits,
}
impl ServerRun {
    pub(crate) fn ingress(
        &self,
        limits: crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits,
    ) -> ServerIngress {
        ServerIngress {
            entrances: self.entrances.clone(),
            pipeline: self.pipeline.clone(),
            journal: self.arrival_journal.clone(),
            limits,
        }
    }
    fn activate_mount_entrances(&mut self) {
        self.entrances =
            MountEntrances::activate(&self.entrances, &self.plan, &self.ports, &self.pipeline);
    }
}
impl ServerRead {
    pub(crate) fn pause(&self, force: bool) -> Option<crate::kernel::system::Acceptance> {
        let input = self.input.as_ref()?;
        Some(input.pipeline.pause(
            RevisionEpochId::new(self.revisions).expect("standing revision"),
            force,
        ))
    }

    pub(crate) fn ingress(&self) -> Option<&ServerIngress> {
        self.input.as_ref()
    }
    pub(crate) fn approval_owner(&self) -> Option<RuntimeApprovalQueue> {
        self.approval_owner.clone()
    }
    pub(crate) fn advanced_input_prefix(&self) -> Option<Self> {
        let journal = self
            .input
            .as_ref()
            .and_then(|input| input.journal.as_ref())?;
        let published = journal.read_view();
        if published.same(&self.store) {
            return None;
        }
        let through = self.store.end();
        let compatible = published.end() >= through
            && (through..published.end()).all(|index| {
                let Ok(Some(revision)) = published.row_revision(index) else {
                    return false;
                };
                revision.get() <= self.revisions
            });
        if !compatible {
            return None;
        }
        let mut read = self.clone();
        read.store = published;
        Some(read)
    }
}
pub(crate) enum IngressRefusal {
    UnknownMount(String),
    NotAccepting,
    Storage(String),
    OutOfDomain(String),
}

impl IngressRefusal {
    fn storage(reason: impl Into<String>) -> Self {
        Self::Storage(reason.into())
    }
}

impl std::fmt::Display for IngressRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownMount(reason) | Self::Storage(reason) | Self::OutOfDomain(reason) => {
                formatter.write_str(reason)
            }
            Self::NotAccepting => formatter.write_str(
                "this mount's source owner has recorded a pause and is not accepting input",
            ),
        }
    }
}

impl ServerIngress {
    pub(crate) fn recorder_stop(
        &self,
    ) -> Option<circular_protocol::rejection_code::RejectionReason> {
        self.journal
            .as_ref()
            .and_then(engine::ProductDurableArrivalJournal::recorder_stop)
            .map(|stop| stop.code())
    }

    fn out_of_domain(binding: &MountBinding, payload: &ProductPayload) -> Option<IngressRefusal> {
        let declared = binding.declared.as_ref()?;
        if circular_actors::value_inhabits(payload.value(), declared) {
            return None;
        }
        Some(IngressRefusal::OutOfDomain(format!(
            "the injected value is outside this mount's declared domain; {}",
            circular_core::spelling::allowed([circular_core::spelling::ShapeText(
                declared.as_shape()
            )
            .to_string()])
        )))
    }

    pub(crate) fn journal_measure(
        &self,
    ) -> Option<Result<crate::daemon::runtime_arrival_retention::JournalMeasure, String>> {
        self.journal.as_ref().map(|journal| {
            crate::daemon::runtime_arrival_retention::JournalMeasure::of(journal, self.limits)
        })
    }

    pub(crate) fn inject(
        &self,
        mount: &PlanExportKey,
        payload: ProductPayload,
        origin: circular_runtime::ExternalOrigin,
    ) -> Result<(), IngressRefusal> {
        let name = mount;
        let entrance = self
            .entrances
            .0
            .get(name)
            .ok_or_else(|| IngressRefusal::UnknownMount(NOT_IN_EXPORT_SECTION.to_owned()))?;
        let binding = entrance
            .binding
            .as_ref()
            .map_err(|reason| IngressRefusal::UnknownMount(reason.clone()))?;
        {
            let mut custody = entrance
                .custody
                .lock()
                .map_err(|_| IngressRefusal::storage("injection custody poisoned"))?;
            match custody.ledger.begin(name.clone(), origin.clone(), ()) {
                InjectionLookup::Replayed { .. } => return Ok(()),
                InjectionLookup::Awaiting { .. } => {
                    return Err(IngressRefusal::storage(
                        "injection has not completed durable acceptance",
                    ));
                }
                InjectionLookup::Exhausted => {
                    return Err(IngressRefusal::storage(
                        "process-lifetime injection FSM capacity is exhausted",
                    ));
                }
                InjectionLookup::Started { .. } => {}
            }
        }
        let committed = match Self::out_of_domain(binding, &payload) {
            Some(refusal) => {
                let mut custody = entrance
                    .custody
                    .lock()
                    .map_err(|_| IngressRefusal::storage("injection custody poisoned"))?;
                custody.ledger.reject(name, &origin);
                return Err(refusal);
            }
            None => binding
                .input
                .inject(binding.outlet.clone(), payload, origin.clone())
                .map(|_| ()),
        };
        let mut custody = entrance
            .custody
            .lock()
            .map_err(|_| IngressRefusal::storage("injection custody poisoned"))?;
        match committed {
            Ok(()) => {}
            Err(crate::kernel::actor::Refusal::NotAccepting) => {
                custody.ledger.reject(name, &origin);
                return Err(IngressRefusal::NotAccepting);
            }
            Err(crate::kernel::actor::Refusal::Rejected(reason)) => {
                custody.ledger.reject(name, &origin);
                return Err(IngressRefusal::storage(format!(
                    "boundary injection was not committed: {reason}"
                )));
            }
        }
        let next = custody
            .sequence
            .checked_add(1)
            .ok_or_else(|| IngressRefusal::storage("injection acceptance sequence is exhausted"))?;
        custody
            .ledger
            .complete(name, &origin, (), next)
            .map_err(|error| {
                IngressRefusal::storage(format!("injection FSM completion failed: {error:?}"))
            })?;
        custody.sequence = next;
        drop(custody);
        Ok(())
    }
}
