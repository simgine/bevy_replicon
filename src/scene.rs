use bevy::{ecs::entity::hash_map::EntityHashMap, prelude::*, scene::DynamicEntity};
use log::debug;

use crate::{prelude::*, shared::replication::rules::ReplicationRules};

/**
Fills scene with all replicated entities and their components.

Components that are not registered using [`App::register_type`]
or do not have `#[reflect(Component)]` will be skipped.

Entities won't have the [`Replicated`] component.
So on deserialization you need to insert it back if you want entities to continue to replicate.

# Examples

```
use bevy::{asset::ron, prelude::*, state::app::StatesPlugin, scene::serde::SceneDeserializer};
use bevy_replicon::{prelude::*, scene};
use serde::de::DeserializeSeed;
# let mut app = App::new();
# app.add_plugins((StatesPlugin, RepliconPlugins));

// Serialization
let registry = app.world().resource::<AppTypeRegistry>();
let type_registry = &*registry.read();
let mut scene = DynamicScene::default();
scene::replicate_into(&mut scene, &app.world());
let scene = scene
    .serialize(type_registry)
    .expect("scene should be serialized");

// Deserialization
let scene_deserializer = SceneDeserializer { type_registry };
let mut deserializer =
    ron::Deserializer::from_str(&scene).expect("scene should be serialized as valid ron");
let mut scene = scene_deserializer
    .deserialize(&mut deserializer)
    .expect("ron should be convertible to scene");

// Re-insert `Replicated` component.
for entity in &mut scene.entities {
    entity.components.push(Replicated.to_dynamic());
}
```
*/
pub fn replicate_into(scene: &mut DynamicScene, world: &World) {
    let Some(marker_id) = world.component_id::<Replicated>() else {
        // Components are initialized lazily.
        // If there is no replication marker, then we have nothing to replicate.
        return;
    };

    let mut entities: EntityHashMap<_> = scene
        .entities
        .drain(..)
        .map(|dyn_entity| (dyn_entity.entity, dyn_entity.components))
        .collect();

    let registry = world.resource::<AppTypeRegistry>();
    let rules = world.resource::<ReplicationRules>();
    let registry = registry.read();
    for archetype in world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.contains(marker_id))
    {
        // Populate entities ahead of time in order to extract entities without components too.
        for entity in archetype.entities() {
            entities.entry(entity.id()).or_default();
        }

        for rule in rules.iter().filter(|rule| rule.matches(archetype)) {
            for component in &rule.components {
                // SAFETY: replication rules can be registered only with valid component IDs.
                let replicated_component =
                    unsafe { world.components().get_info_unchecked(component.id) };
                let type_name = replicated_component.name();
                let type_id = replicated_component
                    .type_id()
                    .unwrap_or_else(|| panic!("`{type_name}` should be a Rust type"));
                let Some(registration) = registry.get(type_id) else {
                    debug!("ignoring `{type_name}` because it's not registered");
                    continue;
                };
                let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                    debug!("ignoring `{type_name}` because it's missing `#[reflect(Component)]`");
                    continue;
                };
                let from_reflect = registration
                    .data::<ReflectFromReflect>()
                    .unwrap_or_else(|| panic!("`{type_name}` should reflect `FromReflect`"));

                for entity in archetype.entities() {
                    let component = reflect_component
                        .reflect(world.entity(entity.id()))
                        .unwrap_or_else(|| panic!("entity should have `{type_name}`"));

                    // Clone via `FromReflect`. Unlike `PartialReflect::clone_value` this
                    // retains the original type and `ReflectSerialize` type data which is needed to
                    // deserialize.
                    let component = from_reflect
                        .from_reflect(component.as_partial_reflect())
                        .unwrap_or_else(|| panic!("`{type_name}` should be dynamically cloneable"));

                    let components = entities
                        .get_mut(&entity.id())
                        .expect("all entities should be populated ahead of time");

                    debug!("adding `{type_name}` to `{}`", entity.id());
                    components.push(component.into_partial_reflect());
                }
            }
        }
    }

    let dyn_entities_iter = entities
        .drain()
        .map(|(entity, components)| DynamicEntity { entity, components });
    scene.entities.extend(dyn_entities_iter);
}
