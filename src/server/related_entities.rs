use bevy::{
    ecs::{component::Immutable, relationship::Relationship},
    prelude::*,
};

pub(super) use crate::send::RelatedEntities;

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
        crate::send::sync_related_entities::<C>(self)
    }
}
