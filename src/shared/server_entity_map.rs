use bevy::{
    ecs::entity::{EntityHash, hash_map::EntityHashMap},
    platform::collections::hash_map::{self, Entry},
    prelude::*,
};
use log::{error, warn};

/// Maps server entities to client entities and vice versa.
///
/// Inserted as resource by [`ClientPlugin`](crate::client::ClientPlugin).
///
/// Automatically updated on replication receive. If the client predicts a despawn,
/// the mapping is kept until the server also despawns the entity.
#[derive(Resource, Default)]
pub struct ServerEntityMap {
    server_to_client: EntityHashMap<Entity>,
    client_to_server: EntityHashMap<Entity>,
}

impl ServerEntityMap {
    /// Inserts a server-client pair into the map.
    pub(crate) fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        if let Some(existing_entity) = self.server_to_client.insert(server_entity, client_entity) {
            if client_entity != existing_entity {
                error!(
                    "mapping {server_entity:?} to {client_entity:?}, but it's already mapped to {existing_entity:?}"
                );
                self.client_to_server.remove(&existing_entity);
            } else {
                warn!("ignoring duplicate mapping from {server_entity:?} to {client_entity:?}");
            }
        }

        self.client_to_server.insert(client_entity, server_entity);
    }

    /// Returns server to client mappings.
    #[inline]
    pub fn to_client(&self) -> &EntityHashMap<Entity> {
        &self.server_to_client
    }

    /// Returns client to server mappings.
    #[inline]
    pub fn to_server(&self) -> &EntityHashMap<Entity> {
        &self.client_to_server
    }

    /// Gets a server entry using the client entity.
    pub(crate) fn server_entry(&mut self, server_entity: Entity) -> EntityEntry<'_> {
        EntityEntry::new(
            self.server_to_client.entry(server_entity),
            &mut self.client_to_server,
        )
    }

    /// Clears the map.
    pub(crate) fn clear(&mut self) {
        self.client_to_server.clear();
        self.server_to_client.clear();
    }
}

/// A view into an entry in [`ServerEntityMap`].
#[must_use]
pub enum EntityEntry<'a> {
    Occupied(OccupiedEntityEntry<'a>),
    Vacant(VacantEntityEntry<'a>),
}

impl<'a> EntityEntry<'a> {
    fn new(
        main_entry: Entry<'a, Entity, Entity, EntityHash>,
        reverse_map: &'a mut EntityHashMap<Entity>,
    ) -> Self {
        match main_entry {
            Entry::Occupied(main_entry) => Self::Occupied(OccupiedEntityEntry {
                main_entry,
                reverse_map,
            }),
            Entry::Vacant(main_entry) => Self::Vacant(VacantEntityEntry {
                main_entry,
                reverse_map,
            }),
        }
    }

    /// Returns the mappend entity for the entry.
    pub fn get(&self) -> Option<Entity> {
        match self {
            EntityEntry::Occupied(entry) => Some(entry.get()),
            EntityEntry::Vacant(_) => None,
        }
    }

    /// Removes the entry and returns the mapped entity.
    pub fn remove(self) -> Option<Entity> {
        match self {
            EntityEntry::Occupied(entry) => Some(entry.remove()),
            EntityEntry::Vacant(_) => None,
        }
    }

    /// Inserts a new mapping from the function if the entry is not mapped, and returns the mapped entity.
    pub fn or_insert_with<F: FnOnce() -> Entity>(self, f: F) -> Entity {
        match self {
            EntityEntry::Occupied(entry) => entry.get(),
            EntityEntry::Vacant(entry) => entry.insert(f()),
        }
    }
}

/// A view into an occupied entry in [`ServerEntityMap`].
///
/// It's part of [`EntityEntry`] enum.
pub struct OccupiedEntityEntry<'a> {
    main_entry: hash_map::OccupiedEntry<'a, Entity, Entity, EntityHash>,
    reverse_map: &'a mut EntityHashMap<Entity>,
}

impl OccupiedEntityEntry<'_> {
    /// Returns the mappend entity for the entry.
    pub fn get(&self) -> Entity {
        *self.main_entry.get()
    }

    /// Removes the entry and returns the mapped entity.
    pub fn remove(self) -> Entity {
        let (_, value) = self.main_entry.remove_entry();
        self.reverse_map.remove(&value);
        value
    }
}

/// A view into a vacant entry in [`ServerEntityMap`].
///
/// It's part of [`EntityEntry`] enum.
pub struct VacantEntityEntry<'a> {
    main_entry: hash_map::VacantEntry<'a, Entity, Entity, EntityHash>,
    reverse_map: &'a mut EntityHashMap<Entity>,
}

impl VacantEntityEntry<'_> {
    /// Sets the mapped entity for the entry and returns it.
    pub fn insert(self, value: Entity) -> Entity {
        let key = *self.main_entry.key();
        self.main_entry.insert(value);
        self.reverse_map.insert(value, key);
        value
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn mapping() {
        const SERVER_ENTITY: Entity = Entity::from_raw_u32(0).unwrap();
        const CLIENT_ENTITY: Entity = Entity::from_raw_u32(1).unwrap();

        let mut map = ServerEntityMap::default();
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), None);

        map.insert(SERVER_ENTITY, Entity::PLACEHOLDER);
        assert_eq!(
            map.server_entry(SERVER_ENTITY).get(),
            Some(Entity::PLACEHOLDER)
        );

        map.insert(SERVER_ENTITY, CLIENT_ENTITY);
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), Some(CLIENT_ENTITY));

        assert_eq!(
            map.server_entry(SERVER_ENTITY).remove(),
            Some(CLIENT_ENTITY)
        );
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), None);

        assert_eq!(
            map.server_entry(SERVER_ENTITY)
                .or_insert_with(|| CLIENT_ENTITY),
            CLIENT_ENTITY
        );
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), Some(CLIENT_ENTITY));

        assert_eq!(
            map.server_entry(SERVER_ENTITY)
                .or_insert_with(|| Entity::PLACEHOLDER),
            CLIENT_ENTITY
        );
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), Some(CLIENT_ENTITY));
    }
}
