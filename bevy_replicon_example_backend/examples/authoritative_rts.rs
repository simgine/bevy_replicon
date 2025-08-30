use bevy::{
    color::palettes::tailwind::{BLUE_500, GREEN_500, RED_500},
    prelude::*,
    render::primitives::Aabb,
};
use bevy_replicon::prelude::*;
use bevy_replicon_example_backend::RepliconExampleBackendPlugins;
use pathfinding::prelude::*;
use serde::{Deserialize, Serialize};

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            RepliconPlugins,
            RepliconExampleBackendPlugins,
        ))
        .init_resource::<Selection>()
        .add_observer(init_unit)
        .add_observer(select_units)
        .add_observer(end_selection)
        .add_observer(clear_selection)
        .add_observer(move_command)
        .add_systems(Startup, spawn_units)
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

const SPACING: f32 = 30.0;

fn spawn_units(mut commands: Commands) {
    commands.spawn(Camera2d);

    const UNITS_COUNT: usize = 50;
    const ROWS: usize = 5;
    const COLS: usize = UNITS_COUNT / ROWS;

    const BLUE_ORIGIN: Vec2 = Vec2::splat(200.0);
    const RED_ORIGIN: Vec2 = Vec2::splat(-200.0);

    let grid_offset = -Vec2::new(COLS as f32 - 1.0, ROWS as f32 - 1.0) / 2.0 * SPACING;

    for index in 0..UNITS_COUNT {
        let row = index / ROWS;
        let col = index % ROWS;

        let grid_position = grid_offset + Vec2::new(row as f32, col as f32) * SPACING;
        let red_position = RED_ORIGIN + grid_position;
        let blue_position = BLUE_ORIGIN + grid_position;

        commands.spawn((
            Unit::Red,
            Transform::from_translation(red_position.extend(0.0)),
        ));
        commands.spawn((
            Unit::Blue,
            Transform::from_translation(blue_position.extend(0.0)),
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

    let unit_material = match unit {
        Unit::Blue => &unit_materials.blue,
        Unit::Red => &unit_materials.red,
    };

    **mesh = unit_mesh.0.clone();
    **material = unit_material.clone();
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
    let grid_offset = -Vec2::new(cols as f32 - 1.0, rows as f32 - 1.0) / 2.0 * SPACING;

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

            let grid_position = grid_offset + Vec2::new(col as f32, row as f32) * SPACING;
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
                SPACING
            } else {
                SPACING * PASSTHROUGH_FACTOR
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
            SPACING * PASSTHROUGH_FACTOR
        } else {
            SPACING
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

fn draw_selection(mut gizmos: Gizmos, selection: Res<Selection>) {
    gizmos.rect_2d(selection.rect.center(), selection.rect.size(), GREEN_500);
}

fn draw_selected(mut gizmos: Gizmos, units: Query<(&GlobalTransform, &Command), With<Selected>>) {
    for (transform, &command) in &units {
        let position = transform.translation().truncate();
        gizmos.circle_2d(position, 15.0, GREEN_500);

        if let Command::Move(translation) = command {
            let isometry = Isometry2d {
                rotation: Rot2::FRAC_PI_4,
                translation,
            };
            gizmos.cross_2d(isometry, 10.0, GREEN_500);
        }
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

struct UnitMaterials {
    blue: Handle<ColorMaterial>,
    red: Handle<ColorMaterial>,
}

impl FromWorld for UnitMaterials {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<ColorMaterial>>();
        let red = materials.add(Color::from(RED_500));
        let blue = materials.add(Color::from(BLUE_500));

        Self { blue, red }
    }
}

#[derive(Component, Serialize, Deserialize)]
#[component(immutable)]
#[require(Replicated, Command, Mesh2d, MeshMaterial2d<ColorMaterial>)]
enum Unit {
    Blue,
    Red,
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
