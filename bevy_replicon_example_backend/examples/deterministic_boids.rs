//! Boids with deterministic replication: on connection, the server sends its state
//! to clients, which then continue simulating independently while achieving identical results
//! through determinism.
//! See https://vanhunteradams.com/Pico/Animal_Movement/Boids-algorithm.html for details on the
//! used algorithm.
//!
//! While deterministic lockstep is very efficient in terms of bandwidth, it's not always possible
//! to make your simulation deterministic. Floating point determinism across platforms is hard.
//! Also clients need to know the full state, which may not be acceptable if your game design
//! uses restricted visibility (e.g. fog of war).
//! Luckily, you can combine it with authoritative replication for things that can't be simulated
//! deterministically or need to be hidden from clients.

use std::{
    f32::consts::TAU,
    net::{IpAddr, Ipv4Addr},
};

use bevy::prelude::*;
use bevy_replicon::{
    bytes::Bytes,
    postcard_utils,
    prelude::*,
    shared::replication::registry::ctx::{SerializeCtx, WriteCtx},
};
use bevy_replicon_example_backend::{ExampleClient, ExampleServer, RepliconExampleBackendPlugins};
use clap::Parser;
use fastrand::Rng;
use fastrand_contrib::RngExt;
use serde::{Deserialize, Serialize};

fn main() {
    App::new()
        .init_resource::<Cli>()
        .add_plugins((
            DefaultPlugins,
            RepliconPlugins,
            RepliconExampleBackendPlugins,
        ))
        .replicate_once::<Boid>()
        .replicate_once::<Bias>()
        .replicate_once::<Velocity>()
        .replicate_with((
            // Customize serialization to skip `scale` and avoid sending Z axis.
            RuleFns::new(serialize_transform, deserialize_transform),
            ReplicationMode::Once,
        ))
        .add_observer(init_boid)
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, update)
        .run();
}

const MAX_SPEED: f32 = 6.0;
const MIN_SPEED: f32 = 3.0;

const MAX_BIAS: f32 = 0.01;
const BIAS_INCREMENT: f32 = 0.00004;

fn setup(mut commands: Commands, cli: Res<Cli>) -> Result<()> {
    commands.spawn(Camera2d);

    match *cli {
        Cli::Local => {
            info!("starting local");
            spawn_boids(&mut commands);
        }
        Cli::Server { port } => {
            info!("starting server at port {port}");

            // Backend initialization
            let server = ExampleServer::new(port)?;
            commands.insert_resource(server);

            commands.spawn((
                Text::new("Server"),
                TextFont {
                    font_size: 30.0,
                    ..Default::default()
                },
                TextColor::WHITE,
            ));
            spawn_boids(&mut commands);
        }
        Cli::Client { ip, port } => {
            info!("connecting to {ip}:{port}");

            // Backend initialization
            let client = ExampleClient::new((ip, port))?;
            let addr = client.local_addr()?;
            commands.insert_resource(client);

            commands.spawn((
                Text(format!("Client: {addr}")),
                TextFont {
                    font_size: 30.0,
                    ..default()
                },
                TextColor::WHITE,
            ));
        }
    }

    Ok(())
}

fn spawn_boids(commands: &mut Commands) {
    let mut rng = Rng::new();
    const BOIDS_COUNT: usize = 250;
    for _ in 0..BOIDS_COUNT {
        let color = Srgba::gray(rng.f32_range(0.4..=1.0)).with_alpha(0.8).into();
        let group = match rng.u8(0..=1) {
            0 => BoidGroup::LeftSide,
            1 => BoidGroup::RightSide,
            _ => unreachable!(),
        };

        let bias = rng.f32_range(BIAS_INCREMENT..=MAX_BIAS);

        let velocity_x = rng.f32_range(MIN_SPEED..=MAX_SPEED);
        let velocity_y = rng.f32_range(MIN_SPEED..=MAX_SPEED);

        let x = rng.f32_range(-0.200..=200.0);
        let y = rng.f32_range(-0.200..=200.0);
        let angle = rng.f32_range(0.0..TAU);

        commands.spawn((
            Boid { color, group },
            Bias(bias),
            Velocity(Vec2::new(velocity_x, velocity_y)),
            Transform {
                translation: Vec3::new(x, y, 0.0),
                rotation: Quat::from_rotation_z(angle),
                ..Default::default()
            },
        ));
    }
}

/// Initializes visuals for boids.
///
/// Materials and meshes are not replicated. Instead, they are created locally when a [`Boid`]
/// is inserted. This way, visuals are applied automatically both when a boid is spawned
/// manually on the server (or in singleplayer) and when it is replicated to a client.
fn init_boid(
    trigger: Trigger<OnInsert, Boid>,
    boid_mesh: Local<BoidMesh>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut boids: Query<(&Boid, &mut Mesh2d, &mut MeshMaterial2d<ColorMaterial>)>,
) {
    let (boid, mut mesh, mut material) = boids.get_mut(trigger.target()).unwrap();

    // All boids share the same mesh, but each gets a unique material from its color.
    **mesh = boid_mesh.0.clone();
    **material = materials.add(boid.color);
}

/// Simulates boid flocking behavior.
///
/// The simulation is deterministic: the algorithm relies only on +, -, *, and /,
/// which are defined by IEEE-754. For more details, see
/// https://github.com/bevyengine/bevy/discussions/8675 (scroll to "Math").
fn update(
    mut cached_boids: Local<Vec<(Vec2, Velocity)>>,
    mut boids: Query<(&Boid, &mut Bias, &mut Transform, &mut Velocity)>,
) {
    // Store all boids data to avoid issues with borrow checker,
    // since since each boid must be iterated against all others.
    cached_boids.clear();
    cached_boids.reserve(boids.iter().len());
    cached_boids.extend(
        boids
            .iter()
            .map(|(.., t, &v)| (t.translation.truncate(), v)),
    );

    const VISUAL_RANGE: f32 = 40.0;
    const PROTECTED_RANGE: f32 = 8.0;

    const CENTERING_FACTOR: f32 = 0.0005;
    const MATCHING_FACTOR: f32 = 0.05;
    const AVOID_FACTOR: f32 = 0.05;
    const TURN_FACTOR: f32 = 0.2;

    const HALF_AREA: Vec2 = Vec2::new(640.0, 360.0);
    const MARGIN: f32 = 100.0;

    for (index, (boid, mut bias, mut transform, mut velocity)) in boids.iter_mut().enumerate() {
        let position = transform.translation.truncate();

        let mut close_diff = Vec2::ZERO;
        let mut position_avg = Vec2::ZERO;
        let mut velocity_avg = Vec2::ZERO;
        let mut neighbors = 0;

        // Collect information about other boids.
        for (other_index, &(other_position, other_velocity)) in cached_boids.iter().enumerate() {
            if index == other_index {
                continue;
            }
            let diff = position - other_position;

            if diff.x.abs() < VISUAL_RANGE && diff.y.abs() < VISUAL_RANGE {
                let len_squared = diff.length_squared();
                if len_squared < PROTECTED_RANGE * PROTECTED_RANGE {
                    close_diff += diff;
                } else if len_squared < VISUAL_RANGE * VISUAL_RANGE {
                    position_avg += other_position;
                    velocity_avg += *other_velocity;
                    neighbors += 1;
                }
            }
        }

        // Apply separation, alignment and cohesion.
        if neighbors > 0 {
            position_avg /= neighbors as f32;
            velocity_avg /= neighbors as f32;

            let centering = (position_avg - position) * CENTERING_FACTOR;
            let matching = (velocity_avg - **velocity) * MATCHING_FACTOR;
            **velocity += centering + matching;
        }
        **velocity += close_diff * AVOID_FACTOR;

        // Avoid screen edges.
        if position.x <= -HALF_AREA.x + MARGIN {
            velocity.x += TURN_FACTOR;
        } else if position.x >= HALF_AREA.x - MARGIN {
            velocity.x -= TURN_FACTOR;
        }
        if position.y <= -HALF_AREA.y + MARGIN {
            velocity.y += TURN_FACTOR;
        } else if position.y >= HALF_AREA.y - MARGIN {
            velocity.y -= TURN_FACTOR;
        }

        // Apply bias.
        let sign = match boid.group {
            BoidGroup::LeftSide => -1.0,
            BoidGroup::RightSide => 1.0,
        };
        if sign * velocity.x > 0.0 {
            **bias = MAX_BIAS.min(**bias + BIAS_INCREMENT)
        } else {
            **bias = BIAS_INCREMENT.max(**bias - BIAS_INCREMENT)
        }
        velocity.x = (1.0 - **bias) * velocity.x + sign * **bias;

        // Limit speed.
        let speed = velocity.length();
        if speed < MIN_SPEED {
            **velocity = **velocity / speed * MIN_SPEED;
        } else if speed > MAX_SPEED {
            **velocity = **velocity / speed * MAX_SPEED;
        }

        // Apply computation results to the sprite.
        transform.translation.x += velocity.x;
        transform.translation.y += velocity.y;

        // `Vec2::to_angle` is not deterministic! However, since this is only used for
        // visualization and never read back, it does not affect simulation determinism.
        transform.rotation = Quat::from_rotation_z(velocity.to_angle());
    }
}

const DEFAULT_PORT: u16 = 5000;

/// Deterministic Boids demo.
#[derive(Parser, PartialEq, Resource)]
enum Cli {
    /// Run locally without any networking.
    Local,
    /// Create a server.
    Server {
        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Connect to a host.
    Client {
        #[arg(short, long, default_value_t = Ipv4Addr::LOCALHOST.into())]
        ip: IpAddr,

        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

impl Default for Cli {
    fn default() -> Self {
        Self::parse()
    }
}

#[derive(Component, Serialize, Deserialize)]
#[require(Replicated, Mesh2d, MeshMaterial2d<ColorMaterial>)]
#[component(immutable)]
struct Boid {
    color: Color,
    group: BoidGroup,
}

#[derive(Serialize, Deserialize)]
enum BoidGroup {
    LeftSide,
    RightSide,
}

#[derive(Component, Serialize, Deserialize, Deref, DerefMut)]
struct Bias(f32);

#[derive(Component, Deref, DerefMut, Serialize, Deserialize, Clone, Copy)]
struct Velocity(Vec2);

/// Mesh for all boids.
///
/// Used as a [`Local`] inside [`init_boid`] to re-use the same handle.
struct BoidMesh(Handle<Mesh>);

impl FromWorld for BoidMesh {
    fn from_world(world: &mut World) -> Self {
        let triangle = Triangle2d::new(
            Vec2::new(8.0, 0.0),
            Vec2::new(-5.0, 5.0),
            Vec2::new(-5.0, -5.0),
        );
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let mesh = meshes.add(triangle);

        Self(mesh)
    }
}

/// Serializes [`Transform`] without [`Transform::scale`] and Z axis.
fn serialize_transform(
    _ctx: &SerializeCtx,
    transform: &Transform,
    message: &mut Vec<u8>,
) -> Result<()> {
    postcard_utils::to_extend_mut(&transform.translation.truncate(), message)?;
    postcard_utils::to_extend_mut(&transform.rotation, message)?;
    Ok(())
}

/// Deserializes [`Transform`] without [`Transform::scale`] and Z axis.
fn deserialize_transform(_ctx: &mut WriteCtx, message: &mut Bytes) -> Result<Transform> {
    let translation: Vec2 = postcard_utils::from_buf(message)?;
    let rotation = postcard_utils::from_buf(message)?;
    Ok(Transform::from_translation(translation.extend(0.0)).with_rotation(rotation))
}
