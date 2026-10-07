use super::*;

struct PrefixUpdate {
    authoring: Option<Arc<authoring::AuthoringState>>,
    server: Option<Option<Arc<ledger::ServerRead>>>,
    system: Option<Arc<ledger::SystemRuntime>>,
}

impl SharedWorld {
    fn return_custody(&self, update: PrefixUpdate, restore: impl FnOnce(&mut World)) {
        #[cfg(test)]
        self.note_world_lock();
        let mut world = self.writer.lock().expect("daemon world poisoned");
        #[cfg(test)]
        let held = std::time::Instant::now();
        restore(&mut world);
        self.prefix.rcu(|old| {
            let authoring = update
                .authoring
                .clone()
                .unwrap_or_else(|| old.authoring.clone());
            let server = match update.server.as_ref() {
                Some(prepared) => prepared.clone().or_else(|| {
                    old.server
                        .as_ref()
                        .filter(|run| {
                            update
                                .system
                                .as_ref()
                                .is_some_and(|system| system.stream == run.stream())
                        })
                        .cloned()
                }),
                None => old.server.clone(),
            };
            if same_arc(&authoring, &old.authoring)
                && same_option_arc(&server, &old.server)
                && same_option_arc(&update.system, &old.system)
            {
                return Arc::clone(old);
            }
            #[cfg(test)]
            self.note_prefix_publication();
            Arc::new(WorldRead {
                authoring,
                server,
                system: update.system.clone(),
                state_directory: old.state_directory.clone(),
            })
        });
        drop(world);
        #[cfg(test)]
        Self::note_world_held(held.elapsed());
    }
}

/// Mutable projection/adoption state has one owner per standing run. Input senders,
/// approval custody, file sources and immutable readers do not borrow this owner.
///
/// System control has its own mailbox and does not borrow this projection owner.
pub(crate) struct RunWrite<'a> {
    world: &'a SharedWorld,
    _projection: MutexGuard<'a, ()>,
    finished: bool,
    pub(crate) server: Option<ledger::ServerRun>,
    pub(crate) system: Option<Arc<ledger::SystemRuntime>>,
    pub(crate) authoring: Option<authoring::AuthoringState>,
    pub(crate) state_directory: std::path::PathBuf,
    pub(crate) limits: crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits,
}

impl SharedWorld {
    pub(crate) fn run_write(&self) -> Result<RunWrite<'_>, &'static str> {
        let projection = self
            .run_projection
            .lock()
            .map_err(|_| "run projection owner poisoned")?;
        #[cfg(test)]
        self.note_world_lock();
        let mut world = self.writer.lock().map_err(|_| "daemon world poisoned")?;
        #[cfg(test)]
        let held = std::time::Instant::now();
        let owner = RunWrite {
            finished: false,
            world: self,
            _projection: projection,
            server: world.server.take(),
            system: world.system.clone(),
            authoring: None,
            state_directory: world.state_directory.clone(),
            limits: world.runtime_arrival_limits,
        };
        drop(world);
        #[cfg(test)]
        Self::note_world_held(held.elapsed());
        Ok(owner)
    }
}

impl RunWrite<'_> {
    pub(crate) fn finish(mut self) {
        self.install();
    }
    fn install(&mut self) {
        if self.finished {
            return;
        }
        if let Some(run) = self.server.as_mut() {
            run.set_journal_limits(self.limits);
            if let Err(error) = run.record_journal_ceiling_health() {
                eprintln!("circular-daemon: journal ceiling health was not recorded: {error}");
            }
        }
        let published = self.world.prefix.load_full();
        let prepared = self.server.as_ref().map(|run| {
            published
                .server
                .as_ref()
                .filter(|old| run.same_read_prefix(old))
                .cloned()
                .unwrap_or_else(|| Arc::new(run.read_prefix()))
        });
        let prepared = prepared.map(|run| match run.advanced_input_prefix() {
            Some(advanced) => Arc::new(advanced),
            None => run,
        });
        let (authoring_state, authoring_read) = match self.authoring.take() {
            Some(state) => {
                let read = Arc::new(state.clone());
                (Some(state), Some(read))
            }
            None => (None, None),
        };
        let server = self.server.take();
        let system = self.system.clone();
        self.world.return_custody(
            PrefixUpdate {
                authoring: authoring_read,
                server: Some(prepared),
                system: system.clone(),
            },
            move |world| {
                world.server = server;
                if let Some(state) = authoring_state {
                    world.authoring = state;
                }
                world.system = system;
            },
        );
        self.finished = true;
        self.world.wake_sessions();
    }
}
impl Drop for RunWrite<'_> {
    fn drop(&mut self) {
        self.install();
    }
}

impl RunWrite<'_> {
    pub(crate) fn parts(
        &mut self,
    ) -> (
        &mut Option<ledger::ServerRun>,
        &mut Option<Arc<ledger::SystemRuntime>>,
    ) {
        (&mut self.server, &mut self.system)
    }
    pub(crate) fn state_directory(&self) -> &std::path::Path {
        &self.state_directory
    }
    pub(crate) fn set_authoring(&mut self, state: Option<authoring::AuthoringState>) {
        self.authoring = state;
    }
    pub(crate) fn world_authoring_cursor(&self) -> u64 {
        self.world.read().authoring.cursor()
    }
}

fn same_arc<T>(left: &Arc<T>, right: &Arc<T>) -> bool {
    Arc::ptr_eq(left, right)
}

fn same_option_arc<T>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}
