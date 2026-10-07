
use super::actor::Refusal;
use super::system::Entrance;
use crate::activation_detail::{RegistrationFailure, source};
use crate::otlp_ingress::edge::{OtlpDelivery, OtlpRefusal};
use crate::otlp_ingress::mount::OtlpMount;
use crate::otlp_ingress::state::{DropReason, OtlpSignal};
use crate::peer_bridges::transcript::{
    FileTail, Glob, TRANSCRIPT_LINE_OFFSET, TRANSCRIPT_LINE_PATH, TailDiagnostic, TailedLine,
    tail_path_allowed, transcript_payload,
};
use circular_actors::{FieldMap, GroundShape, ProductPayload, Shape};
use circular_core::Value;
use circular_plan::{ActorDecl, NamedActorId, PortId};
use circular_runtime::{Capability, DeadLetterReason, ExternalOrigin, PathScopes};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(crate) enum Plan {
    Listener(ListenerPlan),
    Otlp(OtlpPlan),
}

pub(crate) struct ListenerPlan {
    path: circular_runtime::NormalizedPath,
    glob: Glob,
    poll: Duration,
    roots: PathScopes,
    rewind: Arc<AtomicBool>,
}

pub(crate) struct OtlpPlan {
    config: Value,
}

impl Plan {
    pub(crate) fn of(
        declaration: &ActorDecl,
        actor: &super::turn::ProductActor,
    ) -> Result<Option<Self>, RegistrationFailure> {
        use crate::activation_detail::activation;
        let actor_type = *declaration.domain().actor_type();
        if !matches!(
            actor_type,
            circular_core::ActorType::Listener | circular_core::ActorType::Otlp
        ) {
            return Ok(None);
        }
        let value =
            crate::activation_config::fold_config(actor_type, declaration.domain().config())
                .map_err(|error| {
                    RegistrationFailure::new(activation::CONFIG_FOLD, error.to_string())
                })?
                .for_type(actor_type)
                .cloned()
                .map_err(|error| {
                    RegistrationFailure::new(
                        activation::UNEXPECTED_ACTOR_TYPE,
                        format!("{error:?}"),
                    )
                })?;
        if actor_type == circular_core::ActorType::Otlp {
            return Ok(Some(Self::Otlp(OtlpPlan { config: value })));
        }
        let circular_actors::ProductActor::Listener(listener) = actor else {
            return Err(RegistrationFailure::new(
                activation::UNEXPECTED_ACTOR_TYPE,
                "a listener declaration stood another actor",
            ));
        };
        let config =
            circular_actors::listener::ListenerConfig::from_value(&value).map_err(|error| {
                let message = error.to_string();
                RegistrationFailure::new(
                    circular_actors::ProductFactoryError::Listener(error).detail(),
                    message,
                )
            })?;
        let circular_actors::listener::ListenerSource::FileTail { glob, poll } = config.source();
        let unnormalized = |message: String| {
            RegistrationFailure::new(
                circular_actors::ProductFactoryError::Listener(
                    circular_actors::listener::ListenerConfigError::EmptyGlob,
                )
                .detail(),
                message,
            )
        };
        let path = circular_runtime::NormalizedPath::new(glob.as_ref())
            .map_err(|error| unnormalized(format!("glob {error:?}")))?;
        let glob = Glob::parse(glob).map_err(|error| match source::glob(&error) {
            Some(detail) => RegistrationFailure::new(detail, format!("glob {error:?}")),
            None => unnormalized(format!("glob {error:?}")),
        })?;
        let roots = circular_actors::capability_config::roots(&value, Capability::FsRead).map_err(
            |error| RegistrationFailure::new(activation::CAPABILITIES_ADMISSION, error.to_string()),
        )?;
        Ok(Some(Self::Listener(ListenerPlan {
            path,
            glob,
            poll: Duration::from_millis(poll.get().get()),
            roots,
            rewind: listener.rewind_handle(),
        })))
    }
}

impl OtlpPlan {
    pub(crate) fn start(
        &self,
        actor: &NamedActorId,
        entrance: Entrance,
        custody_root: &Path,
        stream: circular_store::StreamId,
    ) -> Result<Worker, RegistrationFailure> {
        let custody = otlp_custody(custody_root, stream, actor)?;
        let delivery = otlp_delivery(entrance.clone());
        let fallen = {
            let entrance = entrance.clone();
            move |failure: RegistrationFailure| entrance.fall(failure)
        };
        let mount = if custody.exists() {
            OtlpMount::resume(&self.config, &custody, delivery, fallen)
        } else {
            OtlpMount::activate(&self.config, &custody, delivery, fallen)
        }?;
        let refusals = entrance.clone();
        mount
            .edge()
            .arm_refusals(Arc::new(move |refusal| {
                let (inlet, subject, reason) = otlp_refusal(&refusal);
                let _ = refusals.refuse(inlet, subject, reason);
            }))
            .map_err(|message| RegistrationFailure::new(source::REFUSAL_DRAIN_FAILED, message))?;
        Ok(Worker(Some(mount)))
    }
}

pub(crate) struct Worker(Option<OtlpMount>);

impl Worker {
    pub(crate) fn pause(&self, paused: bool) {
        if let Some(mount) = &self.0 {
            mount.edge().pause_input(paused);
        }
    }

    pub(crate) fn stop(mut self) -> impl FnOnce() + Send + 'static {
        let mount = self.0.take();
        move || {
            if let Some(mount) = mount
                && let Err(error) = mount.shutdown()
            {
                eprintln!("circular-kernel: an OTLP receiver stopped with: {error}");
            }
        }
    }
}

pub(crate) struct ListenerInterpreter {
    plan: ListenerPlan,
    entrance: Entrance,
    control: Arc<Control>,
    submitted: Option<circular_runtime::EffectId>,
    settled: Arc<AtomicBool>,
    outcomes:
        std::sync::mpsc::Receiver<circular_runtime::EffectOutcome<circular_runtime::EffectId>>,
    sender: std::sync::mpsc::Sender<circular_runtime::EffectOutcome<circular_runtime::EffectId>>,
    wake: crate::direct_effect::OutcomeWake,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ListenerInterpreter {
    pub(crate) fn new(plan: ListenerPlan, entrance: Entrance, paused: bool) -> Self {
        let (sender, outcomes) = std::sync::mpsc::channel();
        Self {
            plan,
            entrance,
            control: Arc::new(Control {
                state: Mutex::new(State {
                    paused,
                    stopped: false,
                }),
                changed: Condvar::new(),
            }),
            submitted: None,
            settled: Arc::new(AtomicBool::new(false)),
            outcomes,
            sender,
            wake: crate::direct_effect::OutcomeWake::default(),
            worker: None,
        }
    }

    fn cancel(&mut self) {
        self.control.set(|state| state.stopped = true);
        if let Some(id) = self.submitted.as_ref()
            && !self.settled.swap(true, Ordering::AcqRel)
        {
            let _ = self.sender.send(circular_runtime::EffectOutcome::new(
                id.clone(),
                Err(circular_runtime::EffectFailure::InterpreterFault(
                    circular_runtime::InterpreterFault::Interrupted,
                )),
            ));
            self.wake.notify();
        }
    }
}

impl circular_runtime::Interpreter<circular_runtime::EffectId> for ListenerInterpreter {
    fn submit(
        &mut self,
        id: circular_runtime::EffectId,
        effect: &circular_runtime::Effect,
    ) -> Result<(), circular_runtime::SubmitError<circular_runtime::EffectId>> {
        use circular_runtime::{Effect, EffectFailure, EffectOutcome, InterpreterFault};
        if self.submitted.is_some() {
            return Err(circular_runtime::SubmitError::DuplicateEffectId(id));
        }
        self.submitted = Some(id.clone());
        let failure = match effect {
            _ if self.control.stopped() => Some(EffectFailure::InterpreterFault(
                InterpreterFault::Interrupted,
            )),
            Effect::FileRead { spec, .. }
                if spec.path() == &self.plan.path
                    && tail_path_allowed(&self.plan.roots, &self.plan.glob.root()) =>
            {
                None
            }
            Effect::FileRead { .. } => Some(EffectFailure::ParameterDenied {
                capability: Capability::FsRead,
            }),
            _ => Some(EffectFailure::EndpointGone),
        };
        if let Some(failure) = failure {
            self.settled.store(true, Ordering::Release);
            let _ = self.sender.send(EffectOutcome::new(id, Err(failure)));
            self.wake.notify();
            return Ok(());
        }
        self.plan.rewind.store(false, Ordering::Release);
        let tail = FileTail::new(self.plan.glob.clone(), self.plan.roots.clone());
        let poll = self.plan.poll;
        let rewind = self.plan.rewind.clone();
        let entrance = self.entrance.clone();
        let control = self.control.clone();
        let sender = self.sender.clone();
        let wake = self.wake.clone();
        let settled = self.settled.clone();
        self.worker = Some(std::thread::spawn(move || {
            let result = crate::direct_effect::caught_worker(|| {
                listen(tail, poll, &rewind, &entrance, &control);
            });
            let failure = match result {
                Err(message) => {
                    eprintln!("circular-kernel: listener effect failed: {message}");
                    EffectFailure::InterpreterFault(InterpreterFault::Other)
                }
                Ok(()) if control.stopped() => {
                    EffectFailure::InterpreterFault(InterpreterFault::Interrupted)
                }
                Ok(()) => EffectFailure::TransportTerminal,
            };
            if !settled.swap(true, Ordering::AcqRel) {
                let _ = sender.send(EffectOutcome::new(id, Err(failure)));
                wake.notify();
            }
        }));
        Ok(())
    }

    fn submit_peer(
        &mut self,
        id: circular_runtime::EffectId,
        _effect: &circular_runtime::PeerEffect,
    ) -> Result<(), circular_runtime::SubmitError<circular_runtime::EffectId>> {
        if self.submitted.is_some() {
            return Err(circular_runtime::SubmitError::DuplicateEffectId(id));
        }
        self.submitted = Some(id.clone());
        self.settled.store(true, Ordering::Release);
        let _ = self.sender.send(circular_runtime::EffectOutcome::new(
            id,
            Err(circular_runtime::EffectFailure::EndpointGone),
        ));
        self.wake.notify();
        Ok(())
    }

    fn next_outcome(
        &mut self,
    ) -> Option<circular_runtime::EffectOutcome<circular_runtime::EffectId>> {
        self.outcomes.try_recv().ok()
    }

    fn set_outcome_waker(&mut self, waker: std::task::Waker) {
        self.wake.bind(waker);
    }

    fn begin_cancel(&mut self) {
        self.cancel();
    }

    fn pause(&mut self, _force: bool) {
        self.control.set(|state| state.paused = true);
    }

    fn resume(&mut self) {
        self.control.set(|state| state.paused = false);
    }
}

impl Drop for ListenerInterpreter {
    fn drop(&mut self) {
        self.control.set(|state| state.stopped = true);
        if let Some(thread) = self.worker.take() {
            std::thread::spawn(move || {
                let _ = thread.join();
            });
        }
    }
}

#[derive(Default)]
struct State {
    paused: bool,
    stopped: bool,
}

#[derive(Default)]
struct Control {
    state: Mutex<State>,
    changed: Condvar,
}

impl Control {
    fn set(&self, change: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            change(&mut state);
        }
        self.changed.notify_all();
    }

    fn stopped(&self) -> bool {
        self.state.lock().map_or(true, |state| state.stopped)
    }

    fn open(&self) -> Option<bool> {
        let state = self.state.lock().ok()?;
        (!state.stopped).then_some(!state.paused)
    }

    fn wait(&self, interval: Duration) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        if state.stopped {
            return false;
        }
        match self.changed.wait_timeout(state, interval) {
            Ok((state, _)) => !state.stopped,
            Err(_) => false,
        }
    }
}

fn listen(
    mut tail: FileTail,
    poll: Duration,
    rewind: &AtomicBool,
    entrance: &Entrance,
    control: &Control,
) {
    let line_port = PortId::try_new("line").expect("the listener outlet");
    loop {
        match control.open() {
            None => return,
            Some(false) => {
                if !control.wait(poll) {
                    return;
                }
                continue;
            }
            Some(true) => {}
        }
        if rewind.swap(false, Ordering::AcqRel) {
            tail.replay_from_start();
        }
        let lines = tail.poll();
        for diagnostic in tail.take_diagnostics() {
            let (reason, subject) = listener_refusal(diagnostic);
            let reason = DeadLetterReason::ActorDeclared(reason.declared());
            if entrance
                .refuse(Some(line_port.clone()), subject, reason)
                .is_err()
            {
                return;
            }
        }
        for line in lines {
            let Some(payload) = transcript_payload(&line) else {
                continue;
            };
            let origin = ExternalOrigin::try_new(line_key(&line).into_bytes())
                .expect("a line key is never empty");
            if !deliver(entrance, control, poll, &line_port, payload, origin) {
                return;
            }
        }
        if !control.wait(poll) {
            return;
        }
    }
}

fn deliver(
    entrance: &Entrance,
    control: &Control,
    poll: Duration,
    port: &PortId,
    payload: ProductPayload,
    origin: ExternalOrigin,
) -> bool {
    loop {
        if control.stopped() {
            return false;
        }
        match entrance.inject_patiently(port.clone(), payload.clone(), origin.clone()) {
            Ok(_) => return true,
            Err(Refusal::NotAccepting) => {
                if !control.wait(poll) {
                    return false;
                }
            }
            Err(Refusal::Rejected(_)) => return false,
        }
    }
}

fn line_key(line: &TailedLine) -> String {
    format!(
        "{}#{}:{}#{}",
        line.path.display(),
        line.file.device,
        line.file.inode,
        line.offset
    )
}

fn listener_refusal(
    diagnostic: TailDiagnostic,
) -> (circular_actors::listener::ListenerRefusal, ProductPayload) {
    use circular_actors::listener::ListenerRefusal;
    use circular_actors::{BaseShape, Name};
    let (reason, path, offset) = match diagnostic {
        TailDiagnostic::NotUtf8 { path, offset } => (ListenerRefusal::NotUtf8, path, Some(offset)),
        TailDiagnostic::FileUnreadable { path } => (ListenerRefusal::FileUnreadable, path, None),
        TailDiagnostic::RootUnreadable { root } => (ListenerRefusal::RootUnreadable, root, None),
    };
    let mut fields = vec![(
        Name::from_static(TRANSCRIPT_LINE_PATH),
        Shape::Base(BaseShape::String),
    )];
    let mut value = vec![(
        TRANSCRIPT_LINE_PATH,
        Value::string(path.display().to_string()),
    )];
    if let Some(offset) = offset {
        fields.push((
            Name::from_static(TRANSCRIPT_LINE_OFFSET),
            Shape::Base(BaseShape::Int),
        ));
        value.push((
            TRANSCRIPT_LINE_OFFSET,
            Value::Int(i64::try_from(offset).unwrap_or(i64::MAX)),
        ));
    }
    let shape = GroundShape::try_new(Shape::Object {
        fields: FieldMap::try_new(fields).expect("distinct refusal fields"),
        open: false,
    })
    .expect("a closed object of base fields is ground");
    let value = Value::object(value).expect("distinct refusal fields");
    (reason, ProductPayload::new(shape, value))
}

fn otlp_custody(
    root: &Path,
    stream: circular_store::StreamId,
    actor: &NamedActorId,
) -> Result<PathBuf, RegistrationFailure> {
    let unavailable =
        |message: String| RegistrationFailure::new(source::CUSTODY_UNAVAILABLE, message);
    let name = circular_store::actor_value(&actor.as_actor_id())
        .map_err(|error| unavailable(error.to_string()))?;
    let bytes = circular_core::encode(
        &name,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Identity),
    )
    .map_err(|error| unavailable(format!("{error:?}")))?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let directory = root
        .join("otlp-source")
        .join(stream.get().to_string())
        .join(hex);
    if let Some(parent) = directory.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| unavailable(format!("cannot create OTLP owner directory: {error}")))?;
    }
    Ok(directory)
}

fn otlp_delivery(
    entrance: Entrance,
) -> impl FnMut(OtlpSignal, &str, &[u8]) -> OtlpDelivery + Send + 'static {
    move |signal, id, bytes| {
        let payload = match fragment_payload(bytes) {
            Ok(payload) => payload,
            Err(failure) => {
                entrance.fall(failure);
                return OtlpDelivery::Held;
            }
        };
        let port = PortId::try_new(signal.as_str()).expect("a signal name is a port name");
        let origin = ExternalOrigin::try_new(format!("otlp#{id}").into_bytes())
            .expect("an OTLP split key is never empty");
        match entrance.inject_patiently(port, payload, origin) {
            Ok(_) => OtlpDelivery::Recorded,
            Err(Refusal::NotAccepting) => OtlpDelivery::Held,
            Err(Refusal::Rejected(message)) => {
                entrance.fall(RegistrationFailure::new(source::SUBMIT_REJECTED, message));
                OtlpDelivery::Held
            }
        }
    }
}

fn fragment_payload(bytes: &[u8]) -> Result<ProductPayload, RegistrationFailure> {
    let json = serde_json::from_slice(bytes).map_err(|error| {
        RegistrationFailure::new(
            source::FRAGMENT_UNPARSABLE,
            format!("invalid scrubbed OTLP fragment: {error}"),
        )
    })?;
    let value = circular_actors::json_value(json).map_err(|error| {
        RegistrationFailure::new(
            source::FRAGMENT_VALUE_REJECTED,
            format!("OTLP value: {error:?}"),
        )
    })?;
    if !matches!(value, Value::Object(_)) {
        return Err(RegistrationFailure::new(
            source::FRAGMENT_NOT_OBJECT,
            "OTLP fragment is not an object",
        ));
    }
    Ok(ProductPayload::new(open_object(), value))
}

fn open_object() -> GroundShape {
    GroundShape::try_new(Shape::Object {
        fields: FieldMap::try_new(Vec::new()).expect("no fields"),
        open: true,
    })
    .expect("ground object")
}

fn otlp_refusal(refusal: &OtlpRefusal) -> (Option<PortId>, ProductPayload, DeadLetterReason) {
    let declared = |name: &str| {
        DeadLetterReason::ActorDeclared(
            circular_actors::otlp::OTLP_DEAD_LETTER_REASONS
                .resolve(name)
                .expect("the OTLP Source declares each of its refusal reasons"),
        )
    };
    let reason = match refusal {
        OtlpRefusal::Record { .. } => declared(circular_actors::otlp::OTLP_SCRUB_UNCLASSIFIABLE),
        OtlpRefusal::Request {
            kind: DropReason::QueueCapacity,
            ..
        } => DeadLetterReason::Capacity,
        OtlpRefusal::Request {
            kind: DropReason::ScrubFailure,
            ..
        } => declared(circular_actors::otlp::OTLP_SCRUB_UNCLASSIFIABLE),
        OtlpRefusal::Request { kind, .. } => declared(kind.as_str()),
    };
    let inlet = refusal
        .signal()
        .map(|signal| PortId::try_new(signal.as_str()).expect("a signal name is a port name"));
    let value = match refusal {
        OtlpRefusal::Record { signal, error } => Value::object([
            ("signal", Value::String(signal.as_str().into())),
            ("diagnostic", Value::String(error.to_string().into())),
        ])
        .expect("two distinct refusal fields"),
        OtlpRefusal::Request { signal, .. } => Value::object([(
            "signal",
            signal.map_or(Value::Null, |signal| Value::String(signal.as_str().into())),
        )])
        .expect("one refusal field"),
    };
    (inlet, ProductPayload::new(open_object(), value), reason)
}

pub(crate) struct Refused {
    pub(crate) inlet: Option<PortId>,
    pub(crate) subject: ProductPayload,
    pub(crate) reason: DeadLetterReason,
}
