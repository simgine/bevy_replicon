//! An RTS demo with authoritative replication: the server executes all game logic,
//! and clients primarily render.
//!
//! This uses more bandwidth than deterministic replication, but it's
//! more secure because clients don't necessarily need the full state, and it's
//! easier to implement because it removes the determinism requirement from the
//! game logic.
//!
//! In this example, clients don't predict or rollback. They simply wait for
//! state updates from the server. It's a common strategy for RTS because the
//! input delay won't be noticeable.
//!
//! The tick rate is 64 Hz (default in Bevy). You can lower it, but if you want
//! to render the movement smoothly in listen-server and single-player
//! modes, you need to apply transform interpolation between fixed updates.
//! See the [interpolation and/or rollback](https://github.com/simgine/bevy_replicon?tab=readme-ov-file#interpolation-andor-rollback)
//! section for available solutions.

use std::{
    f32::consts::TAU,
    net::{IpAddr, Ipv4Addr},
};

use bevy::{
    color::palettes::tailwind::{
        BLUE_500, GREEN_500, GREEN_700, ORANGE_500, PINK_500, PURPLE_500, RED_500, TEAL_500,
        YELLOW_500,
    },
    ecs::entity::MapEntities,
    platform::collections::HashMap,
    prelude::*,
    render::primitives::Aabb,
};
use bevy_replicon::{
    bytes::Bytes,
    postcard_utils,
    prelude::*,
    shared::replication::registry::ctx::{SerializeCtx, WriteCtx},
};
use bevy_replicon_example_backend::{ExampleClient, ExampleServer, RepliconExampleBackendPlugins};
use clap::{Parser, ValueEnum};
use pathfinding::prelude::*;
use serde::{Deserialize, Serialize};

fn main() {
    App::new()
        .init_resource::<Cli>() // Parse CLI before creating window.
        .add_plugins((
            DefaultPlugins,
            RepliconPlugins,
            RepliconExampleBackendPlugins,
        ))
        .init_resource::<Selection>()
        .init_resource::<ClientTeams>()
        .replicate::<Unit>()
        .replicate::<Command>()
        // Customize serialization to skip `scale` and avoid sending Z axis.
        .replicate_with(RuleFns::new(serialize_transform, deserialize_transform))
        .add_client_trigger::<TeamRequest>(Channel::Unordered)
        .add_client_trigger::<UnitSpawn>(Channel::Unordered)
        .add_mapped_client_trigger::<UnitsMove>(Channel::Unordered)
        .add_observer(apply_team_request)
        .add_observer(trigger_unit_spawn)
        .add_observer(apply_unit_spawn)
        .add_observer(init_unit)
        .add_observer(select_units)
        .add_observer(end_selection)
        .add_observer(clear_selection)
        .add_observer(trigger_units_move)
        .add_observer(apply_units_move)
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, move_units.run_if(server_or_singleplayer))
        .add_systems(
            Update,
            (
                trigger_team_request.run_if(client_just_connected),
                draw_selection.run_if(|r: Res<Selection>| r.active),
                draw_selected,
            ),
        )
        .run();
}

fn setup(mut commands: Commands, cli: Res<Cli>) -> Result<()> {
    commands.spawn(Camera2d);
    commands.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            align_self: AlignSelf::FlexEnd,
            align_content: AlignContent::FlexEnd,
            ..Default::default()
        },
        children![
            Text::new("Left-click and drag to select"),
            Text::new("Middle-click to spawn unit"),
            Text::new("Right-click to move"),
        ],
    ));

    match *cli {
        Cli::Singleplayer { team } => {
            info!("starting singleplayer as `{team:?}`");
            commands.client_trigger(TeamRequest { team });
            commands.insert_resource(team);
        }
        Cli::Server { port, team } => {
            info!("starting server as `{team:?}` at port {port}");

            // Backend initialization
            let server = ExampleServer::new(port)?;
            commands.insert_resource(server);

            // The server can also trigger client events.
            // These are re-emitted as `FromClient` with `ClientId::Server`.
            // This allows the code to work simultaneously in both client and listen-server modes.
            commands.client_trigger(TeamRequest { team });
            commands.insert_resource(team);
            commands.spawn(Text::new("Server"));
        }
        Cli::Client { port, ip, team } => {
            info!("connecting to {ip}:{port}");

            // Backend initialization
            let client = ExampleClient::new((ip, port))?;
            let addr = client.local_addr()?;
            commands.insert_resource(client);

            commands.insert_resource(team);
            commands.spawn(Text(format!("Client: {addr}")));
        }
    }

    Ok(())
}

/// Sends the client's team choice to the server.
fn trigger_team_request(mut commands: Commands, team: Res<Team>) {
    commands.client_trigger(TeamRequest { team: *team });
}

/// Assigns a team to a player.
fn apply_team_request(
    trigger: Trigger<FromClient<TeamRequest>>,
    mut events: EventWriter<DisconnectRequest>,
    mut teams: ResMut<ClientTeams>,
) {
    if let Some((client_id, team)) = teams.iter().find(|&(_, team)| *team == trigger.team) {
        error!(
            "`{}` requested team `{team:?}`, but it's already taken by `{client_id}`",
            trigger.client_id,
        );
        let client = trigger
            .client_id
            .entity()
            .expect("server can't request an invalid team");
        events.write(DisconnectRequest { client });
        return;
    }

    info!(
        "associating `{}` with team `{:?}`",
        trigger.client_id, trigger.team
    );

    teams.insert(trigger.client_id, trigger.team);
}

/// Requests spawning a unit at the click location.
///
/// We could network the [`Pointer`] event directly, but it carries a lot of
/// extra data and we only care about middle clicks. Instead, we created a
/// custom event and trigger it on clicks.
///
/// This also makes the logic independent of camera position - even though in
/// this demo the camera cannot move.
fn trigger_unit_spawn(
    trigger: Trigger<Pointer<Pressed>>,
    mut commands: Commands,
    camera: Single<(&Camera, &GlobalTransform)>,
) -> Result<()> {
    if trigger.button != PointerButton::Middle {
        return Ok(());
    }

    let (camera, transform) = *camera;
    let position = camera.viewport_to_world_2d(transform, trigger.pointer_location.position)?;

    commands.client_trigger(UnitSpawn { position });

    Ok(())
}

/// Applies a spawn request.
///
/// Executed on server and singleplayer.
/// The unit will be replicated back to clients.
fn apply_unit_spawn(
    trigger: Trigger<FromClient<UnitSpawn>>,
    mut commands: Commands,
    teams: Res<ClientTeams>,
) {
    let Some(&team) = teams.get(&trigger.client_id) else {
        error!(
            "`{}` attempted to spawn a unit but has no team",
            trigger.client_id
        );
        return;
    };

    commands.spawn((
        Unit { team },
        Transform::from_translation(trigger.position.extend(0.0)),
    ));
}

/// Initializes visuals for the unit.
///
/// We don't replicate materials and meshes, since they aren't dynamically
/// generated and are already known to clients. Instead, we replicate only
/// the minimal information needed, such as the team, to pick the correct
/// material. If units had different meshes, we would replicate the unit
/// type and initialize the corresponding mesh on the client.
///
/// Works for initialization on both the server (when a unit
/// is spawned) and the client (when the unit is replicated).
fn init_unit(
    trigger: Trigger<OnInsert, Unit>,
    unit_mesh: Local<UnitMesh>,
    unit_materials: Local<UnitMaterials>,
    mut units: Query<(&Unit, &mut Mesh2d, &mut MeshMaterial2d<ColorMaterial>)>,
) {
    let (unit, mut mesh, mut material) = units.get_mut(trigger.target()).unwrap();
    **mesh = unit_mesh.0.clone();
    **material = unit_materials.get(&unit.team).unwrap().clone();
}

/// Selects the player's units within a selection rectangle.
///
/// The selection is local to the player and is not networked.
fn select_units(
    trigger: Trigger<Pointer<Drag>>,
    mut commands: Commands,
    mut selection: ResMut<Selection>,
    team: Res<Team>,
    camera: Single<(&Camera, &GlobalTransform)>,
    units: Query<(Entity, &Unit, &GlobalTransform, &Aabb, Has<Selected>)>,
) -> Result<()> {
    if trigger.button != PointerButton::Primary {
        return Ok(());
    }

    let (camera, transform) = *camera;

    let origin = camera.viewport_to_world_2d(
        transform,
        trigger.pointer_location.position - trigger.distance,
    )?;
    let end = camera.viewport_to_world_2d(transform, trigger.pointer_location.position)?;

    selection.rect = Rect::from_corners(origin, end);
    selection.active = true;

    for (unit_entity, unit, transform, aabb, prev_selected) in &units {
        let center = transform.translation_vec3a() + aabb.center;
        let rect = Rect::from_center_half_size(center.truncate(), aabb.half_extents.truncate());
        let selected = !selection.rect.intersect(rect).is_empty();
        if selected != prev_selected {
            if selected && unit.team == *team {
                commands.entity(unit_entity).insert(Selected);
            } else {
                commands.entity(unit_entity).remove::<Selected>();
            }
        }
    }

    Ok(())
}

/// Stops displaying the selection rectangle.
fn end_selection(_trigger: Trigger<Pointer<DragEnd>>, mut rect: ResMut<Selection>) {
    rect.active = false;
}

fn clear_selection(
    trigger: Trigger<Pointer<Pressed>>,
    mut commands: Commands,
    units: Query<Entity, With<Selected>>,
) {
    if trigger.button != PointerButton::Primary {
        return;
    }
    for unit in &units {
        commands.entity(unit).remove::<Selected>();
    }
}

/// Requests movement into a location for previously the selected units.
fn trigger_units_move(
    trigger: Trigger<Pointer<Pressed>>,
    mut commands: Commands,
    camera: Single<(&Camera, &GlobalTransform)>,
    units: Populated<Entity, With<Selected>>,
) -> Result<()> {
    if trigger.button != PointerButton::Secondary {
        return Ok(());
    }

    let (camera, transform) = *camera;
    let position = camera.viewport_to_world_2d(transform, trigger.pointer_location.position)?;

    commands.client_trigger(UnitsMove {
        units: units.iter().collect(),
        position,
    });

    Ok(())
}

const MOVE_SPACING: f32 = 30.0;

/// Applies a move request to the specified units.
///
/// Each unit receives a unique `Command::Move`, arranged in a grid formation
/// centered on the requested position. The grid is oriented toward that position.
fn apply_units_move(
    trigger: Trigger<FromClient<UnitsMove>>,
    teams: Res<ClientTeams>,
    mut slots: Local<Vec<Vec2>>,
    mut positions: Local<Vec<Vec2>>,
    mut units: Query<(&Unit, &GlobalTransform, &mut Command)>,
) {
    // Validate the received data since the client could be malicious.
    // For example, on the client side we skip empty selections, but a modified
    // client could bypass this and cause a division by zero on the server.
    if trigger.units.is_empty() {
        error!("`{}` attempted to move zero units", trigger.client_id);
        return;
    }

    let Some(&client_team) = teams.get(&trigger.client_id) else {
        error!(
            "`{}` attempted to move units but has no team",
            trigger.client_id
        );
        return;
    };

    positions.clear();
    positions.reserve(trigger.units.len());
    for (unit, transform, _) in units.iter_many(&trigger.units) {
        if unit.team != client_team {
            error!(
                "`{}` has team `{client_team:?}`, but tried to move unit with team `{:?}`",
                trigger.client_id, unit.team
            );
            return;
        }

        positions.push(transform.translation().truncate());
    }

    let units_count = trigger.units.len();
    let cols = (units_count as f32).sqrt().ceil() as usize;
    let rows = units_count.div_ceil(cols);
    let centering_offset = -Vec2::new(cols as f32 - 1.0, rows as f32 - 1.0) / 2.0 * MOVE_SPACING;

    // Orientation basis to make grid facing from group centroid toward the click.
    let positions_sum = positions.iter().sum::<Vec2>();
    let centroid = positions_sum / units_count as f32;
    let forward = (trigger.position - centroid).normalize_or(Vec2::Y);
    let right = Vec2::new(forward.y, -forward.x);
    let rotation = Mat2::from_cols(right, forward);

    slots.clear();
    slots.reserve(units_count);
    for index in 0..units_count {
        let row = index / cols;
        let col = index % cols;

        let grid_position = centering_offset + Vec2::new(col as f32, row as f32) * MOVE_SPACING;
        slots.push(trigger.position + rotation * grid_position);
    }

    // Pick closest slot for each unit using
    // Hungarian with squared distance as cost.
    let weights: Matrix<_> = positions
        .iter()
        .map(|p| {
            slots
                .iter()
                .map(|s| p.distance_squared(*s) as i64)
                .collect::<Vec<_>>()
        })
        .collect();
    let (_, unit_to_slot) = kuhn_munkres_min(&weights);

    let mut iter = units.iter_many_mut(&trigger.units);
    for &slot_index in &unit_to_slot {
        let (.., mut command) = iter.fetch_next().unwrap();
        *command = Command::Move(slots[slot_index]);
    }
}

/// Moves and spaces units.
///
/// Steers moving units toward their targets. Applies the separation
/// force from the [Boids](https://en.wikipedia.org/wiki/Boids)
/// model to reduce crowding.
///
/// Also resolves overlaps by applying simple circle–collision physics.
fn move_units(
    mut cached_units: Local<Vec<(Entity, Vec2, bool)>>,
    time: Res<Time>,
    mut units: Query<(Entity, &mut Transform, &mut Command), With<Unit>>,
) {
    const MAX_SPEED: f32 = 240.0;
    const ROT_RATE: f32 = 8.0;
    const SLOWDOWN_RADIUS: f32 = 40.0;
    const STOP_DIST: f32 = 6.0; // Consider slot reached.
    const SEP_STRENGTH: f32 = 600.0;
    const DAMPING: f32 = 0.97; // Tame tiny jitters.
    const IDLE_SPACING: f32 = 18.0;
    const MOVE_MASS: f32 = 1.0;
    const IDLE_MASS: f32 = 3.0;

    cached_units.clear();
    cached_units.reserve(units.iter().len());
    cached_units.extend(
        units
            .iter()
            .map(|(e, t, c)| (e, t.translation.truncate(), matches!(c, Command::Move(_)))),
    );

    // Steering with separation.
    for (entity, mut transform, mut command) in &mut units {
        let Command::Move(target) = *command else {
            continue;
        };

        let mut position = transform.translation.truncate();

        let to_target = target - position;
        let dist = to_target.length();
        if dist <= STOP_DIST {
            *command = Command::None;
            continue;
        }

        // Slowdown near the target to make it look coller,
        let speed_factor = (dist / SLOWDOWN_RADIUS).clamp(0.15, 1.0);
        let move_dir = to_target / dist;
        let mut velocity = move_dir * MAX_SPEED * speed_factor;

        let mut separation = Vec2::ZERO;
        for (other_entity, other_pos, other_moving) in &*cached_units {
            if *other_entity == entity {
                continue;
            }

            // Allow a tighter squeeze when passing through non-moving units.
            let min_dist = if *other_moving {
                MOVE_SPACING
            } else {
                IDLE_SPACING
            };

            let distance = position.distance(*other_pos);
            if distance > 0.0 && distance < min_dist {
                let away_dir = (position - *other_pos) / distance;
                let overlap = min_dist - distance;
                let overlap_ratio = (overlap / min_dist).clamp(0.0, 1.0);
                separation += away_dir * overlap_ratio * SEP_STRENGTH;
            }
        }

        let delta = time.delta_secs();

        velocity += separation * delta;
        velocity *= DAMPING;

        // Limit speed.
        let speed = velocity.length();
        if speed > MAX_SPEED {
            velocity = velocity / speed * MAX_SPEED;
        }

        position += velocity * delta;
        let rotation = Quat::from_rotation_z(velocity.to_angle());

        // Apply computation results to the sprite.
        transform.translation.x = position.x;
        transform.translation.y = position.y;
        transform.rotation.smooth_nudge(&rotation, ROT_RATE, delta);
    }

    // Enforce minimum spacing between units.
    let mut combos = units.iter_combinations_mut();
    while let Some([(_, mut a_transform, a_cmd), (_, mut b_transform, b_cmd)]) = combos.fetch_next()
    {
        let mut a = a_transform.translation.truncate();
        let mut b = b_transform.translation.truncate();

        let offset = b - a;
        let distance = offset.length();

        let a_moving = matches!(*a_cmd, Command::Move(_));
        let b_moving = matches!(*b_cmd, Command::Move(_));

        // Allow tighter squeeze only when exactly one side is idle.
        let min_dist = if a_moving ^ b_moving {
            IDLE_SPACING
        } else {
            MOVE_SPACING
        };

        if distance < min_dist {
            let push_dir = if distance != 0.0 {
                offset / distance
            } else {
                Vec2::from_angle(fastrand_contrib::f32_range(0.0..TAU))
            };
            let overlap = min_dist - distance;

            // Push non-moving units less.
            let a_mass = if a_moving { MOVE_MASS } else { IDLE_MASS };
            let b_mass = if b_moving { MOVE_MASS } else { IDLE_MASS };
            let mass_sum = a_mass + b_mass;
            let a_correction = b_mass / mass_sum;
            let b_correction = a_mass / mass_sum;

            a -= push_dir * (overlap * a_correction);
            b += push_dir * (overlap * b_correction);

            a_transform.translation.x = a.x;
            a_transform.translation.y = a.y;
            b_transform.translation.x = b.x;
            b_transform.translation.y = b.y;
        }
    }
}

const SELECTION_COLOR: Srgba = GREEN_700;

/// Draws a mouse selection rectangle.
fn draw_selection(mut gizmos: Gizmos, selection: Res<Selection>) {
    gizmos.rect_2d(
        selection.rect.center(),
        selection.rect.size(),
        SELECTION_COLOR,
    );
}

/// Draws circles around selected units and a cross at their move target positions.
fn draw_selected(mut gizmos: Gizmos, units: Query<(&GlobalTransform, &Command), With<Selected>>) {
    for (transform, &command) in &units {
        let position = transform.translation().truncate();
        gizmos.circle_2d(position, 15.0, SELECTION_COLOR);

        if let Command::Move(translation) = command {
            let isometry = Isometry2d {
                rotation: Rot2::FRAC_PI_4,
                translation,
            };
            gizmos.cross_2d(isometry, 10.0, SELECTION_COLOR);
        }
    }
}

const DEFAULT_PORT: u16 = 5000;

/// An RTS demo.
#[derive(Parser, PartialEq, Resource)]
enum Cli {
    /// Play locally.
    Singleplayer {
        #[arg(short, long)]
        team: Team,
    },
    /// Create a server that acts as both player and host.
    Server {
        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,

        #[arg(short, long)]
        team: Team,
    },
    /// Connect to a host.
    Client {
        #[arg(short, long, default_value_t = Ipv4Addr::LOCALHOST.into())]
        ip: IpAddr,

        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,

        #[arg(short, long)]
        team: Team,
    },
}

impl Default for Cli {
    fn default() -> Self {
        Self::parse()
    }
}

/// Mouse rectangle selection.
#[derive(Resource, Default)]
struct Selection {
    rect: Rect,
    active: bool,
}

/// Mesh for all units.
///
/// Used as a [`Local`] inside [`init_unit`] to re-use the same handle.
struct UnitMesh(Handle<Mesh>);

impl FromWorld for UnitMesh {
    fn from_world(world: &mut World) -> Self {
        let triangle = Triangle2d::new(
            Vec2::new(10.0, 0.0),
            Vec2::new(-6.0, 6.0),
            Vec2::new(-6.0, -6.0),
        );
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let mesh = meshes.add(triangle);

        Self(mesh)
    }
}

/// Unit materials for each team color.
///
/// Used as a [`Local`] inside [`init_boid`] to re-use the same handle for each team.
#[derive(Deref)]
struct UnitMaterials(HashMap<Team, Handle<ColorMaterial>>);

impl FromWorld for UnitMaterials {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<ColorMaterial>>();

        let mut map = HashMap::default();
        for team in [
            Team::Blue,
            Team::Red,
            Team::Teal,
            Team::Purple,
            Team::Yellow,
            Team::Orange,
            Team::Green,
            Team::Pink,
        ] {
            let color = materials.add(team.color());
            map.insert(team, color);
        }

        Self(map)
    }
}

/// A trigger to join a specific team for a client.
#[derive(Event, Serialize, Deserialize)]
struct TeamRequest {
    team: Team,
}

/// A trigger that spawns a unit at a location.
#[derive(Event, Serialize, Deserialize)]
struct UnitSpawn {
    position: Vec2,
}

/// A trigger that orders units to move to a specified location.
#[derive(Event, Serialize, Deserialize, MapEntities, Clone)]
struct UnitsMove {
    #[entities]
    units: Vec<Entity>,
    position: Vec2,
}

#[derive(Component, Serialize, Deserialize)]
#[component(immutable)]
#[require(Replicated, Command, Mesh2d, MeshMaterial2d<ColorMaterial>)]
struct Unit {
    team: Team,
}

/// Team color.
///
/// This palette matches the team colors from Warcraft III.
///
/// When used as a resource, it represents the local player's team.
#[derive(
    Resource,
    Serialize,
    Deserialize,
    Debug,
    ValueEnum,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Clone,
    Copy,
)]
enum Team {
    Blue,
    Red,
    Teal,
    Purple,
    Yellow,
    Orange,
    Green,
    Pink,
}

impl Team {
    fn color(self) -> Color {
        let color = match self {
            Team::Blue => BLUE_500,
            Team::Red => RED_500,
            Team::Teal => TEAL_500,
            Team::Purple => PURPLE_500,
            Team::Yellow => YELLOW_500,
            Team::Orange => ORANGE_500,
            Team::Green => GREEN_500,
            Team::Pink => PINK_500,
        };

        color.into()
    }
}

/// Associated team for each player.
#[derive(Resource, Default, Deref, DerefMut)]
struct ClientTeams(HashMap<ClientId, Team>);

/// Current command for a [`Unit`].
#[derive(Component, Serialize, Deserialize, Default, Clone, Copy)]
enum Command {
    #[default]
    None,
    Move(Vec2),
}

/// Marker for selected units.
#[derive(Component)]
#[require(Gizmo)]
struct Selected;

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
