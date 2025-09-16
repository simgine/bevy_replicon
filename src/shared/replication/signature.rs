use core::{
    any::{self},
    hash::{Hash, Hasher},
};

use bevy::{
    ecs::{component::HookContext, entity::EntityHashMap, world::DeferredWorld},
    platform::{collections::HashMap, hash::NoOpHash},
    prelude::*,
};
use fnv::FnvHasher;
use log::error;

use crate::prelude::*;

/// Describes how to calculate a deterministic hash that identifies the entity.
///
/// When client receives replication it maps server entities to its own entities
/// using [`ServerEntityMap`](crate::shared::server_entity_map::ServerEntityMap).
/// If there is no mapping, it spawns a new entity and creates a new mapping to it.
///
/// However, sometimes it's needed to spawn something on the client first without
/// waiting for replication from the server. Without this component the client will
/// end up with 2 entities: locally spawned and replicated from the server.
///
/// This component inserted on both client and server allows to match client-spawned
/// entities with replicated from the server by comparing their hashes.
///
/// This also useful to synrhconizing scenes. Both client and server could load a level
/// independently and then match level entities to synchronize certain things, such as
/// opened doors.
///
///
#[derive(Component)]
#[component(immutable, on_add = register_hash, on_remove = unregister_hash)]
pub struct Signature {
    /// User-defined initial state for the hash.
    base_hash: Option<u64>,

    /// Functions to calculate hash from components.
    fns: &'static [HashFn],

    client: Option<Entity>,
}

impl Signature {
    #[must_use]
    pub fn of_single<C: Component + Hash>() -> Self {
        Self {
            base_hash: None,
            fns: &[hash::<C>],
            client: None,
        }
    }

    #[must_use]
    pub fn of<S: SignatureComponents>() -> Self {
        Self {
            base_hash: None,
            fns: S::HASH_FNS,
            client: None,
        }
    }

    #[must_use]
    pub fn with_base<T: Hash>(mut self, value: T) -> Self {
        let mut hasher = FnvHasher::default();
        value.hash(&mut hasher);

        self.base_hash = Some(hasher.finish());
        self
    }

    #[must_use]
    pub fn with_client(mut self, client: Entity) -> Self {
        self.client = Some(client);
        self
    }

    #[must_use]
    fn hash<'w>(&self, entity: impl Into<EntityRef<'w>>) -> u64 {
        let mut hasher = self
            .base_hash
            .map(|hash| FnvHasher::with_key(hash))
            .unwrap_or_default();

        let entity = entity.into();
        for hash_fn in self.fns {
            (hash_fn)(&entity, &mut hasher);
        }

        hasher.finish()
    }
}

impl<T: Hash> From<T> for Signature {
    fn from(value: T) -> Self {
        let mut hasher = FnvHasher::default();
        value.hash(&mut hasher);

        Self {
            base_hash: Some(hasher.finish()),
            fns: &[],
            client: None,
        }
    }
}

fn register_hash(mut world: DeferredWorld, ctx: HookContext) {
    let entity = world.entity(ctx.entity);
    let signature = entity.get::<Signature>().unwrap();
    let hash = signature.hash(entity);

    if let Some(client) = signature.client {
        if let Some(mut pending) = world.get_mut::<MappingsBuffer>(client) {
            pending.push((ctx.entity, hash));
        } else {
            error!("trying to add a signature for a non-authorized client `{client}`");
        }
    } else {
        if *world.resource::<State<ServerState>>() == ServerState::Running {
            let mut pending = world.resource_mut::<MappingsBuffer>();
            pending.push((ctx.entity, hash));
        }

        let mut map = world.resource_mut::<SignatureMap>();
        map.insert(ctx.entity, hash);
    }
}

fn unregister_hash(mut world: DeferredWorld, ctx: HookContext) {
    let mut map = world.resource_mut::<SignatureMap>();
    map.remove(ctx.entity);
}

/// Server entities and their associated hashes from the [`Signature`] component,
/// calculated during this tick.
///
/// When used as a component on a client entity, it is specific to that client.
/// When used as a resource, it is global to all clients.
#[derive(Resource, Component, Deref, DerefMut, Debug, Default)]
pub(crate) struct MappingsBuffer(pub(super) Vec<(Entity, u64)>);

/// Stores hashes calculated from the [`Signature`] component and maps them
/// to their entities in both directions.
///
/// Contains hashes only for global signatures.
///
/// Automatically updated via hooks.
#[derive(Resource, Default)]
pub(crate) struct SignatureMap {
    to_hashes: EntityHashMap<u64>,
    to_entities: HashMap<u64, Entity, NoOpHash>, // Skips hashing because the key is already a hash.
}

impl SignatureMap {
    pub(crate) fn iter(&self) -> impl Iterator<Item = (Entity, u64)> {
        self.to_hashes.iter().map(|(&e, &h)| (e, h))
    }

    pub(crate) fn len(&self) -> usize {
        self.to_hashes.len()
    }

    pub(crate) fn get(&self, hash: u64) -> Option<Entity> {
        self.to_entities.get(&hash).copied()
    }

    fn insert(&mut self, entity: Entity, hash: u64) {
        match self.to_entities.try_insert(hash, entity) {
            Ok(_) => {
                self.to_hashes.insert(entity, hash);
            }
            Err(e) => error!(
                "hash for `{entity}` marches `{}` and will be ignored",
                e.value
            ),
        }
    }

    fn remove(&mut self, entity: Entity) {
        if let Some(hash) = self.to_hashes.remove(&entity) {
            self.to_entities.remove(&hash);
        }
    }
}

pub trait SignatureComponents {
    const HASH_FNS: &'static [HashFn];
}

type HashFn = fn(&EntityRef, &mut FnvHasher);

fn hash<C: Component + Hash>(entity: &EntityRef, hasher: &mut FnvHasher) {
    let type_name = any::type_name::<C>();
    type_name.hash(hasher);
    if let Some(component) = entity.get::<C>() {
        component.hash(hasher);
    } else {
        error!(
            "unable to get `{type_name}` from `{}` to calculate hash",
            entity.id()
        );
    }
}

macro_rules! impl_signature_components {
    ($($C:ident),*) => {
        impl<$($C: Component + Hash),*> SignatureComponents for ($($C,)*) {
            const HASH_FNS: &'static [HashFn] = &[
                $(hash::<$C>,)*
            ];
        }
    };
}

variadics_please::all_tuples!(impl_signature_components, 0, 15, C);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_component() {
        let mut world = World::new();

        let entity1 = world.spawn(C(true)).id();
        let entity2 = world.spawn(C(true)).id();
        let entity3 = world.spawn(C(false)).id();
        let entity4 = world.spawn(A).id();

        let signature = Signature::of_single::<C>();
        let hash1 = signature.hash(world.entity(entity1));
        let hash2 = signature.hash(world.entity(entity2));
        let hash3 = signature.hash(world.entity(entity3));
        let hash4 = signature.hash(world.entity(entity4));
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_ne!(hash3, hash4);
    }

    #[test]
    fn multiple_components() {
        let mut world = World::new();

        let entity1 = world.spawn((A, C(true))).id();
        let entity2 = world.spawn((A, C(true))).id();
        let entity3 = world.spawn((A, C(false))).id();
        let entity4 = world.spawn(A).id();

        let signature = Signature::of::<(A, C)>();
        let hash1 = signature.hash(world.entity(entity1));
        let hash2 = signature.hash(world.entity(entity2));
        let hash3 = signature.hash(world.entity(entity3));
        let hash4 = signature.hash(world.entity(entity4));

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_ne!(hash3, hash4);
        assert_ne!(hash1, hash4);
    }

    #[test]
    fn different_initial_state() {
        let mut world = World::new();

        let entity1 = world.spawn((A, B)).id();
        let entity2 = world.spawn((A, B)).id();

        let signature = Signature::of::<(A, B)>();
        let signature_42 = Signature::of::<(A, B)>().with_base(42);

        let hash1 = signature.hash(world.entity(entity1));
        let hash2 = signature.hash(world.entity(entity2));
        let hash1_42 = signature_42.hash(world.entity(entity1));
        let hash2_42 = signature_42.hash(world.entity(entity2));

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash1_42);
        assert_eq!(hash1_42, hash2_42);
    }

    #[test]
    fn different_component_names() {
        let mut world = World::new();

        let entity = world.spawn((A, B)).id();

        let signature_a = Signature::of_single::<A>();
        let signature_b = Signature::of_single::<B>();

        let hash_a = signature_a.hash(world.entity(entity));
        let hash_b = signature_b.hash(world.entity(entity));

        assert_ne!(hash_a, hash_b);
    }

    #[derive(Component, Hash)]
    struct A;

    #[derive(Component, Hash)]
    struct B;

    #[derive(Component, Hash)]
    struct C(bool);
}
