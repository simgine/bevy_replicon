use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::{
    bytes::Bytes,
    postcard_utils,
    prelude::*,
    shared::replication::registry::{
        ReplicationRegistry,
        ctx::{SerializeCtx, WriteCtx},
        test_fns::TestFnsEntityExt,
    },
};
use serde::{Deserialize, Serialize};
use test_log::test;

#[test]
fn global() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
        .finish();

    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(
                    world,
                    RuleFns::<TestComponent>::new(serialize_with_scale, deserialize_and_store),
                )
            });

    let mut storage = app.world_mut().resource_mut::<ReplicationStorage>();
    storage.global.insert(Scale(2));

    let mut entity = app.world_mut().spawn(TestComponent(10));
    let data = entity.serialize(fns_id, RepliconTick::default());

    entity.remove::<TestComponent>();
    entity.apply_write(data, fns_id, RepliconTick::default());

    let component = *entity.get::<TestComponent>().unwrap();
    assert_eq!(component, TestComponent(20));

    let storage = app.world().resource::<ReplicationStorage>();
    let last_value = *storage.global.get::<LastValue>().unwrap();
    assert_eq!(last_value, LastValue(20));
}

#[test]
fn entity() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
        .finish();

    let (_, fns_id) =
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.register_rule_fns(
                    world,
                    RuleFns::<TestComponent>::new(
                        serialize_with_entity_scale,
                        deserialize_and_store_entity,
                    ),
                )
            });

    let entity = app.world_mut().spawn(TestComponent(10)).id();

    let mut storage = app.world_mut().resource_mut::<ReplicationStorage>();
    storage.insert(entity, Scale(2));

    let mut entity = app.world_mut().entity_mut(entity);
    let data = entity.serialize(fns_id, RepliconTick::default());

    entity.remove::<TestComponent>();
    entity.apply_write(data, fns_id, RepliconTick::default());

    let component = *entity.get::<TestComponent>().unwrap();
    assert_eq!(component, TestComponent(20));

    let entity = entity.id();
    let storage = app.world().resource::<ReplicationStorage>();
    let last_value = *storage.get::<LastValue>(entity).unwrap();
    assert_eq!(last_value, LastValue(20));
}

#[test]
fn entity_cleanup() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, RepliconPlugins))
        .finish();

    let replicated = app.world_mut().spawn(Replicated).id();
    let remote = app.world_mut().spawn(Remote).id();

    let mut storage = app.world_mut().resource_mut::<ReplicationStorage>();
    storage.insert(replicated, Scale(0));
    storage.insert(remote, Scale(0));

    app.world_mut().entity_mut(replicated).despawn();
    app.world_mut().entity_mut(remote).despawn();

    let storage = app.world_mut().resource_mut::<ReplicationStorage>();
    assert!(storage.entities.is_empty());
}

fn serialize_with_scale(
    ctx: &mut SerializeCtx,
    component: &TestComponent,
    message: &mut Vec<u8>,
) -> Result<()> {
    let scale = ctx.storage.global.get_or_default::<Scale>();
    postcard_utils::to_extend_mut(&(component.0 * scale.0), message)?;
    Ok(())
}

fn deserialize_and_store(ctx: &mut WriteCtx, message: &mut Bytes) -> Result<TestComponent> {
    let value: u8 = postcard_utils::from_buf(message)?;
    ctx.storage.global.insert(LastValue(value));
    Ok(TestComponent(value))
}

fn serialize_with_entity_scale(
    ctx: &mut SerializeCtx,
    component: &TestComponent,
    message: &mut Vec<u8>,
) -> Result<()> {
    let scale = ctx.get_or_default::<Scale>();
    postcard_utils::to_extend_mut(&(component.0 * scale.0), message)?;
    Ok(())
}

fn deserialize_and_store_entity(ctx: &mut WriteCtx, message: &mut Bytes) -> Result<TestComponent> {
    let value: u8 = postcard_utils::from_buf(message)?;
    ctx.insert(LastValue(value));
    Ok(TestComponent(value))
}

#[derive(Component, Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
struct TestComponent(u8);

#[derive(Default, Debug, PartialEq, Clone, Copy)]
struct Scale(u8);

#[derive(Default, Debug, PartialEq, Clone, Copy)]
struct LastValue(u8);
