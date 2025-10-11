use bevy::{ecs::entity::hash_set::EntityHashSet, prelude::*};

/// Entity visibility settings for a client.
///
/// Dynamically marked as required for [`AuthorizedClient`](super::AuthorizedClient).
/// based on [`ServerPlugin::visibility_policy`](super::ServerPlugin::visibility_policy).
///
/// # Examples
///
/// ```
/// use bevy::{prelude::*, state::app::StatesPlugin};
/// use bevy_replicon::prelude::*;
///
/// # let mut app = App::new();
/// app.add_plugins((
///     MinimalPlugins,
///     StatesPlugin,
///     RepliconPlugins.set(ServerPlugin {
///         visibility_policy: VisibilityPolicy::Whitelist, // Makes all entities invisible for clients by default.
///         ..Default::default()
///     }),
/// ))
/// .add_systems(Update, update_visibility.run_if(in_state(ServerState::Running)));
///
/// /// Disables the visibility of other players' entities that are further away than the visible distance.
/// fn update_visibility(
///     mut clients: Query<&mut ClientVisibility>,
///     moved_players: Query<(&Transform, &PlayerOwner), Changed<Transform>>,
///     other_players: Query<(Entity, &Transform, &PlayerOwner)>,
/// ) {
///     for (moved_transform, &owner) in &moved_players {
///         let mut visibility = clients.get_mut(*owner).unwrap();
///         for (entity, transform, _) in other_players
///             .iter()
///             .filter(|&(.., other_owner)| **other_owner != *owner)
///         {
///             const VISIBLE_DISTANCE: f32 = 100.0;
///             let distance = moved_transform.translation.distance(transform.translation);
///             visibility.set_visibility(entity, distance < VISIBLE_DISTANCE);
///         }
///     }
/// }
///
/// /// Points to client entity.
/// #[derive(Component, Deref, Clone, Copy)]
/// struct PlayerOwner(Entity);
/// ```
#[derive(Component)]
pub struct ClientVisibility {
    /// The behavior of [`Self::entities`].
    policy: VisibilityPolicy,

    /// List of entities.
    ///
    /// With [`VisibilityPolicy::Blacklist`] they are hidden.
    /// With [`VisibilityPolicy::Whitelist`] they are visible.
    entities: EntityHashSet,

    /// All entities that lost visibility in this tick.
    lost: EntityHashSet,
}

impl ClientVisibility {
    pub(super) fn blacklist() -> Self {
        Self {
            policy: VisibilityPolicy::Blacklist,
            entities: Default::default(),
            lost: Default::default(),
        }
    }

    pub(super) fn whitelist() -> Self {
        Self {
            policy: VisibilityPolicy::Whitelist,
            entities: Default::default(),
            lost: Default::default(),
        }
    }

    /// Removes a despawned entity tracked by this client.
    pub(super) fn remove_despawned(&mut self, entity: Entity) {
        if self.entities.remove(&entity) {
            self.lost.remove(&entity);
        }
    }

    /// Drains all entities for which visibility was lost during this tick.
    pub(super) fn drain_lost(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.lost.drain()
    }

    /// Sets visibility for a specific entity.
    pub fn set_visibility(&mut self, entity: Entity, visible: bool) {
        match (self.policy, visible) {
            (VisibilityPolicy::Blacklist, true) => {
                if self.entities.remove(&entity) {
                    self.lost.remove(&entity);
                }
            }
            (VisibilityPolicy::Blacklist, false) => {
                if self.entities.insert(entity) {
                    self.lost.insert(entity);
                }
            }
            (VisibilityPolicy::Whitelist, true) => {
                if self.entities.insert(entity) {
                    self.lost.remove(&entity);
                }
            }
            (VisibilityPolicy::Whitelist, false) => {
                if self.entities.remove(&entity) {
                    self.lost.insert(entity);
                }
            }
        }
    }

    /// Checks if a specific entity is visible.
    pub fn is_visible(&self, entity: Entity) -> bool {
        match self.policy {
            VisibilityPolicy::Blacklist => !self.entities.contains(&entity),
            VisibilityPolicy::Whitelist => self.entities.contains(&entity),
        }
    }
}

/// Controls how visibility will be managed via [`ClientVisibility`].
#[derive(Default, Debug, Clone, Copy)]
pub enum VisibilityPolicy {
    /// All entities are visible by default and should be explicitly registered to be hidden.
    #[default]
    Blacklist,
    /// All entities are hidden by default and should be explicitly registered to be visible.
    Whitelist,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_insertion() {
        let mut visibility = ClientVisibility::blacklist();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_empty_removal() {
        let mut visibility = ClientVisibility::blacklist();
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));
        assert!(!visibility.lost.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_removal() {
        let mut visibility = ClientVisibility::blacklist();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));
        assert!(!visibility.lost.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_duplicate_insertion() {
        let mut visibility = ClientVisibility::blacklist();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_insertion() {
        let mut visibility = ClientVisibility::whitelist();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));
        assert!(!visibility.lost.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_empty_removal() {
        let mut visibility = ClientVisibility::whitelist();
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));
        assert!(!visibility.lost.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_removal() {
        let mut visibility = ClientVisibility::whitelist();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));
        assert!(visibility.lost.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_duplicate_insertion() {
        let mut visibility = ClientVisibility::whitelist();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));
        assert!(!visibility.lost.contains(&Entity::PLACEHOLDER));
    }
}
