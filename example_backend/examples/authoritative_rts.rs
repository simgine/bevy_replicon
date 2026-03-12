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

use std::{
    f32::consts::TAU,
    iter,
    net::{IpAddr, Ipv4Addr},
};

use bevy::{
    camera::primitives::Aabb,
    color::palettes::tailwind::{
        BLUE_500, GREEN_500, LIME_500, ORANGE_500, PINK_500, PURPLE_500, RED_500, TEAL_500,
        YELLOW_500,
    },
    ecs::entity::MapEntities,
    platform::collections::HashMap,
    prelude::*,
};
use bevy_replicon::prelude::*;
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
        .replicate::<Unit>()
        .replicate::<Team>()
        .replicate::<Command>()
        .replicate_as::<Transform, Transform2DWithoutScale>()
        .add_visibility_filter::<Team>()
        .add_client_event::<TeamRequest>(Channel::Unordered)
        .add_client_event::<UnitSpawn>(Channel::Unordered)
        .add_mapped_client_event::<MoveUnits>(Channel::Unordered)
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
        .add_systems(OnEnter(ClientState::Connected), trigger_team_request)
        .add_systems(
            FixedUpdate,
            move_units.run_if(in_state(ClientState::Disconnected)),
        )
        .add_systems(
            Update,
            (
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
            commands.insert_resource(LocalTeam(team));
        }
        Cli::Server { port, team } => {
            info!("starting server as `{team:?}` at port {port}");

            // Backend initialization
            let server = ExampleServer::new(port)?;
            commands.insert_resource(server);

            commands.insert_resource(LocalTeam(team));
            commands.spawn(Text::new("Server"));
        }
        Cli::Client { port, ip, team } => {
            info!("connecting to {ip}:{port}");

            // Backend initialization
            let client = ExampleClient::new((ip, port))?;
            let addr = client.local_addr()?;
            commands.insert_resource(client);

            commands.insert_resource(LocalTeam(team));
            commands.spawn(Text(format!("Client: {addr}")));
        }
    }

    Ok(())
}

/// Sends the client's team choice to the server.
fn trigger_team_request(mut commands: Commands, local_team: Res<LocalTeam>) {
    commands.client_trigger(TeamRequest { team: **local_team });
}

/// Assigns a team to a player.
fn apply_team_request(
    team_request: On<FromClient<TeamRequest>>,
    mut commands: Commands,
    mut disconnects: MessageWriter<DisconnectRequest>,
    server_team: Res<LocalTeam>,
    teams: Query<(Entity, &Team)>,
) {
    let client = team_request
        .client_id
        .entity()
        .expect("server never requests a team");

    if let Some((client_id, team)) = teams
        .iter()
        .map(|(client, team)| (ClientId::Client(client), *team))
        .chain(iter::once((ClientId::Server, **server_team)))
        .find(|&(_, team)| team == team_request.team)
    {
        error!(
            "`{}` requested team `{team:?}`, but it's already taken by `{client_id}`",
            team_request.client_id,
        );
        disconnects.write(DisconnectRequest { client });
        return;
    }

    info!("associating `{client}` with team `{:?}`", team_request.team);
    commands.entity(client).insert((Player, team_request.team));
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
    press: On<Pointer<Press>>,
    mut commands: Commands,
    camera: Single<(&Camera, &GlobalTransform)>,
) -> Result<()> {
    if press.button != PointerButton::Middle {
        return Ok(());
    }

    let (camera, transform) = *camera;
    let position = camera.viewport_to_world_2d(transform, press.pointer_location.position)?;

    commands.client_trigger(UnitSpawn { position });

    Ok(())
}

/// Applies a spawn request.
///
/// Executed on server and singleplayer.
/// The unit will be replicated back to clients.
fn apply_unit_spawn(
    spawn: On<FromClient<UnitSpawn>>,
    mut commands: Commands,
    server_team: Res<LocalTeam>,
    teams: Query<&Team>,
) {
    let Some(team) = select_team(spawn.client_id, **server_team, teams) else {
        error!(
            "`{}` attempted to spawn a unit but has no team",
            spawn.client_id
        );
        return;
    };

    commands.spawn((
        Unit,
        team,
        Transform::from_translation(spawn.position.extend(0.0)),
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
    insert: On<Insert, Unit>,
    unit_mesh: Local<UnitMesh>,
    unit_materials: Local<UnitMaterials>,
    mut units: Query<(&Team, &mut Mesh2d, &mut MeshMaterial2d<ColorMaterial>)>,
) {
    let (team, mut mesh, mut material) = units.get_mut(insert.entity).unwrap();
    **mesh = unit_mesh.0.clone();
    **material = unit_materials.get(team).unwrap().clone();
}

/// Selects the player's units within a selection rectangle.
///
/// The selection is local to the player and is not networked.
fn select_units(
    drag: On<Pointer<Drag>>,
    mut commands: Commands,
    mut selection: ResMut<Selection>,
    local_team: Res<LocalTeam>,
    camera: Single<(&Camera, &GlobalTransform)>,
    units: Query<(Entity, &Team, &GlobalTransform, &Aabb, Has<Selected>)>,
) -> Result<()> {
    if drag.button != PointerButton::Primary {
        return Ok(());
    }

    let (camera, transform) = *camera;

    let origin =
        camera.viewport_to_world_2d(transform, drag.pointer_location.position - drag.distance)?;
    let end = camera.viewport_to_world_2d(transform, drag.pointer_location.position)?;

    selection.rect = Rect::from_corners(origin, end);
    selection.active = true;

    for (entity, &team, transform, aabb, prev_selected) in &units {
        let center = transform.translation_vec3a() + aabb.center;
        let rect = Rect::from_center_half_size(center.truncate(), aabb.half_extents.truncate());
        let selected = !selection.rect.intersect(rect).is_empty();
        if selected != prev_selected {
            if selected && team == **local_team {
                commands.entity(entity).insert(Selected);
            } else {
                commands.entity(entity).remove::<Selected>();
            }
        }
    }

    Ok(())
}

/// Stops displaying the selection rectangle.
fn end_selection(_on: On<Pointer<DragEnd>>, mut rect: ResMut<Selection>) {
    rect.active = false;
}

fn clear_selection(
    press: On<Pointer<Press>>,
    mut commands: Commands,
    units: Query<Entity, With<Selected>>,
) {
    if press.button != PointerButton::Primary {
        return;
    }
    for unit in &units {
        commands.entity(unit).remove::<Selected>();
    }
}

/// Requests movement into a location for previously the selected units.
fn trigger_units_move(
    press: On<Pointer<Press>>,
    mut commands: Commands,
    camera: Single<(&Camera, &GlobalTransform)>,
    units: Populated<Entity, With<Selected>>,
) -> Result<()> {
    if press.button != PointerButton::Secondary {
        return Ok(());
    }

    let (camera, transform) = *camera;
    let position = camera.viewport_to_world_2d(transform, press.pointer_location.position)?;

    commands.client_trigger(MoveUnits {
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
    move_units: On<FromClient<MoveUnits>>,
    mut slots: Local<Vec<Vec2>>,
    mut positions: Local<Vec<Vec2>>,
    server_team: Res<LocalTeam>,
    teams: Query<&Team>,
    mut units: Query<(&Team, &GlobalTransform, &mut Command)>,
) {
    // Validate the received data since the client could be malicious.
    // For example, on the client side we skip empty selections, but a modified
    // client could bypass this and cause a division by zero on the server.
    if move_units.units.is_empty() {
        error!("`{}` attempted to move zero units", move_units.client_id);
        return;
    }

    let Some(team) = select_team(move_units.client_id, **server_team, teams) else {
        error!(
            "`{}` attempted to move units but has no team",
            move_units.client_id
        );
        return;
    };

    positions.clear();
    positions.reserve(move_units.units.len());
    for (&unit_team, transform, _) in units.iter_many(&move_units.units) {
        if unit_team != team {
            error!(
                "`{}` has team `{team:?}`, but tried to move unit with team `{unit_team:?}`",
                move_units.client_id,
            );
            return;
        }

        positions.push(transform.translation().truncate());
    }

    let units_count = move_units.units.len();
    let cols = (units_count as f32).sqrt().ceil() as usize;
    let rows = units_count.div_ceil(cols);
    let centering_offset = -Vec2::new(cols as f32 - 1.0, rows as f32 - 1.0) / 2.0 * MOVE_SPACING;

    // Orientation basis to make grid facing from group centroid toward the click.
    let positions_sum = positions.iter().sum::<Vec2>();
    let centroid = positions_sum / units_count as f32;
    let forward = (move_units.position - centroid).normalize_or(Vec2::Y);
    let right = Vec2::new(forward.y, -forward.x);
    let rotation = Mat2::from_cols(right, forward);

    slots.clear();
    slots.reserve(units_count);
    for index in 0..units_count {
        let row = index / cols;
        let col = index % cols;

        let grid_position = centering_offset + Vec2::new(col as f32, row as f32) * MOVE_SPACING;
        slots.push(move_units.position + rotation * grid_position);
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

    let mut iter = units.iter_many_mut(&move_units.units);
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

const SELECTION_COLOR: Srgba = LIME_500;

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

/// Picks a team by ID.
///
/// If it's [`ClientId::Server`], uses the value from resource.
/// Should be called only on server.
fn select_team(client_id: ClientId, server_team: Team, teams: Query<&Team>) -> Option<Team> {
    if let ClientId::Client(client) = client_id {
        teams.get(client).copied().ok()
    } else {
        Some(server_team)
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

/// Request to join a team.
#[derive(Event, Serialize, Deserialize)]
struct TeamRequest {
    team: Team,
}

/// Request to spawn a unit at a location.
#[derive(Event, Serialize, Deserialize)]
struct UnitSpawn {
    position: Vec2,
}

/// Orders units to move to a specified location.
#[derive(Event, Serialize, Deserialize, MapEntities, Clone)]
struct MoveUnits {
    #[entities]
    units: Vec<Entity>,
    position: Vec2,
}

#[derive(Component, Serialize, Deserialize)]
#[component(immutable)]
#[require(Replicated, Command, Mesh2d, MeshMaterial2d<ColorMaterial>)]
struct Unit;

#[derive(Component)]
struct Player;

#[derive(Resource, Deref, DerefMut, Clone, Copy)]
struct LocalTeam(Team);

/// Team color.
///
/// This palette matches the team colors from Warcraft III.
///
/// When used as a resource, it represents the local player's team.
/// Also present on player entities and units as component to associate
/// them with teams.
#[derive(
    Component,
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
#[component(immutable)]
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

/// Replicate commands only for units owned by the player.
impl VisibilityFilter for Team {
    type ClientComponent = Self;
    type Scope = SingleComponent<Command>;

    fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
        component.is_some_and(|c| self == c)
    }
}

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

/// Helper to send [`Transform`] without [`Transform::scale`] and Z axis.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct Transform2DWithoutScale {
    translation: Vec2,
    rotation: Quat,
}

impl From<Transform> for Transform2DWithoutScale {
    fn from(value: Transform) -> Self {
        Self {
            translation: value.translation.truncate(),
            rotation: value.rotation,
        }
    }
}

impl From<Transform2DWithoutScale> for Transform {
    fn from(value: Transform2DWithoutScale) -> Self {
        Self {
            translation: value.translation.extend(0.0),
            rotation: value.rotation,
            ..Default::default()
        }
    }
}
