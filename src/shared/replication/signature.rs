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
use log::{debug, error};

/// Describes how to calculate a deterministic hash that identifies an entity.
///
/// When the client receives replication, it maps server entities to its own entities
/// using [`ServerEntityMap`](crate::shared::server_entity_map::ServerEntityMap).
/// If there is no mapping, it spawns a new entity and creates a new mapping to it.
///
/// To re-use a previously spawned entity on the client, insert this component on both
/// server and client. On insertion it will calculate a hash and on replication client
/// will try to match. If the hash matches, the replication will continue to previously
/// spawned entity.
///
/// The hash can be calculated from components on the entity, a user-defined struct, or both.
/// The user needs to use something that is unique for each entity, but identical for the
/// same entity on both client and server.
///
/// Signatures can also be relevant only to a specific client. In this case, the signature
/// will be sent only to that client.
#[derive(Component, Debug, Clone, Copy)]
#[component(on_add = register_hash, on_remove = unregister_hash)]
pub struct Signature {
    /// User-defined initial state for the hash.
    base_hash: Option<u64>,

    /// Functions to calculate hash from components.
    fns: &'static [HashFn],

    /// Relevant client.
    client: Option<Entity>,

    /// Resulting hash.
    ///
    /// Calculated when added to an entity.
    hash: u64,
}

impl Signature {
    /**
    Creates a new instance that hashes the specified component and its type name.

    Pairs well with the required components but can also be inserted during a
    regular entity spawn.

    # Examples

    Chess board deterministically spawned on both client and server without sending
    the entire board data through the network:

    ```
    # use bevy::state::app::StatesPlugin;
    use bevy::{color::palettes::css::{BLACK, WHITE}, prelude::*};
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
    app.replicate::<Square>()
        .add_systems(Startup, spawn_chessboard);

    // Spawn chessboard as usual, no network-related code.
    fn spawn_chessboard(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<ColorMaterial>>,
    ) {
        const SQUARES_PER_SIDE: u8 = 8;
        const SQUARE_SIZE: f32 = 64.0;

        let square = meshes.add(Rectangle::new(SQUARE_SIZE, SQUARE_SIZE));
        let black = materials.add(Color::from(BLACK));
        let white = materials.add(Color::from(WHITE));

        let board_size = SQUARE_SIZE * SQUARES_PER_SIDE as f32;
        let origin = -board_size / 2.0 + SQUARE_SIZE / 2.0;

        for file in 0..SQUARES_PER_SIDE {
            for rank in 0..SQUARES_PER_SIDE {
                let x = origin + (file as f32) * SQUARE_SIZE;
                let y = origin + (rank as f32) * SQUARE_SIZE;

                let is_light = (file + rank) % 2 == 0;
                let material = if is_light {
                    white.clone()
                } else {
                    black.clone()
                };

                commands.spawn((
                    MeshMaterial2d(material.clone()),
                    Mesh2d(square.clone()),
                    Transform::from_xyz(x, y, 0.0),
                    Square { file, rank },
                ));
            }
        }
    }

    /// Square location on the chessboard.
    ///
    /// We want to replicate all squares, so we set [`Replicated`] as a required component.
    /// We also want entities with this component to be automatically mapped between
    /// client and server, so we require the [`Signature`] component, which generates a hash
    /// based on [`Square`]. Each entity will be spawned with a different value, making the hash
    /// unique. Since spawning on the server is identical, the server will generate the same hashes
    /// for the same squares, and the client will match them to the corresponding local squares.
    #[derive(Component, Serialize, Deserialize, Hash)]
    #[require(
        Replicated,
        Signature::of::<Square>(),
    )]
    struct Square {
        /// Column, a..h.
        file: u8,
        /// Row, 1..8.
        rank: u8,
    }
    ```
    */
    #[must_use]
    pub fn of<C: Component + Hash>() -> Self {
        Self {
            base_hash: None,
            fns: &[hash::<C>],
            client: None,
            hash: 0,
        }
    }

    /**
    Creates a new instance that hashes the specified set of components and their type names.

    # Examples

    Predicting a projectile on the client.

    ```
    # use bevy::state::app::StatesPlugin;
    use bevy::{input::common_conditions::*, prelude::*};
    use bevy_replicon::prelude::*;
    use serde::{Deserialize, Serialize};

    # let mut app = App::new();
    # app.add_plugins((StatesPlugin, RepliconPlugins));
    app.add_client_trigger::<SpawnFireball>(Channel::Ordered)
        .add_observer(confirm_projectile)
        .add_systems(
            FixedUpdate,
            shoot_projectile
                .run_if(input_just_pressed(MouseButton::Left))
                .run_if(in_state(ClientState::Connected)),
        );

    /// System that shoots a fireball and spawns it on the client.
    fn shoot_projectile(
        mut commands: Commands,
        instigator: Single<Entity, With<Player>>,
    ) {
        commands.spawn((
            Projectile {
                instigator: *instigator,
            },
            Fireball,
            Signature::of_n::<(Projectile, Fireball)>(),
        ));
        commands.trigger(SpawnFireball);
    }

    /// Validation to check if client is not cheating or the simulation is correct.
    ///
    /// Depending on the type of game you may want to correct the client or disconnect it.
    /// In this example we just always confirm the spawn.
    fn confirm_projectile(
        trigger: Trigger<FromClient<SpawnFireball>>,
        mut commands: Commands,
        clients: Query<&Controls>,
    ) {
        if let ClientId::Client(client) = trigger.client_id {
            let instigator = **clients.get(client).unwrap();

            // You can insert more components, they will be replicated to the client's entity.
            commands.spawn((
                Projectile {
                    instigator,
                },
                Fireball,
                Signature::of_n::<(Projectile, Fireball)>().for_client(client),
            ));
        }
    }

    /// Trigger to ask server to spawn a projectile.
    #[derive(Event, Serialize, Deserialize)]
    struct SpawnFireball;

    /// Marker for player entity.
    #[derive(Component)]
    struct Player;

    /// Holds the player entity controlled by the client.
    #[derive(Component, Deref)]
    struct Controls(Entity);

    #[derive(Component, Hash)]
    struct Projectile {
        instigator: Entity,
    }

    #[derive(Component, Hash)]
    struct Fireball;
    ```
    */
    #[must_use]
    pub fn of_n<S: SignatureComponents>() -> Self {
        Self {
            base_hash: None,
            fns: S::HASH_FNS,
            client: None,
            hash: 0,
        }
    }

    /// Sets the base hash by hashing the given value.
    ///
    /// The resulting hash will be used as the initial state before
    /// component data is hashed. This allows entities with
    /// identical components to be distinguished.
    ///
    /// You can also use [`Self::from`] to create a signature
    /// that doesn't hash any components.
    #[must_use]
    pub fn with_base<T: Hash>(mut self, value: T) -> Self {
        let mut hasher = FnvHasher::default();
        value.hash(&mut hasher);

        self.base_hash = Some(hasher.finish());
        self
    }

    /// Associates the signature with a specific client.
    ///
    /// Such a signature will be sent only to that client.
    #[must_use]
    pub fn for_client(mut self, client: Entity) -> Self {
        self.client = Some(client);
        self
    }

    pub(crate) fn client(&self) -> Option<Entity> {
        self.client
    }

    pub(crate) fn hash(&self) -> u64 {
        self.hash
    }

    fn eval<'a>(&self, entity: impl Into<EntityRef<'a>>) -> u64 {
        let mut hasher = self.base_hash.map(FnvHasher::with_key).unwrap_or_default();

        let entity = entity.into();
        for hash_fn in self.fns {
            (hash_fn)(&entity, &mut hasher);
        }

        hasher.finish()
    }
}

impl<T: Hash> From<T> for Signature {
    /// Creates a new instance with only the base hash,
    /// that won't additionally hash any components.
    ///
    /// It's usually better to use components because their names
    /// are also hashed.
    fn from(value: T) -> Self {
        let mut hasher = FnvHasher::default();
        value.hash(&mut hasher);

        Self {
            base_hash: Some(hasher.finish()),
            fns: &[],
            client: None,
            hash: 0,
        }
    }
}

fn register_hash(mut world: DeferredWorld, ctx: HookContext) {
    let mut entity = world.entity_mut(ctx.entity);
    let signature = entity.get::<Signature>().unwrap();
    let hash = signature.eval(&entity);

    // Re-borrow due to borrow-checker.
    let mut signature = entity.get_mut::<Signature>().unwrap();
    signature.hash = hash;

    world
        .resource_mut::<SignatureMap>()
        .insert(ctx.entity, hash);
}

fn unregister_hash(mut world: DeferredWorld, ctx: HookContext) {
    // The map will be unavailable during replication because the
    // resource will be temporarily removed from the world.
    if let Some(mut map) = world.get_resource_mut::<SignatureMap>() {
        map.remove(ctx.entity);
    }
}

/// Stores hashes calculated from the [`Signature`] component and maps them
/// to their entities in both directions.
///
/// Used to detect hash collisions and on the client it's used to map received
/// hashes from server to client's entities.
///
/// Automatically updated via hooks.
#[derive(Resource, Default)]
pub(crate) struct SignatureMap {
    to_hashes: EntityHashMap<u64>,
    to_entities: HashMap<u64, Entity, NoOpHash>, // Skip hashing because the key is already a hash.
}

impl SignatureMap {
    pub(crate) fn get(&self, hash: u64) -> Option<Entity> {
        self.to_entities.get(&hash).copied()
    }

    fn insert(&mut self, entity: Entity, hash: u64) {
        match self.to_entities.try_insert(hash, entity) {
            Ok(_) => {
                debug!("inserting hash 0x{hash:016x} for `{entity}`");
                self.to_hashes.insert(entity, hash);
            }
            Err(e) => error!(
                "hash 0x{hash:016x} for `{entity}` already corresponds to `{}` and will be ignored",
                e.value
            ),
        }
    }

    pub(crate) fn remove(&mut self, entity: Entity) {
        if let Some(hash) = self.to_hashes.remove(&entity) {
            debug!("removing hash 0x{hash:016x} for `{entity}`");
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

variadics_please::all_tuples!(impl_signature_components, 0, 6, C);

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

        let signature = Signature::of::<C>();
        let hash1 = signature.eval(world.entity(entity1));
        let hash2 = signature.eval(world.entity(entity2));
        let hash3 = signature.eval(world.entity(entity3));
        let hash4 = signature.eval(world.entity(entity4));
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

        let signature = Signature::of_n::<(A, C)>();
        let hash1 = signature.eval(world.entity(entity1));
        let hash2 = signature.eval(world.entity(entity2));
        let hash3 = signature.eval(world.entity(entity3));
        let hash4 = signature.eval(world.entity(entity4));

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

        let signature = Signature::of_n::<(A, B)>();
        let signature_42 = Signature::of_n::<(A, B)>().with_base(42);

        let hash1 = signature.eval(world.entity(entity1));
        let hash2 = signature.eval(world.entity(entity2));
        let hash1_42 = signature_42.eval(world.entity(entity1));
        let hash2_42 = signature_42.eval(world.entity(entity2));

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash1_42);
        assert_eq!(hash1_42, hash2_42);
    }

    #[test]
    fn different_component_names() {
        let mut world = World::new();

        let entity = world.spawn((A, B)).id();

        let signature_a = Signature::of::<A>();
        let signature_b = Signature::of::<B>();

        let hash_a = signature_a.eval(world.entity(entity));
        let hash_b = signature_b.eval(world.entity(entity));

        assert_ne!(hash_a, hash_b);
    }

    #[derive(Component, Hash)]
    struct A;

    #[derive(Component, Hash)]
    struct B;

    #[derive(Component, Hash)]
    struct C(bool);
}
