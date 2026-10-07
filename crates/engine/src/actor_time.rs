use circular_core::{Tick, TickSource, Ticks};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{mpsc, oneshot};

type TimeQuery = oneshot::Sender<Tick>;

pub struct TimeActor {
    queries: arc_swap::ArcSwap<mpsc::UnboundedSender<TimeQuery>>,
    runtime: tokio::runtime::Handle,
    declared: Arc<dyn TickSource>,
    successor_used: AtomicBool,
    successor_stood: AtomicBool,
}

impl TimeActor {
    pub fn spawn(handle: &tokio::runtime::Handle, source: Arc<dyn TickSource>) -> Self {
        let (queries, join) = Self::stand_task(handle, &source);
        drop(join);
        Self {
            queries: arc_swap::ArcSwap::from_pointee(queries),
            runtime: handle.clone(),
            declared: source,
            successor_used: AtomicBool::new(false),
            successor_stood: AtomicBool::new(false),
        }
    }

    fn stand_task(
        handle: &tokio::runtime::Handle,
        source: &Arc<dyn TickSource>,
    ) -> (
        mpsc::UnboundedSender<TimeQuery>,
        tokio::task::JoinHandle<()>,
    ) {
        let (queries, mut mailbox) = mpsc::unbounded_channel::<TimeQuery>();
        let sampling = source.clone();
        let join = handle.spawn(async move {
            while let Some(reply) = mailbox.recv().await {
                let _ = reply.send(sampling.current_tick());
            }
        });
        (queries, join)
    }

    #[must_use]
    pub fn successor_stood(&self) -> bool {
        self.successor_stood.load(Ordering::SeqCst)
    }

    async fn ask(&self) -> Option<Tick> {
        let (reply, answer) = oneshot::channel();
        self.queries.load().send(reply).ok()?;
        answer.await.ok()
    }

    fn stand_successor(&self) -> bool {
        if self.successor_used.swap(true, Ordering::SeqCst) {
            return false;
        }
        let (queries, join) = Self::stand_task(&self.runtime, &self.declared);
        if join.is_finished() {
            return false;
        }
        drop(join);
        eprintln!(
            "circular: the time actor task ended while its mailbox was open; a successor stands"
        );
        self.successor_stood.store(true, Ordering::SeqCst);
        self.queries.store(Arc::new(queries));
        true
    }

    pub fn query_tick(&self) -> Pin<Box<dyn Future<Output = Option<Tick>> + Send + '_>> {
        Box::pin(async move {
            if let Some(tick) = self.ask().await {
                return Some(tick);
            }
            if !self.stand_successor() {
                return None;
            }
            self.ask().await
        })
    }

    pub fn wait_until(&self, at: Tick) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            while self.query_tick().await.is_some_and(|now| now < at) {
                self.declared.wait_until(at).await;
            }
        })
    }

    #[must_use]
    pub fn heartbeat_cadence(&self) -> Ticks {
        self.declared.heartbeat_cadence()
    }

    #[must_use]
    pub fn ticks_per_second(&self) -> u32 {
        self.declared.ticks_per_second()
    }

    #[must_use]
    pub fn is_replay(&self) -> bool {
        self.declared.is_replay()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{ManualTimeSource, NonZeroTicks, RecordedInstant, TicksPerSecond};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn manual(start: u64) -> Arc<ManualTimeSource> {
        Arc::new(ManualTimeSource::new(
            Tick::new(start),
            NonZeroTicks::new(1).unwrap(),
            TicksPerSecond::new(1_000).unwrap(),
        ))
    }

    #[test]
    fn an_actor_awaiting_its_time_answer_does_not_stop_another_actor() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let source = manual(11);
        let time = Arc::new(TimeActor::spawn(runtime.handle(), source.clone()));
        runtime.block_on(async move {
            let arrivals = Arc::new(AtomicUsize::new(0));
            let asking = {
                let time = time.clone();
                let arrivals = arrivals.clone();
                tokio::task::spawn(async move {
                    let mut last = Tick::ZERO;
                    for _ in 0..100 {
                        last = time.query_tick().await.expect("the time actor is standing");
                        tokio::task::yield_now().await;
                    }
                    assert!(
                        arrivals.load(Ordering::SeqCst) > 0,
                        "a sibling actor never ran while waiting for the query"
                    );
                    last
                })
            };
            let sibling = {
                let arrivals = arrivals.clone();
                tokio::task::spawn(async move {
                    for _ in 0..100 {
                        arrivals.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                    }
                })
            };
            assert_eq!(asking.await.expect("asking actor"), Tick::new(11));
            sibling.await.expect("sibling actor");
            assert_eq!(arrivals.load(Ordering::SeqCst), 100);
        });
    }

    struct DyingSource {
        inner: Arc<ManualTimeSource>,
        reads: AtomicUsize,
        deaths: std::ops::Range<usize>,
    }

    impl TickSource for DyingSource {
        fn current_tick(&self) -> Tick {
            let read = self.reads.fetch_add(1, Ordering::SeqCst);
            assert!(!self.deaths.contains(&read), "sample source died at {read}");
            self.inner.current_tick()
        }
        fn wait_until(&self, tick: Tick) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.inner.wait_until(tick)
        }
        fn heartbeat_cadence(&self) -> Ticks {
            self.inner.heartbeat_cadence()
        }
        fn ticks_per_second(&self) -> u32 {
            self.inner.ticks_per_second()
        }
    }

    fn dying(deaths: std::ops::Range<usize>) -> Arc<DyingSource> {
        Arc::new(DyingSource {
            inner: manual(23),
            reads: AtomicUsize::new(0),
            deaths,
        })
    }
}
