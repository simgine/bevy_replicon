use std::net::{IpAddr, Ipv4Addr};

use bevy::{
    color::palettes::tailwind::{
        BLUE_500, GREEN_500, GREEN_700, ORANGE_500, PINK_500, PURPLE_500, RED_500, TEAL_500,
        YELLOW_500,
    },
    platform::collections::HashMap,
    prelude::*,
    render::primitives::Aabb,
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
        .replicate::<Player>()
        .replicate::<Unit>()
        .add_server_trigger::<MakeLocal>(Channel::Unordered)
        .add_client_trigger::<TeamRequest>(Channel::Unordered)
        .add_observer(init_client)
        .add_observer(make_local)
        .add_observer(init_unit)
        .add_observer(select_units)
        .add_observer(end_selection)
        .add_observer(clear_selection)
        .add_observer(move_command)
        .add_observer(spawn_units)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                draw_selection.run_if(|r: Res<Selection>| r.active),
                draw_selected,
            ),
        )
        .add_systems(FixedUpdate, move_units)
        .run();
}

fn setup(mut commands: Commands, cli: Res<Cli>) -> Result<()> {
    commands.spawn(Camera2d);

    match *cli {
        Cli::Singleplayer { team } => {
            info!("starting singleplayer as `{team:?}`");
        }
        Cli::Server { port, team } => {
            info!("starting server as `{team:?}` at port {port}");

            // Backend initialization
            let server = ExampleServer::new(port)?;
            commands.insert_resource(server);

            commands.spawn(Text::new("Server"));
        }
        Cli::Client { port, ip, team } => {
            info!("connecting to {ip}:{port}");

            // Backend initialization
            let client = ExampleClient::new((ip, port))?;
            let addr = client.local_addr()?;
            commands.insert_resource(client);

            commands.spawn(Text(format!("Client: {addr}")));
        }
    }

    Ok(())
}

fn init_client(
    trigger: Trigger<FromClient<TeamRequest>>,
    players: Query<&Player>,
    mut commands: Commands,
) {
    let client_entity = trigger.client_id.entity().unwrap();
    if players.iter().any(|p| p.team == trigger.team) {
        error!(
            "client `{client_entity}` requested team `{:?}`, but it's already taken",
            trigger.team
        );
        return;
    }

    info!(
        "associating client `{client_entity}` with team `{:?}`",
        trigger.team
    );

    commands
        .entity(client_entity)
        .insert(Player { team: trigger.team });

    commands.client_trigger_targets(MakeLocal, client_entity);
}

fn make_local(trigger: Trigger<MakeLocal>, mut commands: Commands) {
    commands.entity(trigger.target()).insert(LocalPlayer);
}

const UNIT_SPACING: f32 = 30.0;

fn spawn_units(
    trigger: Trigger<OnAdd, Player>,
    mut commands: Commands,
    client: Option<Res<RepliconClient>>,
    players: Query<&Player>,
) {
    if !server_or_singleplayer(client) {
        return;
    }

    const UNITS_COUNT: usize = 50;
    const COLS: usize = 10;
    const ROWS: usize = UNITS_COUNT / COLS;

    let grid_offset = -Vec2::new(COLS as f32 - 1.0, ROWS as f32 - 1.0) / 2.0 * UNIT_SPACING;
    let player = players.get(trigger.target()).unwrap();

    info!("spawning units for team `{:?}`", player.team);

    for index in 0..UNITS_COUNT {
        let col = index % COLS;
        let row = index / COLS;

        let grid_position = grid_offset + Vec2::new(col as f32, row as f32) * UNIT_SPACING;
        let position = player.team.spawn_origin() + grid_position;

        commands.spawn((
            Unit { team: player.team },
            Transform::from_translation(position.extend(0.0)),
        ));
    }
}

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

fn select_units(
    trigger: Trigger<Pointer<Drag>>,
    mut commands: Commands,
    camera: Single<(&Camera, &GlobalTransform)>,
    mut selection: ResMut<Selection>,
    units: Query<(Entity, &GlobalTransform, &Aabb, Has<Selected>)>,
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

    for (unit, transform, aabb, prev_selected) in &units {
        let center = transform.translation_vec3a() + aabb.center;
        let rect = Rect::from_center_half_size(center.truncate(), aabb.half_extents.truncate());
        let selected = !selection.rect.intersect(rect).is_empty();
        if selected != prev_selected {
            if selected {
                commands.entity(unit).insert(Selected);
            } else {
                commands.entity(unit).remove::<Selected>();
            }
        }
    }

    Ok(())
}

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

fn move_command(
    trigger: Trigger<Pointer<Pressed>>,
    mut slots: Local<Vec<Vec2>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    mut units: Populated<(&GlobalTransform, &mut Command), With<Selected>>,
) -> Result<()> {
    if trigger.button != PointerButton::Secondary {
        return Ok(());
    }

    let units_count = units.iter().len();
    let cols: usize = (units_count as f32).sqrt().ceil() as usize;
    let rows: usize = ((units_count + cols - 1) / cols).max(1);
    let grid_offset = -Vec2::new(cols as f32 - 1.0, rows as f32 - 1.0) / 2.0 * UNIT_SPACING;

    let (camera, transform) = *camera;
    let click_point = camera.viewport_to_world_2d(transform, trigger.pointer_location.position)?;

    // Orientation basis to make grid facing from group centroid toward the click.
    let positions_sum = units
        .iter()
        .map(|(t, _)| t.translation().truncate())
        .sum::<Vec2>();
    let centroid = positions_sum / units_count as f32;
    let forward = (click_point - centroid).normalize_or(Vec2::Y);
    let right = Vec2::new(forward.y, -forward.x);
    let rotation = Mat2::from_cols(forward, right);

    slots.clear();
    for row in 0..rows {
        for col in 0..cols {
            if slots.len() == units_count {
                break;
            }

            let grid_position = grid_offset + Vec2::new(col as f32, row as f32) * UNIT_SPACING;
            slots.push(click_point + rotation * grid_position);
        }
    }
    debug_assert_eq!(slots.len(), units_count);

    // Pick closest slot for each unit using
    // Hungarian with squared distance as cost.
    let weights: Matrix<_> = units
        .iter()
        .map(|(transform, _)| {
            let position = transform.translation().truncate();
            slots
                .iter()
                .map(|s| position.distance_squared(*s) as i64)
                .collect::<Vec<_>>()
        })
        .collect();
    let (_, slot_for_unit) = kuhn_munkres_min(&weights);

    for (index, (_, mut command)) in units.iter_mut().enumerate() {
        let slot_index = slot_for_unit[index];
        *command = Command::Move(slots[slot_index]);
    }

    Ok(())
}

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
    const PASSTHROUGH_FACTOR: f32 = 0.6; // Reduces the spacing for non-moving units.
    const MOVE_MASS: f32 = 1.0;
    const IDLE_MASS: f32 = 3.0;

    let delta = time.delta_secs();
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

        let speed_factor = (dist / SLOWDOWN_RADIUS).clamp(0.15, 1.0);
        let move_dir = to_target / dist;
        let mut velocity = move_dir * MAX_SPEED * speed_factor;

        let mut separation = Vec2::ZERO;
        for (other_entity, other_pos, other_moving) in &*cached_units {
            if *other_entity == entity {
                continue;
            }

            // movers see idle neighbors as "smaller"
            let min_dist = if *other_moving {
                UNIT_SPACING
            } else {
                UNIT_SPACING * PASSTHROUGH_FACTOR
            };

            let distance = position.distance(*other_pos);
            if distance > 0.0 && distance < min_dist {
                let away_dir = (position - *other_pos) / distance;
                let overlap = min_dist - distance;
                let overlap_ratio = (overlap / min_dist).clamp(0.0, 1.0);
                separation += away_dir * overlap_ratio * SEP_STRENGTH;
            }
        }

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
        if distance <= 0.0 {
            continue;
        }

        let a_moving = matches!(*a_cmd, Command::Move(_));
        let b_moving = matches!(*b_cmd, Command::Move(_));

        // Allow tighter squeeze only when exactly one side is idle.
        let min_dist = if a_moving ^ b_moving {
            UNIT_SPACING * PASSTHROUGH_FACTOR
        } else {
            UNIT_SPACING
        };

        if distance < min_dist {
            let push_dir = offset / distance;
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

    cached_units.clear();
}

const SELECTION_COLOR: Srgba = GREEN_700;

fn draw_selection(mut gizmos: Gizmos, selection: Res<Selection>) {
    gizmos.rect_2d(
        selection.rect.center(),
        selection.rect.size(),
        SELECTION_COLOR,
    );
}

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

#[derive(Resource, Default)]
struct Selection {
    rect: Rect,
    active: bool,
}

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

#[derive(Component, Serialize, Deserialize)]
#[component(immutable)]
#[require(Replicated)]
struct Player {
    team: Team,
}

#[derive(Component)]
struct LocalPlayer;

/// A trigger that instructs the client to mark a specific entity as [`LocalPlayer`].
#[derive(Event, Serialize, Deserialize)]
struct MakeLocal;

#[derive(Event, Serialize, Deserialize)]
struct TeamRequest {
    team: Team,
}

#[derive(Component, Serialize, Deserialize)]
#[component(immutable)]
#[require(Replicated, Command, Mesh2d, MeshMaterial2d<ColorMaterial>)]
struct Unit {
    team: Team,
}

/// Team colors from Warcraft 3.
#[derive(
    Serialize, Deserialize, Debug, ValueEnum, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy,
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

    fn spawn_origin(self) -> Vec2 {
        const COLS: u16 = 4;
        const ROWS: u16 = 2;
        const SPAWN_SPACING: Vec2 = Vec2::splat(200.0);

        let grid_offset = -Vec2::new(COLS as f32 - 1.0, ROWS as f32 - 1.0) / 2.0 * SPAWN_SPACING;

        let index = self as u16;
        let col = index % COLS;
        let row = index / COLS;
        debug_assert!(index < COLS * ROWS);

        grid_offset + Vec2::new(col as f32, row as f32) * SPAWN_SPACING
    }
}

#[derive(Component, Default, Clone, Copy)]
enum Command {
    #[default]
    None,
    Move(Vec2),
}

#[derive(Component)]
#[require(Gizmo)]
struct Selected;
