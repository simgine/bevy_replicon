use bevy::{
    prelude::*,
    utils::{TypeIdMap, TypeIdMapExt},
};

use super::{FilterScope, filters_mask::FilterBit};
use crate::{
    prelude::*,
    shared::replication::registry::{ReplicationRegistry, component_mask::ComponentMask},
};

/// Maps visibility filters data to [`FilterBit`]s.
///
/// This allows entities to store their active filters in a compact
/// [`FiltersMask`](super::filters_mask::FiltersMask) rather than in an
/// allocation-heavy `HashSet<TypeId>`.
///
/// This greatly reduces per-entity memory usage when many entities
/// affected by filters.
#[derive(Resource, Default)]
pub(crate) struct FilterRegistry {
    bits: TypeIdMap<FilterBit>,
    scopes: Vec<VisibilityScope>,
}

impl FilterRegistry {
    pub(super) fn register<F: VisibilityFilter>(
        &mut self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) {
        if self.scopes.len() >= u8::BITS as usize {
            panic!(
                "`{}` can't be registered because the number of filters can't exceed {}",
                ShortName::of::<F>(),
                u32::BITS
            );
        }

        let bit = FilterBit::new(self.scopes.len() as u8);
        if self.bits.insert_type::<F>(bit).is_some() {
            panic!(
                "`{}` can't be registered more than once",
                ShortName::of::<F>()
            )
        }

        let scope = F::Scope::visibility_scope(world, registry);
        self.scopes.push(scope);
    }

    pub(super) fn bit<F: VisibilityFilter>(&self) -> FilterBit {
        *self.bits.get_type::<F>().unwrap_or_else(|| {
            panic!(
                "`{}` should've been previously registered",
                ShortName::of::<F>()
            )
        })
    }

    pub(super) fn scope(&self, bit: FilterBit) -> &VisibilityScope {
        self.scopes
            .get(*bit as usize)
            .unwrap_or_else(|| panic!("component for `{bit:?}` should've been registered"))
    }
}

/// Data affected by [`VisibilityFilter`].
#[derive(Clone)]
pub enum VisibilityScope {
    /// Whole entity.
    Entity,
    /// Specific components on the entity.
    Components(ComponentMask),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::visibility::filters_mask::FiltersMask;

    #[test]
    fn registration() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register::<EntityVisibility>(&mut world, &mut registry);
        filter_registry.register::<ComponentVisibility>(&mut world, &mut registry);
        filter_registry.register::<MultiComponentVisibility>(&mut world, &mut registry);

        let entity_bit = filter_registry.bit::<EntityVisibility>();
        let component_bit = filter_registry.bit::<ComponentVisibility>();
        let multi_component_bit = filter_registry.bit::<MultiComponentVisibility>();
        assert_eq!(entity_bit, FilterBit::new(0));
        assert_eq!(component_bit, FilterBit::new(1));
        assert_eq!(multi_component_bit, FilterBit::new(2));

        assert!(matches!(
            filter_registry.scope(entity_bit),
            VisibilityScope::Entity
        ));
        assert!(matches!(
            filter_registry.scope(component_bit),
            VisibilityScope::Components(_)
        ));
        assert!(matches!(
            filter_registry.scope(multi_component_bit),
            VisibilityScope::Components(_)
        ));
    }

    #[test]
    #[should_panic]
    fn max() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry {
            scopes: vec![VisibilityScope::Entity; 32],
            ..Default::default()
        };
        filter_registry.register::<EntityVisibility>(&mut world, &mut registry);
    }

    #[test]
    #[should_panic]
    fn duplicate() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register::<EntityVisibility>(&mut world, &mut registry);
        filter_registry.register::<EntityVisibility>(&mut world, &mut registry);
    }

    #[test]
    fn entity_visibility() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register::<EntityVisibility>(&mut world, &mut registry);

        let bit = filter_registry.bit::<EntityVisibility>();
        let mut mask = FiltersMask::default();
        mask.insert(bit);

        assert!(mask.is_hidden(&filter_registry));

        let (a_index, _) = registry.init_component_fns::<A>(&mut world);
        assert!(mask.is_component_hidden(&filter_registry, a_index));
    }

    #[test]
    fn component_visibility() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register::<ComponentVisibility>(&mut world, &mut registry);

        let bit = filter_registry.bit::<ComponentVisibility>();
        let mut mask = FiltersMask::default();
        mask.insert(bit);

        assert!(!mask.is_hidden(&filter_registry));
        assert_eq!(
            mask.hidden_components(&filter_registry)
                .flat_map(|m| m.iter())
                .count(),
            1
        );

        let (a_index, _) = registry.init_component_fns::<A>(&mut world);
        assert!(mask.is_component_hidden(&filter_registry, a_index));

        let (b_index, _) = registry.init_component_fns::<B>(&mut world);
        assert!(!mask.is_component_hidden(&filter_registry, b_index));
    }

    #[test]
    fn multi_component_visibility() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register::<MultiComponentVisibility>(&mut world, &mut registry);

        let bit = filter_registry.bit::<MultiComponentVisibility>();
        let mut mask = FiltersMask::default();
        mask.insert(bit);

        assert!(!mask.is_hidden(&filter_registry));
        assert_eq!(
            mask.hidden_components(&filter_registry)
                .flat_map(|m| m.iter())
                .count(),
            2
        );

        let (a_index, _) = registry.init_component_fns::<A>(&mut world);
        assert!(mask.is_component_hidden(&filter_registry, a_index));

        let (b_index, _) = registry.init_component_fns::<B>(&mut world);
        assert!(mask.is_component_hidden(&filter_registry, b_index));

        let (c_index, _) = registry.init_component_fns::<C>(&mut world);
        assert!(!mask.is_component_hidden(&filter_registry, c_index));
    }

    #[derive(Component)]
    #[component(immutable)]
    struct EntityVisibility;

    impl VisibilityFilter for EntityVisibility {
        type Scope = Entity;

        fn is_visible(&self, _entity_filter: &Self) -> bool {
            true
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct ComponentVisibility;

    impl VisibilityFilter for ComponentVisibility {
        type Scope = ComponentScope<A>;

        fn is_visible(&self, _entity_filter: &Self) -> bool {
            true
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct MultiComponentVisibility;

    impl VisibilityFilter for MultiComponentVisibility {
        type Scope = (A, B);

        fn is_visible(&self, _entity_filter: &Self) -> bool {
            true
        }
    }

    #[derive(Component)]
    struct A;

    #[derive(Component)]
    struct B;

    #[derive(Component)]
    struct C;
}
