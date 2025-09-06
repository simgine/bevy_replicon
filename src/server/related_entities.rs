use core::any::{self, TypeId};

use bevy::{
    ecs::{component::Immutable, relationship::Relationship},
    platform::collections::HashMap,
    prelude::*,
};
use log::{debug, trace};
use petgraph::{
    Direction,
    algo::TarjanScc,
    graph::{EdgeIndex, NodeIndex},
    prelude::StableUnGraph,
    visit::EdgeRef,
};

use crate::prelude::*;

pub trait SyncRelatedAppExt {
    /// Ensures that entities related by `C` are replicated in sync.
    ///
    /// By default, we split mutations across multiple messages to apply them independently.
    /// We guarantee that all mutations for a single entity won't be split across messages,
    /// but mutations for separate entities may be received independently if they arrive in
    /// different messages.
    ///
    /// Calling this method guarantees that all mutations related by `C` are included in
    /// a single message.
    ///
    /// Internally we maintain a graph of all relationship types marked for replication in sync.
    /// It's updated via observers, so frequent changes may impact the performance.
    ///
    /// # Examples
    /// ```
    /// # use bevy::state::app::StatesPlugin;
    /// use bevy::prelude::*;
    /// use bevy_replicon::prelude::*;
    ///
    /// # let mut app = App::new();
    /// # app.add_plugins((StatesPlugin, RepliconPlugins));
    /// app.sync_related_entities::<ChildOf>();
    ///
    /// // Changes to any replicated components on these
    /// // entities will be replicated in sync.
    /// app.world_mut().spawn((
    ///     Replicated,
    ///     Transform::default(),
    ///     children![(Replicated, Transform::default())],
    /// ));
    /// ```
    fn sync_related_entities<C>(&mut self) -> &mut Self
    where
        C: Relationship + Component<Mutability = Immutable>;
}

impl SyncRelatedAppExt for App {
    fn sync_related_entities<C>(&mut self) -> &mut Self
    where
        C: Relationship + Component<Mutability = Immutable>,
    {
        self.add_systems(
            OnEnter(ServerState::Running),
            read_relations::<C>.in_set(ServerSet::ReadRelations),
        )
        .add_observer(add_relation::<C>)
        .add_observer(remove_relation::<C>)
        .add_observer(start_replication::<C>)
        .add_observer(stop_replication::<C>)
    }
}

/// Disjoined graphs of related entities.
///
/// Each graph represented by index.
///
/// Updated only when the server is running and cleared on stop.
#[derive(Resource, Default)]
pub(super) struct RelatedEntities {
    /// Global graph of all relationship types marked for replication in sync.
    ///
    /// We use a stable graph to avoid indices invalidation since we map them to entities and
    /// can't use graphmap because it doesn't support parallel connections (needed when
    /// relationships overlap).
    graph: StableUnGraph<Entity, TypeId>,
    entity_to_node: HashMap<Entity, NodeIndex>,
    node_to_entity: HashMap<NodeIndex, Entity>,

    /// Intermediate buffer to store connected edges before removal.
    remove_buffer: Vec<EdgeIndex>,

    /// Indicates whether there were any changes in the graph since the last rebuild.
    rebuild_needed: bool,

    /// Calculates disconnected subgraphs from [`Self::graph`].
    scc: TarjanScc<NodeIndex>,

    /// Maps each entity to its disconnected graph's index.
    entity_graphs: HashMap<Entity, usize>,
    graphs_count: usize,
}

impl RelatedEntities {
    fn add_relation<C: Relationship>(&mut self, source: Entity, target: Entity) {
        let source_node = self.register_entity(source);
        let target_node = self.register_entity(target);
        let type_id = TypeId::of::<C>();
        debug!(
            "connecting `{source}` with `{target}` via `{}`",
            any::type_name::<C>()
        );

        self.graph.add_edge(source_node, target_node, type_id);
        self.rebuild_needed = true;
    }

    fn remove_relation<C: Relationship>(&mut self, source: Entity, target: Entity) {
        let Some(&source_node) = self.entity_to_node.get(&source) else {
            return;
        };
        let Some(&target_node) = self.entity_to_node.get(&target) else {
            return;
        };

        let type_id = TypeId::of::<C>();
        debug!(
            "disconnecting `{source}` from `{target}` via `{}`",
            any::type_name::<C>()
        );

        // Remove all matching edges of this type.
        self.remove_buffer.extend(
            self.graph
                .edges_connecting(source_node, target_node)
                .filter(|e| *e.weight() == type_id)
                .map(|e| e.id()),
        );

        for edge in self.remove_buffer.drain(..) {
            self.graph.remove_edge(edge);
        }

        if self.is_orphan(target_node) {
            self.remove_entity(target, target_node);
        }

        if self.is_orphan(source_node) {
            self.remove_entity(source, source_node);
        }

        self.rebuild_needed = true;
    }

    fn register_entity(&mut self, entity: Entity) -> NodeIndex {
        if let Some(&node) = self.entity_to_node.get(&entity) {
            return node;
        }

        let node = self.graph.add_node(entity);
        self.entity_to_node.insert(entity, node);
        self.node_to_entity.insert(node, entity);
        node
    }

    fn is_orphan(&self, node: NodeIndex) -> bool {
        let incoming = self
            .graph
            .edges_directed(node, Direction::Incoming)
            .next()
            .is_some();
        let outcoming = self
            .graph
            .edges_directed(node, Direction::Outgoing)
            .next()
            .is_some();
        !incoming && !outcoming
    }

    fn remove_entity(&mut self, entity: Entity, node: NodeIndex) {
        debug!("removing orphan `{entity}`");
        self.graph.remove_node(node);
        self.entity_to_node.remove(&entity);
        self.node_to_entity.remove(&node);
    }

    /// Recalculates graphs from SCC if there were any changes.
    ///
    /// The recalculation is not incremental, so it isn't performed automatically
    /// on every change. Instead, manually call this before replication begins.
    ///
    /// Benchmarks show the performance impact is negligible.
    /// The biggest overhead comes from keeping the main graph in sync via observers.
    pub(super) fn rebuild_graphs(&mut self) {
        if !self.rebuild_needed {
            return;
        }
        self.rebuild_needed = false;

        debug!("rebuilding graphs");
        self.graphs_count = 0;
        self.entity_graphs.clear();
        self.scc.run(&self.graph, |nodes| {
            for node in nodes {
                let entity = self.node_to_entity[node];
                self.entity_graphs.insert(entity, self.graphs_count);
                trace!("assigning `{entity}` to graph {}`", self.graphs_count);
            }
            self.graphs_count += 1;
        });
    }

    /// Returns graph index for an entity if it has a relationship.
    ///
    /// Should be called only after [`Self::rebuild_graphs`]
    pub(super) fn graph_index(&self, entity: Entity) -> Option<usize> {
        debug_assert!(
            !self.rebuild_needed,
            "`rebuild_graphs` should be called beforehand"
        );
        self.entity_graphs.get(&entity).copied()
    }

    pub(super) fn graphs_count(&self) -> usize {
        self.graphs_count
    }

    pub(super) fn clear(&mut self) {
        self.graph.clear();
        self.entity_to_node.clear();
        self.node_to_entity.clear();
        self.rebuild_needed = false;
        self.entity_graphs.clear();
        self.graphs_count = 0;
    }
}

/// Collects all existing relations.
///
/// Used to gather previously spawned entities when the server starts,
/// since [`add_relation`] triggers only on hierarchy changes.
fn read_relations<C: Relationship>(
    mut related_entities: ResMut<RelatedEntities>,
    components: Query<(Entity, &C), With<Replicated>>,
) {
    for (entity, relationship) in &components {
        related_entities.add_relation::<C>(entity, relationship.get());
    }
}

fn add_relation<C: Relationship>(
    trigger: Trigger<OnInsert, C>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    components: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = components.get(trigger.target())
    {
        related_entities.add_relation::<C>(trigger.target(), relationship.get());
    }
}

fn remove_relation<C: Relationship>(
    trigger: Trigger<OnReplace, C>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    relationships: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = relationships.get(trigger.target())
    {
        related_entities.remove_relation::<C>(trigger.target(), relationship.get());
    }
}

fn start_replication<C: Relationship>(
    trigger: Trigger<OnInsert, Replicated>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    components: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = components.get(trigger.target())
    {
        related_entities.add_relation::<C>(trigger.target(), relationship.get());
    }
}

fn stop_replication<C: Relationship>(
    trigger: Trigger<OnReplace, Replicated>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    relationships: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = relationships.get(trigger.target())
    {
        related_entities.remove_relation::<C>(trigger.target(), relationship.get());
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;
    use test_log::test;

    use super::*;

    #[test]
    fn orphan() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let entity1 = app
            .world_mut()
            .spawn((Replicated, Children::default()))
            .id();
        let entity2 = app
            .world_mut()
            .spawn((Replicated, Children::default()))
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(entity1), None);
        assert_eq!(related.graph_index(entity2), None);
    }

    #[test]
    fn single() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let child1 = app.world_mut().spawn(Replicated).id();
        let child2 = app.world_mut().spawn(Replicated).id();
        let root = app
            .world_mut()
            .spawn(Replicated)
            .add_children(&[child1, child2])
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[test]
    fn disjoint() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let child1 = app.world_mut().spawn(Replicated).id();
        let root1 = app.world_mut().spawn(Replicated).add_child(child1).id();
        let child2 = app.world_mut().spawn(Replicated).id();
        let root2 = app.world_mut().spawn(Replicated).add_child(child2).id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 2);
        assert_eq!(related.graph_index(root1), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(root2), Some(1));
        assert_eq!(related.graph_index(child2), Some(1));
    }

    #[test]
    fn nested() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let grandchild = app.world_mut().spawn(Replicated).id();
        let child = app.world_mut().spawn(Replicated).add_child(grandchild).id();
        let root = app.world_mut().spawn(Replicated).add_child(child).id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
        assert_eq!(related.graph_index(grandchild), Some(0));
    }

    #[test]
    fn split() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let grandgrandchild = app.world_mut().spawn(Replicated).id();
        let grandchild = app
            .world_mut()
            .spawn(Replicated)
            .add_child(grandgrandchild)
            .id();
        let child = app.world_mut().spawn(Replicated).add_child(grandchild).id();
        let root = app.world_mut().spawn(Replicated).add_child(child).id();

        app.world_mut().entity_mut(grandchild).remove::<ChildOf>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 2);
        assert_eq!(related.graph_index(root), Some(1));
        assert_eq!(related.graph_index(child), Some(1));
        assert_eq!(related.graph_index(grandchild), Some(0));
        assert_eq!(related.graph_index(grandgrandchild), Some(0));
    }

    #[test]
    fn join() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let child1 = app.world_mut().spawn(Replicated).id();
        let root1 = app.world_mut().spawn(Replicated).add_child(child1).id();
        let child2 = app.world_mut().spawn(Replicated).id();
        let root2 = app.world_mut().spawn(Replicated).add_child(child2).id();

        app.world_mut().entity_mut(child1).add_child(root2);

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root1), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(root2), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[test]
    fn reparent() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let child1 = app.world_mut().spawn(Replicated).id();
        let root1 = app.world_mut().spawn(Replicated).add_child(child1).id();
        let child2 = app.world_mut().spawn(Replicated).id();
        let root2 = app.world_mut().spawn(Replicated).add_child(child2).id();

        app.world_mut().entity_mut(child1).insert(ChildOf(root2));

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root1), None);
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(root2), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[test]
    fn orphan_after_split() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let child = app.world_mut().spawn(Replicated).id();
        let root = app.world_mut().spawn(Replicated).add_child(child).id();

        app.world_mut().entity_mut(child).remove::<ChildOf>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child), None);
    }

    #[test]
    fn despawn() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let child1 = app.world_mut().spawn(Replicated).id();
        let child2 = app.world_mut().spawn(Replicated).id();
        let root = app
            .world_mut()
            .spawn(Replicated)
            .add_children(&[child1, child2])
            .id();

        app.world_mut().entity_mut(root).despawn();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child1), None);
        assert_eq!(related.graph_index(child2), None);
    }

    #[test]
    fn intersection() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let child = app.world_mut().spawn(Replicated).id();
        let root1 = app.world_mut().spawn(Replicated).add_child(child).id();
        let root2 = app
            .world_mut()
            .spawn(Replicated)
            .add_one_related::<OwnedBy>(child)
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root1), Some(0));
        assert_eq!(related.graph_index(root2), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn overlap() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let child = app.world_mut().spawn(Replicated).id();
        let root = app
            .world_mut()
            .spawn(Replicated)
            .add_child(child)
            .add_one_related::<OwnedBy>(child)
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn overlap_removal() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let child = app.world_mut().spawn(Replicated).id();
        let root = app
            .world_mut()
            .spawn(Replicated)
            .add_child(child)
            .add_one_related::<OwnedBy>(child)
            .id();

        app.world_mut().entity_mut(child).remove::<ChildOf>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn connected() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let grandchild = app.world_mut().spawn(Replicated).id();
        let child = app.world_mut().spawn(Replicated).add_child(grandchild).id();
        let root = app
            .world_mut()
            .spawn(Replicated)
            .add_one_related::<OwnedBy>(child)
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
        assert_eq!(related.graph_index(grandchild), Some(0));
    }

    #[test]
    fn replication_start() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let child = app.world_mut().spawn_empty().id();
        let root = app.world_mut().spawn_empty().add_child(child).id();

        app.world_mut().entity_mut(child).insert(Replicated);
        app.world_mut().entity_mut(root).insert(Replicated);

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child), Some(0));
    }

    #[test]
    fn replication_stop() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(ServerState::Running)
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>()
            .sync_related_entities::<OwnedBy>();

        let child = app.world_mut().spawn(Replicated).id();
        let root = app
            .world_mut()
            .spawn(Replicated)
            .add_child(child)
            .add_one_related::<OwnedBy>(child)
            .id();

        app.world_mut().entity_mut(child).remove::<Replicated>();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child), None);
    }

    #[test]
    fn runs_only_with_server() {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .init_state::<ServerState>()
            .init_resource::<RelatedEntities>()
            .sync_related_entities::<ChildOf>();

        let child1 = app.world_mut().spawn(Replicated).id();
        let child2 = app.world_mut().spawn(Replicated).id();
        let root = app
            .world_mut()
            .spawn(Replicated)
            .add_children(&[child1, child2])
            .id();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 0);
        assert_eq!(related.graph_index(root), None);
        assert_eq!(related.graph_index(child1), None);
        assert_eq!(related.graph_index(child2), None);

        app.world_mut()
            .resource_mut::<NextState<ServerState>>()
            .set(ServerState::Running);

        app.update();

        let mut related = app.world_mut().resource_mut::<RelatedEntities>();
        related.rebuild_graphs();
        assert_eq!(related.graphs_count(), 1);
        assert_eq!(related.graph_index(root), Some(0));
        assert_eq!(related.graph_index(child1), Some(0));
        assert_eq!(related.graph_index(child2), Some(0));
    }

    #[derive(Component)]
    #[relationship(relationship_target = Owning)]
    struct OwnedBy(Entity);

    #[derive(Component)]
    #[relationship_target(relationship = OwnedBy)]
    struct Owning(Vec<Entity>);
}
