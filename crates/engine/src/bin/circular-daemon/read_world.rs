//! A published immutable prefix, separate from append/write coordination.
use crate::daemon::{authoring, ledger, platform::World};
use arc_swap::ArcSwap;
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(test)]
use std::ops::{Deref, DerefMut};

#[derive(Clone)]
pub(crate) struct WorldRead {
    pub(crate) authoring: Arc<authoring::AuthoringState>,
    pub(crate) server: Option<Arc<ledger::ServerRead>>,
    pub(crate) system: Option<Arc<ledger::SystemRuntime>>,
    pub(crate) state_directory: std::path::PathBuf,
}
impl WorldRead {
    fn capture(world: &mut World, previous: Option<&Self>) -> Self {
        if let Some(run) = world.server.as_mut() {
            run.set_journal_limits(world.runtime_arrival_limits);
        }
        let same_authoring =
            previous.filter(|old| old.authoring.cursor() == world.authoring.cursor());
        let authoring = same_authoring
            .map(|old| old.authoring.clone())
            .unwrap_or_else(|| Arc::new(world.authoring.clone()));
        let server = world
            .server
            .as_ref()
            .map(|run| {
                previous
                    .and_then(|old| old.server.as_ref())
                    .filter(|old| run.same_read_prefix(old))
                    .cloned()
                    .unwrap_or_else(|| Arc::new(run.read_prefix()))
            })
            .or_else(|| {
                previous
                    .and_then(|old| old.server.as_ref())
                    .filter(|read| {
                        world
                            .system
                            .as_ref()
                            .is_some_and(|system| system.stream == read.stream())
                    })
                    .cloned()
            });
        Self {
            authoring,
            server,
            system: world.system.clone(),
            state_directory: world.state_directory.clone(),
        }
    }
}
pub(crate) struct SharedWorld {
    writer: Mutex<World>,
    commits: Mutex<()>,
    run_projection: Mutex<()>,
    #[cfg(test)]
    lock_attempts: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    prefix_publications: std::sync::atomic::AtomicUsize,
    prefix: ArcSwap<WorldRead>,
    session_wakes: Mutex<Vec<std::sync::Weak<engine::wake::Wake>>>,
}
#[cfg(test)]
thread_local! {
    static THREAD_WORLD_LOCKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static THREAD_WORLD_HELD: std::cell::Cell<u128> = const { std::cell::Cell::new(0) };
}
impl SharedWorld {
    #[cfg(test)]
    pub(crate) fn thread_lock_attempts() -> usize {
        THREAD_WORLD_LOCKS.get()
    }
    #[cfg(test)]
    pub(crate) fn thread_world_held_ns() -> u128 {
        THREAD_WORLD_HELD.get()
    }
    #[cfg(test)]
    fn note_world_held(duration: std::time::Duration) {
        THREAD_WORLD_HELD.set(THREAD_WORLD_HELD.get() + duration.as_nanos());
    }
    #[cfg(test)]
    fn note_world_lock(&self) {
        THREAD_WORLD_LOCKS.set(THREAD_WORLD_LOCKS.get() + 1);
        self.lock_attempts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(test)]
    fn note_prefix_publication(&self) {
        self.prefix_publications
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(test)]
    pub(crate) fn prefix_publications(&self) -> usize {
        self.prefix_publications
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn new(world: World) -> Self {
        Self::with_restored_prefix(world, None)
    }
    pub(crate) fn with_restored_prefix(
        mut world: World,
        restored: Option<ledger::ServerRead>,
    ) -> Self {
        let mut read = WorldRead::capture(&mut world, None);
        if read.server.is_none() {
            read.server = restored.map(Arc::new);
        }
        let prefix = ArcSwap::from_pointee(read);
        Self {
            writer: Mutex::new(world),
            commits: Mutex::new(()),
            run_projection: Mutex::new(()),
            prefix,
            #[cfg(test)]
            lock_attempts: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            prefix_publications: std::sync::atomic::AtomicUsize::new(0),
            session_wakes: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn register_session_wake(&self, wake: &std::sync::Arc<engine::wake::Wake>) {
        let mut wakes = self
            .session_wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        wakes.retain(|registered| registered.strong_count() > 0);
        wakes.push(std::sync::Arc::downgrade(wake));
    }

    pub(crate) fn wake_sessions(&self) {
        let wakes = self
            .session_wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for registered in wakes.iter() {
            if let Some(wake) = registered.upgrade() {
                wake.notify();
            }
        }
    }
    pub(crate) fn commit_gate(&self) -> Result<MutexGuard<'_, ()>, &'static str> {
        self.commits
            .lock()
            .map_err(|_| "daemon commit gate poisoned")
    }
    /// Atomic Arc acquisition; never acquires or probes the world writer mutex.
    pub(crate) fn read(&self) -> Arc<WorldRead> {
        let world = self.prefix.load_full();
        match world.server.as_ref().and_then(|server| server.caught_up()) {
            Some(server) => Arc::new(WorldRead {
                server: Some(Arc::new(server)),
                ..(*world).clone()
            }),
            None => world,
        }
    }
    #[cfg(test)]
    pub(crate) fn lock(&self) -> Result<WriteGuard<'_>, &'static str> {
        #[cfg(test)]
        self.note_world_lock();
        let projection = self
            .run_projection
            .lock()
            .map_err(|_| "run projection owner poisoned")?;
        self.writer
            .lock()
            .map(|guard| WriteGuard {
                _projection: projection,
                acquired: std::time::Instant::now(),
                guard,
                prefix: &self.prefix,
                publications: &self.prefix_publications,
            })
            .map_err(|_| "daemon world poisoned")
    }
    #[cfg(test)]
    pub(crate) fn lock_attempts(&self) -> usize {
        self.lock_attempts
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(test)]
    pub(crate) fn try_lock(&self) -> Result<WriteGuard<'_>, &'static str> {
        let projection = self
            .run_projection
            .try_lock()
            .map_err(|_| "run projection unavailable")?;
        self.writer
            .try_lock()
            .map(|guard| WriteGuard {
                _projection: projection,
                acquired: std::time::Instant::now(),
                guard,
                prefix: &self.prefix,
                publications: &self.prefix_publications,
            })
            .map_err(|_| "world unavailable")
    }
}
#[cfg(test)]
pub(crate) struct WriteGuard<'a> {
    guard: MutexGuard<'a, World>,
    _projection: MutexGuard<'a, ()>,
    acquired: std::time::Instant,
    prefix: &'a ArcSwap<WorldRead>,
    publications: &'a std::sync::atomic::AtomicUsize,
}
#[cfg(test)]
impl Deref for WriteGuard<'_> {
    type Target = World;
    fn deref(&self) -> &World {
        &self.guard
    }
}
#[cfg(test)]
impl DerefMut for WriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut World {
        &mut self.guard
    }
}
#[cfg(test)]
impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        let old = self.prefix.load_full();
        self.publications
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.prefix
            .store(Arc::new(WorldRead::capture(&mut self.guard, Some(&old))));
        SharedWorld::note_world_held(self.acquired.elapsed());
    }
}

#[path = "input_owners.rs"]
mod input_owners;

#[path = "run_write_owner.rs"]
mod run_write_owner;
