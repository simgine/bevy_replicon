use bevy::{
    ecs::{
        bundle::{BundleScratch, BundleWriter},
        component::{ComponentId, Components, ComponentsRegistrator, Mutable},
    },
    prelude::*,
};

/// Like [`EntityWorldMut`], but buffers all structural changes.
///
/// Components are deserialized one by one. To avoid archetype moves or
/// triggering observers before all components have been processed, insertions
/// and removals are buffered and then applied together as a single removal
/// bundle and a single insertion bundle.
#[derive(Deref)]
pub struct DeferredEntity<'w> {
    #[deref]
    entity: EntityWorldMut<'w>,
    buffer: EntityBuffer<'w>,
}

impl<'w> DeferredEntity<'w> {
    /// Wraps an entity with scratch space to make deferred changes.
    ///
    /// For safety / correctness, this will clear the scratch.
    ///
    /// Note that for performance reasons this does _not_ clear the
    /// allocator used for inserted components. To avoid leaking,
    /// make sure insertions are followed by either
    /// [`Self::flush`] or [`EntityScratch::manual_drop`].
    pub fn new(entity: EntityWorldMut<'w>, scratch: &'w mut EntityScratch) -> Self {
        Self {
            entity,
            buffer: scratch.buffer(),
        }
    }

    /// Like [`EntityWorldMut::insert`], but accepts only a single component insertion and buffers it.
    ///
    /// Calling this function multiple times for different components is equivalent to inserting a bundle with them.
    pub fn insert<C: Component>(&mut self, component: C) -> &mut Self {
        // SAFETY: no location update is needed because we only access the registrator
        // from the world, and it is from the same world as the entity.
        unsafe {
            let mut registrator = self.entity.world_mut().components_registrator();
            self.buffer.push_component(&mut registrator, component);
        }
        self
    }

    /// Like [`EntityWorldMut::remove`], but accepts only a single component removal and buffers it.
    ///
    /// Calling this function multiple times for different components is equivalent to removing a bundle with them.
    pub fn remove<C: Component>(&mut self) -> &mut Self {
        // SAFETY: no location update is needed because we only access the registrator.
        let mut registrator = unsafe { self.entity.world_mut().components_registrator() };
        self.buffer.push_removal::<C>(&mut registrator);
        self
    }

    /// Gets mutable access to the component of type `C` for the current entity.
    ///
    /// Returns `None` if the entity does not have a component of type `C`.
    #[inline]
    pub fn get_mut<C: Component<Mutability = Mutable>>(&mut self) -> Option<Mut<'_, C>> {
        self.entity.get_mut()
    }

    /// Returns this entity's world.
    ///
    /// # Safety
    ///
    /// Must only be used to make non-structural ECS changes,
    /// similar to [`DeferredWorld`](bevy::ecs::world::DeferredWorld).
    pub unsafe fn world_mut(&mut self) -> &mut World {
        unsafe { self.entity.world_mut() }
    }

    /// Flushes buffered changes to the entity and clears the scratch.
    pub fn flush(mut self) {
        // SAFETY: All buffered components were recorded using the same world
        // that entity belongs to.
        unsafe { self.buffer.write(&mut self.entity) };
    }
}

#[deprecated(note = "renamed into `EntityScratch`")]
pub type DeferredChanges = EntityScratch;

/// Like [`BundleScratch`], but can also buffer removals.
#[derive(Default)]
pub struct EntityScratch {
    insertions: BundleScratch,
    removals: Vec<ComponentId>,
}

impl EntityScratch {
    fn buffer<'a>(&'a mut self) -> EntityBuffer<'a> {
        debug_assert!(
            self.insertions.is_empty(),
            "insertions should be cleared to avoid leaking"
        );
        self.removals.clear();
        EntityBuffer {
            removals: &mut self.removals,
            insertions: self.insertions.writer(),
        }
    }

    /// Drops all components currently stored in the scratch space.
    ///
    /// # Safety
    ///
    /// `components` must come from the same world as the components that
    /// were pushed into this buffer.
    pub unsafe fn manual_drop(&mut self, components: &Components) {
        unsafe { self.insertions.manual_drop(components) };
    }
}

/// Borrowed buffer used by [`DeferredEntity`] to stage structural changes.
#[derive(Deref, DerefMut)]
struct EntityBuffer<'a> {
    #[deref]
    insertions: BundleWriter<'a>,
    removals: &'a mut Vec<ComponentId>,
}

impl EntityBuffer<'_> {
    fn push_removal<C: Component>(&mut self, registrator: &mut ComponentsRegistrator) {
        let id = registrator.register_component::<C>();
        self.removals.push(id);
    }

    /// Writes all buffered changes to the entity.
    ///
    /// # Safety
    ///
    /// All insertions must have been pushed using the same world
    /// as the entity.
    unsafe fn write(self, entity: &mut EntityWorldMut) {
        if !self.removals.is_empty() {
            entity.remove_by_ids(self.removals);
            self.removals.clear();
        }

        if !self.insertions.is_empty() {
            unsafe { self.insertions.write(entity) };
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use core::any::Any;

    use super::*;

    #[test]
    fn buffering() {
        let mut world = World::new();
        let before_archetypes = world.archetypes().len();
        let mut scratch = EntityScratch::default();
        let mut entity = DeferredEntity::new(world.spawn_empty(), &mut scratch);
        let entity_id = entity.id();

        entity
            .insert(Unit)
            .insert(Trivial(1))
            .insert(WithVec(vec![2, 3]))
            .insert(WithBox(Box::new(Trivial(4))))
            .insert(WithArc(Arc::new(Trivial(5))));

        entity.flush();

        let mut entity = DeferredEntity::new(world.entity_mut(entity_id), &mut scratch);

        assert!(entity.get::<Unit>().is_some());
        assert_eq!(**entity.get::<Trivial>().unwrap(), 1);
        assert_eq!(**entity.get::<WithVec>().unwrap(), [2, 3]);

        let with_box = entity.get::<WithBox>().unwrap();
        assert_eq!(**with_box.downcast_ref::<Trivial>().unwrap(), 4);

        let with_arc = entity.get::<WithArc>().unwrap();
        assert_eq!(Arc::strong_count(with_arc), 1);
        assert_eq!(**with_arc.downcast_ref::<Trivial>().unwrap(), 5);

        let after_archetypes = entity.world().archetypes().len();
        assert_eq!(
            after_archetypes - before_archetypes,
            1,
            "insertions should batch into one archetype move"
        );

        entity
            .remove::<Unit>()
            .remove::<Trivial>()
            .remove::<WithVec>()
            .remove::<WithBox>()
            .remove::<WithArc>();

        entity.flush();

        let entity = world.entity(entity_id);

        assert!(!entity.contains::<Unit>());
        assert!(!entity.contains::<Trivial>());
        assert!(!entity.contains::<WithVec>());
        assert!(!entity.contains::<WithBox>());
        assert!(!entity.contains::<WithArc>());
        assert_eq!(
            world.archetypes().len(),
            after_archetypes,
            "removals shouldn't create intermediate archetypes"
        );
    }

    #[derive(Component)]
    struct Unit;

    #[derive(Component, Deref)]
    struct Trivial(usize);

    #[derive(Component, Deref)]
    struct WithVec(Vec<u8>);

    #[derive(Component, Deref)]
    struct WithBox(Box<dyn Any + Send + Sync>);

    #[derive(Component, Deref)]
    struct WithArc(Arc<dyn Any + Send + Sync>);
}
