use core::any::TypeId;

use bevy::{
    ecs::{entity::EntityHashMap, relationship::Relationship},
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

/// Disjoined graphs of related entities.
///
/// Each graph represented by index.
///
/// Updated only when the server is running and cleared on stop.
#[derive(Resource, Default)]
pub(crate) struct RelatedEntities {
    /// Global graph of all relationship types marked for replication in sync.
    ///
    /// We use a stable graph to avoid indices invalidation since we map them to entities and
    /// can't use graphmap because it doesn't support parallel connections (needed when
    /// relationships overlap).
    graph: StableUnGraph<Entity, TypeId>,
    entity_to_node: EntityHashMap<NodeIndex>,
    node_to_entity: HashMap<NodeIndex, Entity>,

    /// Intermediate buffer to store connected edges before removal.
    remove_buffer: Vec<EdgeIndex>,

    /// Indicates whether there were any changes in the graph since the last rebuild.
    rebuild_needed: bool,

    /// Calculates disconnected subgraphs from [`Self::graph`].
    scc: TarjanScc<NodeIndex>,

    /// Maps each entity to its disconnected graph's index.
    entity_graphs: EntityHashMap<usize>,
    graphs_count: usize,
}

impl RelatedEntities {
    fn add_relation<C: Relationship>(&mut self, source: Entity, target: Entity) {
        let source_node = self.register_entity(source);
        let target_node = self.register_entity(target);
        let type_id = TypeId::of::<C>();
        debug!(
            "connecting `{source}` with `{target}` via `{}`",
            ShortName::of::<C>()
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
            ShortName::of::<C>()
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
    pub(crate) fn rebuild_graphs(&mut self) {
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
    pub(crate) fn graph_index(&self, entity: Entity) -> Option<usize> {
        debug_assert!(
            !self.rebuild_needed,
            "`rebuild_graphs` should be called beforehand"
        );
        self.entity_graphs.get(&entity).copied()
    }

    pub(crate) fn graphs_count(&self) -> usize {
        self.graphs_count
    }

    pub(crate) fn clear(&mut self) {
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
pub(crate) fn read_relations<C: Relationship>(
    mut related_entities: ResMut<RelatedEntities>,
    components: Query<(Entity, &C), With<Replicated>>,
) {
    for (entity, relationship) in &components {
        related_entities.add_relation::<C>(entity, relationship.get());
    }
}

pub(crate) fn add_relation<C: Relationship>(
    insert: On<Insert, C>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    components: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = components.get(insert.entity)
    {
        related_entities.add_relation::<C>(insert.entity, relationship.get());
    }
}

pub(crate) fn remove_relation<C: Relationship>(
    replace: On<Replace, C>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    relationships: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = relationships.get(replace.entity)
    {
        related_entities.remove_relation::<C>(replace.entity, relationship.get());
    }
}

pub(crate) fn start_replication<C: Relationship>(
    insert: On<Insert, Replicated>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    components: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = components.get(insert.entity)
    {
        related_entities.add_relation::<C>(insert.entity, relationship.get());
    }
}

pub(crate) fn stop_replication<C: Relationship>(
    replace: On<Replace, Replicated>,
    mut related_entities: ResMut<RelatedEntities>,
    state: Res<State<ServerState>>,
    relationships: Query<&C, With<Replicated>>,
) {
    if *state == ServerState::Running
        && let Ok(relationship) = relationships.get(replace.entity)
    {
        related_entities.remove_relation::<C>(replace.entity, relationship.get());
    }
}
