use core::{alloc::Layout, ptr::NonNull};

use bevy::{
    ecs::component::{ComponentId, Mutable},
    prelude::*,
    ptr::PtrMut,
};

/// Like [`EntityWorldMut`], but buffers all structural changes.
///
/// Components are deserialized one by one, and to avoid causing archetype moves
/// or triggering observers without all components being inserted, we buffer all
/// insertions and removals and apply them as a single removal bundle and a single
/// insertion bundle.
#[derive(Deref)]
pub struct DeferredEntity<'w> {
    #[deref]
    entity: EntityWorldMut<'w>,
    changes: &'w mut DeferredChanges,
}

impl<'w> DeferredEntity<'w> {
    /// Wraps entity with a differed buffer.
    pub fn new(entity: EntityWorldMut<'w>, changes: &'w mut DeferredChanges) -> Self {
        changes.clear();
        Self { entity, changes }
    }

    /// Like [`EntityWorldMut::insert`], but accepts only a single component insertion and buffers it.
    ///
    /// Calling this function multiple times for different components is equivalent to inserting a bundle with them.
    pub fn insert<C: Component>(&mut self, component: C) -> &mut Self {
        let component_id = self.register_component::<C>();
        // SAFETY: component ID belongs to this type.
        unsafe { self.changes.insertions.insert(component, component_id) };
        self
    }

    /// Like [`EntityWorldMut::remove`], but accepts only a single component removal and buffers it.
    ///
    /// Calling this function multiple times for different components is equivalent to removing a bundle with them.
    pub fn remove<C: Component>(&mut self) -> &mut Self {
        let component_id = self.register_component::<C>();
        self.changes.removals.push(component_id);
        self
    }

    /// Gets mutable access to the component of type `C` for the current entity.
    ///
    /// Returns `None` if the entity does not have a component of type `C`.
    #[inline]
    pub fn get_mut<C: Component<Mutability = Mutable>>(&mut self) -> Option<Mut<'_, C>> {
        self.entity.get_mut()
    }

    fn register_component<C: Component>(&mut self) -> ComponentId {
        // SAFETY: no location update is needed because we only register the component ID.
        unsafe { self.entity.world_mut().register_component::<C>() }
    }

    /// Flushes the world and applies all buffered changes.
    ///
    /// Needed to be called after processing each entity
    /// to spawn all allocated entities from mappings.
    pub fn flush(&mut self) {
        // SAFETY: entity location is unchanged because all changes applied after.
        unsafe { self.entity.world_mut().flush() };
        self.changes.apply(&mut self.entity);
    }
}

/// Buffered changes for [`DeferredEntity`].
#[derive(Default)]
pub struct DeferredChanges {
    removals: Vec<ComponentId>,
    insertions: DeferredInsertions,
}

impl DeferredChanges {
    fn apply(&mut self, entity: &mut EntityWorldMut) {
        if !self.removals.is_empty() {
            entity.remove_by_ids(&self.removals);
        }

        if !self.insertions.is_empty() {
            self.insertions.apply(entity);
        }

        self.clear();
    }

    fn clear(&mut self) {
        self.removals.clear();
        self.insertions.clear();
    }
}

/// Buffered insertions stored in type-erased way.
#[derive(Default)]
pub(crate) struct DeferredInsertions {
    ids: Vec<ComponentId>,
    offsets: Vec<usize>,
    data: Vec<u8>,
}

impl DeferredInsertions {
    /// Moves component data into a dynamically allocated buffer.
    ///
    /// # Safety
    ///
    /// Component ID should correspond to the passed type, otherwise [`Self::apply`] won't
    /// write the data correctly.
    unsafe fn insert<C: Component>(&mut self, component: C, component_id: ComponentId) {
        let layout = Layout::new::<C>();

        // If items would otherwise not be aligned, add alignment.
        let align = layout.align();
        let extra_offset = if !self.data.len().is_multiple_of(align) {
            align - (self.data.len() % align)
        } else {
            0
        };

        let grow = layout.size() + extra_offset;
        let offset = self.data.len() + extra_offset;

        self.ids.push(component_id);
        self.offsets.push(offset);
        self.data.resize(self.data.len() + grow, 0);

        // SAFETY: pointer references a properly allocated memory.
        unsafe {
            let ptr = self.data.as_mut_ptr().byte_add(offset) as *mut C;
            ptr.write(component);
        }
    }

    fn apply(&mut self, entity: &mut EntityWorldMut) {
        // SAFETY: iterator produces valid pointers for each component ID.
        unsafe {
            let iter = self.offsets.iter().map(|&offset| {
                let ptr = PtrMut::new(NonNull::new_unchecked(self.data.as_mut_ptr()));
                ptr.byte_add(offset).promote()
            });
            entity.insert_by_ids(&self.ids, iter);
        }
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    fn clear(&mut self) {
        self.ids.clear();
        self.offsets.clear();
        self.data.clear();
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
        let mut changes = DeferredChanges::default();
        let mut entity = DeferredEntity::new(world.spawn_empty(), &mut changes);

        entity
            .insert(Unit)
            .insert(Trivial(1))
            .insert(WithVec(vec![2, 3]))
            .insert(WithBox(Box::new(Trivial(4))))
            .insert(WithArc(Arc::new(Trivial(5))));

        entity.flush();
        let after_archetypes = entity.world().archetypes().len();

        assert!(entity.get::<Unit>().is_some());
        assert_eq!(**entity.get::<Trivial>().unwrap(), 1);
        assert_eq!(**entity.get::<WithVec>().unwrap(), [2, 3]);

        let with_box = entity.get::<WithBox>().unwrap();
        assert_eq!(**with_box.downcast_ref::<Trivial>().unwrap(), 4);

        let with_arc = entity.get::<WithArc>().unwrap();
        assert_eq!(Arc::strong_count(with_arc), 1);
        assert_eq!(**with_arc.downcast_ref::<Trivial>().unwrap(), 5);

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

        assert!(!entity.contains::<Unit>());
        assert!(!entity.contains::<Trivial>());
        assert!(!entity.contains::<WithVec>());
        assert!(!entity.contains::<WithBox>());
        assert!(!entity.contains::<WithArc>());
        assert_eq!(
            entity.world().archetypes().len(),
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
