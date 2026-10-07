use super::*;
impl SharedWorld {
    pub(crate) fn inject(&self, payload: &[u8]) -> crate::daemon::injection::InjectionAnswer {
        let prefix = self.read();
        let input = prefix.server.as_ref().and_then(|run| run.ingress());
        let answer = crate::daemon::injection::answer_ingress(payload, input);
        self.publish_input_prefix();
        answer
    }
    pub(crate) fn inject_decoded(
        &self,
        request: circular_protocol::declaration_payload::Inject,
    ) -> crate::daemon::injection::InjectionAnswer {
        let prefix = self.read();
        let input = prefix.server.as_ref().and_then(|run| run.ingress());
        let answer = crate::daemon::injection::answer_decoded_ingress(request, input);
        self.publish_input_prefix();
        answer
    }
    fn publish_input_prefix(&self) {
        self.prefix.rcu(|old| {
            let Some(advanced) = old
                .server
                .as_ref()
                .and_then(|run| run.advanced_input_prefix())
            else {
                return Arc::clone(old);
            };
            #[cfg(test)]
            self.note_prefix_publication();
            let mut next = (**old).clone();
            next.server = Some(Arc::new(advanced));
            Arc::new(next)
        });
    }
    pub(crate) fn start_input_owners(
        self: &Arc<Self>,
        gateway: Option<crate::daemon::webhook::WebhookGateway>,
    ) -> Result<Option<crate::daemon::webhook::WebhookClosing>, String> {
        let Some(gateway) = gateway else {
            return Ok(None);
        };
        let closing = gateway.closing();
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
                .name("webhook-input".into())
                .spawn(move || {
                    while let Ok(request) = gateway.recv() {
                        let Some(world) = weak.upgrade() else { break };
                        if let Err(error) = std::thread::Builder::new()
                            .name("webhook-admission".into())
                            .spawn(move || {
                                let adapter =
                                    crate::daemon::webhook::WebhookInjectAdapter::new(request.mount());
                                let answer = match adapter
                                    .envelope(request.body(), request.idempotency())
                                {
                                    Ok(envelope) => match world
                                        .inject(envelope.decode().payload())
                                        .disposition()
                                    {
                                        crate::daemon::injection::InjectionDisposition::Accepted => {
                                            crate::daemon::webhook::WebhookResponse::Accepted
                                        }
                                        crate::daemon::injection::InjectionDisposition::NoRun => {
                                            crate::daemon::webhook::WebhookResponse::NoRun
                                        }
                                        crate::daemon::injection::InjectionDisposition::UnknownMount => {
                                            crate::daemon::webhook::WebhookResponse::UnknownMount
                                        }
                                        crate::daemon::injection::InjectionDisposition::NotAccepting => {
                                            crate::daemon::webhook::WebhookResponse::NotAccepting
                                        }
                                        crate::daemon::injection::InjectionDisposition::Failed => {
                                            crate::daemon::webhook::WebhookResponse::Failed
                                        }
                                    },
                                    Err(reason) => {
                                        eprintln!("circular-daemon: failed to build webhook Inject envelope: {reason}");
                                        crate::daemon::webhook::WebhookResponse::Failed
                                    },
                                };
                                request.answer(answer);
                            })
                        {
                            eprintln!("circular-daemon: webhook admission owner: {error}");
                        }
                    }
                })
                .map_err(|e| format!("webhook input owner: {e}"))?;
        Ok(Some(closing))
    }
}
