use crate::daemon::declaration::malformed;
use crate::daemon::ledger;
use circular_core::{Boundary, Ceilings};
use circular_protocol::Partition;
use circular_protocol::declaration_payload::{Accepted, CommandResult, Rejected, decode_inject};
use circular_protocol::rejection_code::RejectionReason;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InjectionDisposition {
    Accepted,
    NoRun,
    UnknownMount,
    NotAccepting,
    Failed,
}

pub(crate) struct InjectionAnswer {
    disposition: InjectionDisposition,
    command_result: CommandResult,
}

impl InjectionAnswer {
    fn accepted() -> Self {
        Self {
            disposition: InjectionDisposition::Accepted,
            command_result: CommandResult::Accepted(Accepted::Nothing),
        }
    }

    fn failed(rejection: Rejected) -> Self {
        Self {
            disposition: InjectionDisposition::Failed,
            command_result: CommandResult::Rejected(rejection),
        }
    }

    fn no_run(rejection: Rejected) -> Self {
        Self {
            disposition: InjectionDisposition::NoRun,
            command_result: CommandResult::Rejected(rejection),
        }
    }

    fn unknown_mount(rejection: Rejected) -> Self {
        Self {
            disposition: InjectionDisposition::UnknownMount,
            command_result: CommandResult::Rejected(rejection),
        }
    }

    fn not_accepting(rejection: Rejected) -> Self {
        Self {
            disposition: InjectionDisposition::NotAccepting,
            command_result: CommandResult::Rejected(rejection),
        }
    }

    fn malformed(message: String) -> Self {
        Self {
            disposition: InjectionDisposition::Failed,
            command_result: malformed(message),
        }
    }

    pub(crate) const fn disposition(&self) -> InjectionDisposition {
        self.disposition
    }

    pub(crate) fn into_command_result(self) -> CommandResult {
        self.command_result
    }
}

pub(crate) fn answer_ingress(
    payload: &[u8],
    server: Option<&ledger::ServerIngress>,
) -> InjectionAnswer {
    match decode_inject(payload, Ceilings::for_boundary(Boundary::Wire)) {
        Ok(inject) => answer_decoded_ingress(inject, server),
        Err(rejection) => {
            InjectionAnswer::malformed(format!("the Inject payload is malformed: {rejection}"))
        }
    }
}

pub(crate) fn answer_decoded_ingress(
    inject: circular_protocol::declaration_payload::Inject,
    server: Option<&ledger::ServerIngress>,
) -> InjectionAnswer {
    let mount = circular_core::spelling::Quoted(&inject.mount.to_string()).to_string();
    match server {
        None => InjectionAnswer::no_run(RejectionReason::Unresolved.reject(
            Partition::EventInjection,
            format!("no mount named {} is registered", mount),
        )),
        Some(run) => {
            match circular_runtime::ExternalOrigin::try_new(inject.idempotency.clone()) {
                Err(_) => InjectionAnswer::failed(RejectionReason::Malformed.reject(Partition::EventInjection, "idempotency key is empty — injection without an origin cannot create an arrival"
                        .to_owned())),
                Ok(origin) => {
                    let payload = circular_actors::ProductPayload::new(
                        circular_actors::GroundShape::try_new(circular_actors::Shape::Any)
                            .expect("Any is ground"),
                        inject.payload.clone(),
                    );
                    match run.inject(&inject.mount, payload, origin) {
                        Ok(()) => {
                            InjectionAnswer::accepted()
                        }
                        Err(crate::daemon::ledger::IngressRefusal::UnknownMount(
                            reason,
                        )) => InjectionAnswer::unknown_mount(RejectionReason::Unresolved.reject(Partition::EventInjection, format!(
                                "no mount named {} is registered: {reason}",
                                mount
                            ))),
                        Err(crate::daemon::ledger::IngressRefusal::NotAccepting) => {
                            InjectionAnswer::not_accepting(Rejected {
                                code: RejectionReason::InputNotAccepted
                                    .number_in(Partition::EventInjection),
                                message: format!(
                                    "mount {} resolved, but its source owner has recorded a pause and is not accepting input; Resume reopens it",
                                    mount
                                ),
                                hint: Some(
                                    "resend after Resume — this arrival was not recorded"
                                        .to_owned(),
                                ),
                                at: None,
                            })
                        }
                        Err(crate::daemon::ledger::IngressRefusal::OutOfDomain(reason)) => {
                            eprintln!(
                                "circular-daemon: Inject — mount {} refused: code {}",
                                mount,
                                RejectionReason::Malformed.number_in(Partition::EventInjection)
                            );
                            InjectionAnswer::failed(RejectionReason::Malformed.reject(Partition::EventInjection, format!(
                                    "mount {} resolved, but {reason}",
                                    mount
                                )).hint(
                                    "send a value in the domain this mount declares — this arrival was not recorded"
                                        .to_owned(),
                                ))
                        }
                        Err(crate::daemon::ledger::IngressRefusal::Storage(reason)) => {
                            match run.recorder_stop() {
                                Some(code) => {
                                    InjectionAnswer::failed(Rejected {
                                        code: code.number_in(Partition::EventInjection),
                                        message: format!(
                                            "mount {} resolved, but {}: {reason}",
                                            mount,
                                            code.message()
                                        ),
                                        hint: None,
                                        at: None,
                                    })
                                }
                                None => {
                                    eprintln!(
                                        "circular-daemon: Inject — mount {} refused: code {}: {reason}",
                                        mount,
                                        RejectionReason::Unresolved.number_in(Partition::EventInjection)
                                    );
                                    InjectionAnswer::failed(RejectionReason::Unresolved.reject(Partition::EventInjection, format!(
                                        "mount {} resolved, but this arrival was not committed: {reason}",
                                        mount
                                    )))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

