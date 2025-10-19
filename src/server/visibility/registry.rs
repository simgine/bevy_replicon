use bevy::{
    prelude::*,
    utils::{TypeIdMap, TypeIdMapExt},
};

/// Maps filter types to bit indices (0-31).
///
/// This registry assigns each filter type a unique index so entities
/// can store their set of filters as a compact [`u32`] bitmask instead
/// of an allocation-heavy `HashSet<TypeId>`.
///
/// This greatly reduces per-entity memory usage when many entities
/// need to track filters.
#[derive(Resource, Default)]
pub(crate) struct FilterRegistry {
    filters: TypeIdMap<u8>,
    next: u8,
}

impl FilterRegistry {
    pub(super) fn register<F: 'static>(&mut self) {
        if self.next >= u32::BITS as u8 {
            panic!(
                "`{}` can't be registered because the number of filters can't exceed {}",
                ShortName::of::<F>(),
                u32::BITS
            );
        }

        if self.filters.insert_type::<F>(self.next).is_some() {
            panic!(
                "`{}` can't be registered more than once",
                ShortName::of::<F>()
            )
        }

        self.next += 1;
    }

    pub(super) fn get<F: 'static>(&self) -> u8 {
        *self.filters.get_type::<F>().unwrap_or_else(|| {
            panic!(
                "`{}` should've been previously registered",
                ShortName::of::<F>()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get() {
        let mut registry = FilterRegistry::default();
        registry.register::<A>();
        registry.register::<B>();

        assert_eq!(registry.get::<A>(), 0);
        assert_eq!(registry.get::<B>(), 1);
    }

    #[test]
    #[should_panic]
    fn max() {
        let mut registry = FilterRegistry {
            next: 32,
            ..Default::default()
        };
        registry.register::<A>();
    }

    #[test]
    #[should_panic]
    fn duplicate() {
        let mut registry = FilterRegistry::default();
        registry.register::<A>();
        registry.register::<A>();
    }

    struct A;
    struct B;
}
