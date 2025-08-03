use core::{
    any::{self, TypeId},
    mem,
};

use bevy::{
    ecs::component::{Immutable, Mutable},
    prelude::*,
};
use bytes::Bytes;

use super::ctx::{RemoveCtx, WriteCtx};
use crate::{prelude::*, shared::replication::deferred_entity::DeferredEntity};

/// Writing and removal functions for a component, like [`Commands`].
#[derive(Clone, Copy)]
pub(super) struct UntypedCommandFns {
    type_id: TypeId,
    type_name: &'static str,

    write: unsafe fn(),
    remove: RemoveFn,
}

impl UntypedCommandFns {
    /// Creates a new instance with default command functions for `C`.
    pub(super) fn default_fns<C: Component<Mutability: MutWrite<C>>>() -> Self {
        Self::new(C::Mutability::default_write_fn(), default_remove::<C>)
    }

    /// Creates a new instance by erasing the function pointer for `write`.
    pub(super) fn new<C: Component>(write: WriteFn<C>, remove: RemoveFn) -> Self {
        Self {
            type_id: TypeId::of::<C>(),
            type_name: any::type_name::<C>(),
            // SAFETY: the function won't be called until the type is restored.
            write: unsafe { mem::transmute::<WriteFn<C>, unsafe fn()>(write) },
            remove,
        }
    }

    /// Calls the assigned writing function.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same `C` with which this instance was created.
    pub(super) unsafe fn write<C: Component>(
        &self,
        ctx: &mut WriteCtx,
        rule_fns: &RuleFns<C>,
        entity: &mut DeferredEntity,
        message: &mut Bytes,
    ) -> Result<()> {
        debug_assert_eq!(
            self.type_id,
            TypeId::of::<C>(),
            "trying to call a command write function with `{}`, but it was created with `{}`",
            any::type_name::<C>(),
            self.type_name,
        );

        let write: WriteFn<C> = unsafe { mem::transmute(self.write) };
        (write)(ctx, rule_fns, entity, message)
    }

    /// Calls the assigned removal function.
    pub(super) fn remove(&self, ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
        (self.remove)(ctx, entity);
    }
}

/// Defines the default writing function for a [`Component`] based its [`Component::Mutability`].
pub trait MutWrite<C: Component> {
    /// Returns [`default_write`] for [`Mutable`] and [`default_insert_write`] for [`Immutable`].
    fn default_write_fn() -> WriteFn<C>;
}

impl<C: Component<Mutability = Self>> MutWrite<C> for Mutable {
    fn default_write_fn() -> WriteFn<C> {
        default_write::<C>
    }
}

impl<C: Component<Mutability = Self>> MutWrite<C> for Immutable {
    fn default_write_fn() -> WriteFn<C> {
        default_insert_write::<C>
    }
}

/// Signature of component writing function.
pub type WriteFn<C> = fn(&mut WriteCtx, &RuleFns<C>, &mut DeferredEntity, &mut Bytes) -> Result<()>;

/// Signature of component removal functions.
pub type RemoveFn = fn(&mut RemoveCtx, &mut DeferredEntity);

/// Default component writing function for [`Mutable`] components.
///
/// If the component does not exist on the entity, it will be deserialized with [`RuleFns::deserialize`] and inserted via [`Commands`].
/// If the component exists on the entity, [`RuleFns::deserialize_in_place`] will be used directly on the entity's component.
///
/// See also [`default_insert_write`].
pub fn default_write<C: Component<Mutability = Mutable>>(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    if let Some(mut component) = entity.get_mut::<C>() {
        rule_fns.deserialize_in_place(ctx, &mut *component, message)?;
    } else {
        let component: C = rule_fns.deserialize(ctx, message)?;
        entity.insert(component);
    }

    Ok(())
}

/// Default component writing function for [`Immutable`] components.
///
/// The component will be deserialized with [`RuleFns::deserialize`] and inserted via [`Commands`].
///
/// Similar to [`default_write`], but always performs an insertion regardless of whether the component exists.
pub fn default_insert_write<C: Component>(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let component: C = rule_fns.deserialize(ctx, message)?;
    entity.insert(component);
    Ok(())
}

/// Default component removal function.
pub fn default_remove<C: Component>(_ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
    entity.remove::<C>();
}
