
use circular_core::{NonZeroTicks, Tick, TickSource, TicksPerSecond, WallClockTimeSource};
use std::sync::{Arc, mpsc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Answer {
    pub(crate) generation: u64,
    pub(crate) now: Option<Tick>,
}

struct Deadline {
    at: Tick,
    generation: u64,
    reply: mpsc::Sender<Answer>,
    wake: Arc<engine::wake::Wake>,
}

pub(crate) struct SystemTime {
    deadlines: tokio::sync::mpsc::UnboundedSender<Deadline>,
}

impl SystemTime {
    pub(crate) fn stand() -> std::io::Result<Arc<Self>> {
        let (deadlines, mut mailbox) = tokio::sync::mpsc::unbounded_channel::<Deadline>();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;
        std::thread::Builder::new()
            .name("system-time".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    let source: Arc<dyn TickSource> = Arc::new(WallClockTimeSource::new(
                        NonZeroTicks::new(1_000).expect("nonzero cadence"),
                        TicksPerSecond::new(1_000).expect("millisecond resolution"),
                    ));
                    let time = Arc::new(engine::TimeActor::spawn(
                        &tokio::runtime::Handle::current(),
                        source,
                    ));
                    while let Some(deadline) = mailbox.recv().await {
                        let time = time.clone();
                        tokio::spawn(async move {
                            time.wait_until(deadline.at).await;
                            let now = time.query_tick().await;
                            let _ = deadline.reply.send(Answer {
                                generation: deadline.generation,
                                now,
                            });
                            deadline.wake.notify();
                        });
                    }
                });
            })?;
        Ok(Arc::new(Self { deadlines }))
    }

    pub(crate) fn register(
        &self,
        at: Tick,
        generation: u64,
        reply: mpsc::Sender<Answer>,
        wake: Arc<engine::wake::Wake>,
    ) -> bool {
        self.deadlines
            .send(Deadline {
                at,
                generation,
                reply,
                wake,
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_deadline_is_answered_by_message_after_it_passes() {
        let time = SystemTime::stand().expect("the time actor stands");
        let wake = Arc::new(engine::wake::Wake::new().expect("wake"));
        let (reply, answers) = mpsc::channel();
        let started = std::time::Instant::now();
        assert!(time.register(Tick::new(0), 1, reply.clone(), wake.clone()));
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        let first = answers
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first answer arrives");
        assert_eq!(first.generation, 1);
        let now = first.now.expect("the time actor answers");
        assert!(time.register(Tick::new(now.get() + 40), 2, reply, wake));
        let second = answers
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the deadline answer arrives");
        assert_eq!(second.generation, 2);
        assert!(second.now.expect("answers").get() >= now.get() + 40);
    }
}
