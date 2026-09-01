use std::{collections::HashMap, sync::Arc};

use super::{ConnectionGeneration, ConnectionSnapshot, DriverHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConnectionIntentGeneration(pub(super) u64);

pub(super) struct RuntimeState<A> {
    drivers: HashMap<String, ConnectedDriver<A>>,
    current_intents: HashMap<String, ConnectionIntentGeneration>,
    next_intent_generation: u64,
}

pub(super) enum ReplacingInstallation<A> {
    Installed {
        replaced: Option<ConnectedDriver<A>>,
    },
    Superseded {
        discarded: ConnectedDriver<A>,
    },
}

pub(super) enum ExclusiveInstallation<A> {
    Installed,
    ExistingSameGeneration {
        discarded: ConnectedDriver<A>,
    },
    Occupied {
        existing_generation: ConnectionGeneration,
        discarded: ConnectedDriver<A>,
    },
}

pub(super) struct ConnectedDriver<A> {
    pub(super) handle: DriverHandle,
    pub(super) attachment: Arc<A>,
    pub(super) profile_revision: i64,
    pub(super) generation: ConnectionGeneration,
}

impl<A> Default for RuntimeState<A> {
    fn default() -> Self {
        Self {
            drivers: HashMap::new(),
            current_intents: HashMap::new(),
            next_intent_generation: 0,
        }
    }
}

impl<A> RuntimeState<A> {
    pub(super) fn advance_intent(&mut self, connection_id: &str) -> ConnectionIntentGeneration {
        self.next_intent_generation = self
            .next_intent_generation
            .checked_add(1)
            .expect("connection intent generation overflow");
        let generation = ConnectionIntentGeneration(self.next_intent_generation);
        self.current_intents
            .insert(connection_id.to_string(), generation);
        generation
    }

    pub(super) fn install_replacing_if_current(
        &mut self,
        connection_id: String,
        intent: ConnectionIntentGeneration,
        candidate: ConnectedDriver<A>,
    ) -> ReplacingInstallation<A> {
        if self.current_intents.get(&connection_id) != Some(&intent) {
            return ReplacingInstallation::Superseded {
                discarded: candidate,
            };
        }

        let replaced = self.drivers.insert(connection_id, candidate);
        if let Some(replaced) = &replaced {
            replaced.handle.retire();
        }
        ReplacingInstallation::Installed { replaced }
    }

    pub(super) fn install_exclusive(
        &mut self,
        connection_id: String,
        candidate: ConnectedDriver<A>,
    ) -> ExclusiveInstallation<A> {
        if let Some(existing) = self.drivers.get(&connection_id) {
            let existing_generation = existing.generation;
            return if existing_generation == candidate.generation {
                ExclusiveInstallation::ExistingSameGeneration {
                    discarded: candidate,
                }
            } else {
                ExclusiveInstallation::Occupied {
                    existing_generation,
                    discarded: candidate,
                }
            };
        }
        self.drivers.insert(connection_id, candidate);
        ExclusiveInstallation::Installed
    }

    pub(super) fn connection(&self, connection_id: &str) -> Option<ConnectionSnapshot<A>> {
        self.drivers
            .get(connection_id)
            .map(ConnectedDriver::snapshot)
    }

    pub(super) fn driver(&self, connection_id: &str) -> Option<DriverHandle> {
        self.drivers
            .get(connection_id)
            .map(|driver| driver.handle.clone())
    }

    pub(super) fn driver_session(&self, connection_id: &str) -> Option<(DriverHandle, u64)> {
        self.drivers
            .get(connection_id)
            .map(|driver| (driver.handle.clone(), driver.generation))
    }

    pub(super) fn session_generations(&self) -> HashMap<String, u64> {
        self.drivers
            .iter()
            .map(|(connection_id, driver)| (connection_id.clone(), driver.generation))
            .collect()
    }

    pub(super) fn is_current_driver(&self, connection_id: &str, expected: &DriverHandle) -> bool {
        self.drivers
            .get(connection_id)
            .is_some_and(|driver| driver.handle.is_same(expected))
    }

    pub(super) fn invalidate_and_detach(
        &mut self,
        connection_id: &str,
    ) -> Option<ConnectedDriver<A>> {
        self.advance_intent(connection_id);
        self.detach(connection_id)
    }

    pub(super) fn detach(&mut self, connection_id: &str) -> Option<ConnectedDriver<A>> {
        let driver = self.drivers.remove(connection_id)?;
        driver.handle.retire();
        Some(driver)
    }

    pub(super) fn detach_if_generation(
        &mut self,
        connection_id: &str,
        generation: ConnectionGeneration,
    ) -> Option<ConnectedDriver<A>> {
        if self
            .drivers
            .get(connection_id)
            .is_some_and(|driver| driver.generation == generation)
        {
            self.detach(connection_id)
        } else {
            None
        }
    }

    pub(super) fn detach_stale(
        &mut self,
        revisions: &HashMap<String, i64>,
    ) -> Vec<ConnectedDriver<A>> {
        let stale_ids = self
            .drivers
            .iter()
            .filter(|(connection_id, connected)| {
                revisions.get(*connection_id) != Some(&connected.profile_revision)
            })
            .map(|(connection_id, _)| connection_id.clone())
            .collect::<Vec<_>>();
        stale_ids
            .into_iter()
            .filter_map(|connection_id| self.detach(&connection_id))
            .collect()
    }
}

impl<A> ConnectedDriver<A> {
    fn snapshot(&self) -> ConnectionSnapshot<A> {
        ConnectionSnapshot {
            handle: self.handle.clone(),
            _attachment: self.attachment.clone(),
            profile_revision: self.profile_revision,
            generation: self.generation,
        }
    }
}
