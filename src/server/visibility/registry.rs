use bevy::{
    prelude::*,
    utils::{TypeIdMap, TypeIdMapExt},
};

use super::{FilterScope, filters_mask::FilterBit};
use crate::shared::replication::visibility::ScopeLifetime;
use crate::{
    prelude::*,
    shared::replication::{registry::ReplicationRegistry, visibility::VisibilityScope},
};

/// Maps the [`VisibilityScope`] of each filter to a [`FilterBit`].
///
/// This allows entities to store their active filters as a mask
/// rather than in an allocation-heavy `HashSet<TypeId>`.
///
/// This greatly reduces per-entity memory usage when many entities
/// affected by filters.
#[derive(Resource, Default)]
pub struct FilterRegistry {
    bits: TypeIdMap<FilterBit>,
    scopes: Vec<(VisibilityScope, ScopeLifetime)>,
}

impl FilterRegistry {
    pub(super) fn register_filter<F: VisibilityFilter>(
        &mut self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
    ) {
        let bit = self.register_scope::<F::Scope>(world, registry, F::LIFETIME);
        if self.bits.insert_type::<F>(bit).is_some() {
            panic!(
                "`{}` can't be registered more than once",
                ShortName::of::<F>()
            )
        }
    }

    /// Registers a new visibility scope and returns the [`FilterBit`] assigned to it.
    ///
    /// The returned bit should be managed manually via
    /// [`ClientVisibility::set`](super::client_visibility::ClientVisibility::set) to
    /// control visibility.
    ///
    /// # Panics
    ///
    /// Panics if the number of registered visibility scopes exceeds [`u32::BITS`].
    pub fn register_scope<S: FilterScope>(
        &mut self,
        world: &mut World,
        registry: &mut ReplicationRegistry,
        lifetime: ScopeLifetime,
    ) -> FilterBit {
        if self.scopes.len() >= u32::BITS as usize {
            panic!("number of visibility scopes can't exceed {}", u32::BITS);
        }

        let bit = FilterBit::new(self.scopes.len() as u8);
        let scope = S::visibility_scope(world, registry);
        self.scopes.push((scope, lifetime));
        bit
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
            .map(|(scope, _)| scope)
            .unwrap_or_else(|| panic!("scope for `{bit:?}` should've been registered"))
    }

    pub(super) fn lifetime(&self, bit: FilterBit) -> ScopeLifetime {
        self.scopes
            .get(*bit as usize)
            .map(|&(_, lifetime)| lifetime)
            .unwrap_or_else(|| panic!("lifetime for `{bit:?}` should've been registered"))
    }
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
        filter_registry.register_filter::<EntityVisibility>(&mut world, &mut registry);
        filter_registry.register_filter::<ComponentVisibility>(&mut world, &mut registry);
        filter_registry.register_filter::<MultiComponentVisibility>(&mut world, &mut registry);

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
    fn max() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry {
            scopes: vec![(VisibilityScope::Entity, ScopeLifetime::WhileVisible); 31],
            ..Default::default()
        };
        filter_registry.register_filter::<EntityVisibility>(&mut world, &mut registry);
    }

    #[test]
    #[should_panic]
    fn beyond_max() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry {
            scopes: vec![(VisibilityScope::Entity, ScopeLifetime::WhileVisible); 32],
            ..Default::default()
        };
        filter_registry.register_filter::<EntityVisibility>(&mut world, &mut registry);
    }

    #[test]
    #[should_panic]
    fn duplicate() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register_filter::<EntityVisibility>(&mut world, &mut registry);
        filter_registry.register_filter::<EntityVisibility>(&mut world, &mut registry);
    }

    #[test]
    fn entity_visibility() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register_filter::<EntityVisibility>(&mut world, &mut registry);

        let bit = filter_registry.bit::<EntityVisibility>();
        let mut mask = FiltersMask::default();
        mask.insert(bit);

        assert!(mask.hides_entity(&filter_registry, ScopeLifetime::WhileVisible));

        let (a_index, _) = registry.init_component_fns::<A>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, a_index)
                .is_some()
        );
    }

    #[test]
    fn component_visibility() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register_filter::<ComponentVisibility>(&mut world, &mut registry);

        let bit = filter_registry.bit::<ComponentVisibility>();
        let mut mask = FiltersMask::default();
        mask.insert(bit);

        assert!(!mask.hides_entity(&filter_registry, ScopeLifetime::WhileVisible));

        let (a_index, _) = registry.init_component_fns::<A>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, a_index)
                .is_some()
        );

        let (b_index, _) = registry.init_component_fns::<B>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, b_index)
                .is_none()
        );
    }

    #[test]
    fn multi_component_visibility() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register_filter::<MultiComponentVisibility>(&mut world, &mut registry);

        let bit = filter_registry.bit::<MultiComponentVisibility>();
        let mut mask = FiltersMask::default();
        mask.insert(bit);

        assert!(!mask.hides_entity(&filter_registry, ScopeLifetime::WhileVisible));

        let (a_index, _) = registry.init_component_fns::<A>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, a_index)
                .is_some()
        );

        let (b_index, _) = registry.init_component_fns::<B>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, b_index)
                .is_some()
        );

        let (c_index, _) = registry.init_component_fns::<C>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, c_index)
                .is_none()
        );
    }

    #[test]
    fn all_except_visibility() {
        let mut world = World::new();
        let mut registry = ReplicationRegistry::default();
        let mut filter_registry = FilterRegistry::default();
        filter_registry.register_filter::<AllExceptVisibility>(&mut world, &mut registry);

        let bit = filter_registry.bit::<AllExceptVisibility>();
        let mut mask = FiltersMask::default();
        mask.insert(bit);

        assert!(!mask.hides_entity(&filter_registry, ScopeLifetime::WhileVisible));

        let (a_index, _) = registry.init_component_fns::<A>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, a_index)
                .is_none()
        );

        let (b_index, _) = registry.init_component_fns::<B>(&mut world);
        assert!(
            mask.hidden_component_lifetime(&filter_registry, b_index)
                .is_some()
        );
    }

    #[derive(Component)]
    #[component(immutable)]
    struct EntityVisibility;

    impl VisibilityFilter for EntityVisibility {
        type ClientComponent = Self;
        type Scope = Entity;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some()
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct ComponentVisibility;

    impl VisibilityFilter for ComponentVisibility {
        type ClientComponent = Self;
        type Scope = SingleComponent<A>;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some()
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct MultiComponentVisibility;

    impl VisibilityFilter for MultiComponentVisibility {
        type ClientComponent = Self;
        type Scope = (A, B);

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some()
        }
    }

    #[derive(Component)]
    #[component(immutable)]
    struct AllExceptVisibility;

    impl VisibilityFilter for AllExceptVisibility {
        type ClientComponent = Self;
        type Scope = AllExcept<SingleComponent<A>>;

        fn is_visible(&self, _client: Entity, component: Option<&Self::ClientComponent>) -> bool {
            component.is_some()
        }
    }

    #[derive(Component)]
    struct A;

    #[derive(Component)]
    struct B;

    #[derive(Component)]
    struct C;
}
