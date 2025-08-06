use bevy::{
    ecs::{archetype::Archetype, component::ComponentId},
    prelude::*,
};

/// Filter for [`ReplicationRule`](super::ReplicationRule).
#[derive(Clone, Debug)]
pub enum FilterRule {
    /// Corresponds to [`With`].
    With(ComponentId),
    /// Corresponds to [`Without`].
    Without(ComponentId),
    /// Corresponds to [`Or`].
    Or(Vec<FilterRule>),
}

impl FilterRule {
    pub(super) fn matches(&self, archetype: &Archetype) -> bool {
        match self {
            Self::With(id) => archetype.contains(*id),
            Self::Without(id) => !archetype.contains(*id),
            Self::Or(children) => children.iter().any(|child| child.matches(archetype)),
        }
    }
}

/// Types that produce filters for [`ReplicationRule`](super::ReplicationRule).
pub trait FilterRules {
    /// Number of top-level rules.
    ///
    /// Used for preallocation.
    const ARITY: usize;

    /// Priority that is added the default components priority.
    ///
    /// Equal to the number of all rules, including nested.
    const DEFAULT_PRIORITY: usize;

    /// Returns all filters for the type.
    fn filter_rules(world: &mut World) -> Vec<FilterRule> {
        let mut filters = Vec::with_capacity(Self::ARITY);
        Self::push_filters(world, &mut filters);
        filters
    }

    /// Appends filters to a [`Vec`].
    fn push_filters(world: &mut World, filters: &mut Vec<FilterRule>);
}

impl<T: Component> FilterRules for With<T> {
    const ARITY: usize = 1;
    const DEFAULT_PRIORITY: usize = 1;

    fn push_filters(world: &mut World, filters: &mut Vec<FilterRule>) {
        let id = world.register_component::<T>();
        filters.push(FilterRule::With(id));
    }
}

impl<T: Component> FilterRules for Without<T> {
    const ARITY: usize = 1;
    const DEFAULT_PRIORITY: usize = 1;

    fn push_filters(world: &mut World, filters: &mut Vec<FilterRule>) {
        let id = world.register_component::<T>();
        filters.push(FilterRule::Without(id));
    }
}

impl<F: FilterRules> FilterRules for Or<F> {
    const ARITY: usize = 1;
    const DEFAULT_PRIORITY: usize = F::DEFAULT_PRIORITY;

    fn push_filters(world: &mut World, filters: &mut Vec<FilterRule>) {
        filters.push(FilterRule::Or(F::filter_rules(world)));
    }
}

macro_rules! impl_replication_filter {
    ($($name:ident),*) => {
        impl<$($name: FilterRules),*> FilterRules for ($($name,)*) {
            const ARITY: usize = 0 $(+ $name::ARITY)*;
            const DEFAULT_PRIORITY: usize = 0 $(+ $name::DEFAULT_PRIORITY)*;

            fn push_filters(_world: &mut World, _filters: &mut Vec<FilterRule>) {
                $($name::push_filters(_world, _filters);)*
            }
        }
    };
}

variadics_please::all_tuples!(impl_replication_filter, 0, 15, F);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with() {
        let mut world = World::new();
        let [rule] = With::<A>::filter_rules(&mut world).try_into().unwrap();

        assert!(rule.matches(world.spawn(A).archetype()));
        assert!(rule.matches(world.spawn((A, B)).archetype()));
        assert!(!rule.matches(world.spawn(B).archetype()));
    }

    #[test]
    fn without() {
        let mut world = World::new();
        let [rule] = Without::<A>::filter_rules(&mut world).try_into().unwrap();

        assert!(!rule.matches(world.spawn(A).archetype()));
        assert!(!rule.matches(world.spawn((A, B)).archetype()));
        assert!(rule.matches(world.spawn(B).archetype()));
    }

    #[test]
    fn or() {
        type Filters = Or<(With<A>, With<B>)>;
        assert_eq!(Filters::ARITY, 1);
        assert_eq!(Filters::DEFAULT_PRIORITY, 2);

        let mut world = World::new();
        let [rule] = Filters::filter_rules(&mut world).try_into().unwrap();

        assert!(rule.matches(world.spawn(A).archetype()));
        assert!(rule.matches(world.spawn(B).archetype()));
        assert!(rule.matches(world.spawn((A, C)).archetype()));
        assert!(rule.matches(world.spawn((B, C)).archetype()));
        assert!(!rule.matches(world.spawn(C).archetype()));
    }

    #[test]
    fn tuple() {
        type Filters = (With<A>, Without<B>, Or<(With<B>, With<C>)>);
        assert_eq!(Filters::DEFAULT_PRIORITY, 4);
        assert_eq!(Filters::ARITY, 3);

        let mut world = World::new();
        let [with_a, without_b, with_b_or_c] =
            Filters::filter_rules(&mut world).try_into().unwrap();

        let a = world.spawn(A).archetype().id();
        let b = world.spawn(B).archetype().id();
        let ac = world.spawn((A, C)).archetype().id();
        let ab = world.spawn((A, B)).archetype().id();

        let a = world.archetypes().get(a).unwrap();
        let b = world.archetypes().get(b).unwrap();
        let ac = world.archetypes().get(ac).unwrap();
        let ab = world.archetypes().get(ab).unwrap();

        assert!(with_a.matches(a));
        assert!(without_b.matches(a));
        assert!(!with_b_or_c.matches(a));

        assert!(!with_a.matches(b));
        assert!(!without_b.matches(b));
        assert!(with_b_or_c.matches(b));

        assert!(with_a.matches(ac));
        assert!(without_b.matches(ac));
        assert!(with_b_or_c.matches(ac));

        assert!(with_a.matches(ab));
        assert!(!without_b.matches(ab));
        assert!(with_b_or_c.matches(ab));
    }

    #[derive(Component)]
    struct A;

    #[derive(Component)]
    struct B;

    #[derive(Component)]
    struct C;
}
