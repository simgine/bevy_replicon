use bevy::{
    ecs::entity::{hash_map::EntityHashMap, hash_set::EntityHashSet},
    platform::collections::hash_map::Entry,
    prelude::*,
};

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
    /// List of entities.
    list: VisibilityList,

    /// All entities that were added to the list in this tick.
    ///
    /// Visibility of these entities has been gained or removed based on [`Self::list`].
    added: EntityHashSet,

    /// All entities that were removed from the list in this tick.
    ///
    /// Visibility of these entities has been gained or removed based on [`Self::list`].
    removed: EntityHashSet,
}

impl ClientVisibility {
    pub(super) fn blacklist() -> Self {
        Self {
            list: VisibilityList::Blacklist(Default::default()),
            added: Default::default(),
            removed: Default::default(),
        }
    }

    pub(super) fn whitelist() -> Self {
        Self {
            list: VisibilityList::Whitelist(Default::default()),
            added: Default::default(),
            removed: Default::default(),
        }
    }

    /// Updates list information and its sets based on the filter.
    ///
    /// Should be called after each tick.
    pub(super) fn update(&mut self) {
        match &mut self.list {
            VisibilityList::Blacklist(list) => {
                // Remove all entities queued for removal.
                for entity in self.removed.drain() {
                    list.remove(&entity);
                }
                self.added.clear();
            }
            VisibilityList::Whitelist(list) => {
                // Change all recently added entities to `WhitelistInfo::Visible`
                // from `WhitelistInfo::JustVisible`.
                for entity in self.added.drain() {
                    list.insert(entity, WhitelistInfo::Visible);
                }
                self.removed.clear();
            }
        }
    }

    /// Removes a despawned entity tracked by this client.
    pub(super) fn remove_despawned(&mut self, entity: Entity) {
        let removed = match &mut self.list {
            VisibilityList::Blacklist(list) => list.remove(&entity).is_some(),
            VisibilityList::Whitelist(list) => list.remove(&entity).is_some(),
        };

        if removed {
            self.added.remove(&entity);
            self.removed.remove(&entity);
        }
    }

    /// Drains all entities for which visibility was lost during this tick.
    pub(super) fn drain_lost(&mut self) -> impl Iterator<Item = Entity> + '_ {
        match &mut self.list {
            VisibilityList::Blacklist(_) => self.added.drain(),
            VisibilityList::Whitelist(_) => self.removed.drain(),
        }
    }

    /// Sets visibility for a specific entity.
    pub fn set_visibility(&mut self, entity: Entity, visible: bool) {
        match &mut self.list {
            VisibilityList::Blacklist(list) => {
                if visible {
                    // If the entity is already visible, do nothing.
                    let Entry::Occupied(mut entry) = list.entry(entity) else {
                        return;
                    };

                    // If the entity was previously added in this tick, then undo it.
                    if self.added.remove(&entity) {
                        entry.remove();
                        return;
                    }

                    // For blacklisting an entity we don't remove the entity right away.
                    // Instead we mark it as queued for removal and remove it
                    // later in `Self::update`. This allows us to avoid accessing
                    // the blacklist's `removed` field in `Self::visibility_state`.
                    entry.insert(BlacklistInfo::QueuedForRemoval);
                    self.removed.insert(entity);
                } else {
                    // If the entity is already registered, reset its removal status.
                    if list.insert(entity, BlacklistInfo::Hidden).is_some() {
                        self.removed.remove(&entity);
                        return;
                    };

                    self.added.insert(entity);
                }
            }
            VisibilityList::Whitelist(list) => {
                if visible {
                    // Similar to blacklist removal, we don't just add the entity to the list.
                    // Instead we mark it as `WhitelistInfo::JustAdded` and then set it to
                    // 'WhitelistInfo::Visible' in `Self::update`.
                    // This allows us to avoid accessing the whitelist's `added` field in
                    // `Self::visibility_state`.
                    if *list.entry(entity).or_insert(WhitelistInfo::JustAdded)
                        == WhitelistInfo::JustAdded
                    {
                        // Do not mark an entry as newly added if the entry was already in the list.
                        self.added.insert(entity);
                    }
                    self.removed.remove(&entity);
                } else {
                    // If the entity is not in the whitelist, do nothing.
                    if list.remove(&entity).is_none() {
                        return;
                    }

                    // If the entity was added in this tick, then undo it.
                    if self.added.remove(&entity) {
                        return;
                    }

                    self.removed.insert(entity);
                }
            }
        }
    }

    /// Checks if a specific entity is visible.
    pub fn is_visible(&self, entity: Entity) -> bool {
        match self.state(entity) {
            Visibility::Hidden => false,
            Visibility::Gained | Visibility::Visible => true,
        }
    }

    /// Returns visibility of a specific entity.
    pub(super) fn state(&self, entity: Entity) -> Visibility {
        match &self.list {
            VisibilityList::Blacklist(list) => match list.get(&entity) {
                Some(BlacklistInfo::QueuedForRemoval) => Visibility::Gained,
                Some(BlacklistInfo::Hidden) => Visibility::Hidden,
                None => Visibility::Visible,
            },
            VisibilityList::Whitelist(list) => match list.get(&entity) {
                Some(WhitelistInfo::JustAdded) => Visibility::Gained,
                Some(WhitelistInfo::Visible) => Visibility::Visible,
                None => Visibility::Hidden,
            },
        }
    }
}

enum VisibilityList {
    Blacklist(EntityHashMap<BlacklistInfo>),
    Whitelist(EntityHashMap<WhitelistInfo>),
}

#[derive(PartialEq, Clone, Copy)]
enum WhitelistInfo {
    Visible,
    JustAdded,
}

#[derive(PartialEq, Clone, Copy)]
enum BlacklistInfo {
    Hidden,
    QueuedForRemoval,
}

/// Visibility state for an entity in the current tick, from the perspective of one client.
///
/// Note that the distinction between 'lost visibility' and 'don't have visibility' is not exposed here.
/// There is only [`Visibility::Hidden`] to encompass both variants.
///
/// Lost visibility is handled separately with [`ClientVisibility::drain_lost_visibility`].
#[derive(PartialEq, Default, Clone, Copy)]
pub(crate) enum Visibility {
    /// The client does not have visibility of the entity in this tick.
    #[default]
    Hidden,
    /// The client gained visibility of the entity in this tick (it was not visible in the previous tick).
    Gained,
    /// The entity is visible to the client (and was visible in the previous tick).
    Visible,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_insertion() {
        let mut visibility = ClientVisibility::blacklist();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Blacklist(list) = &visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityList::Blacklist(list) = &visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_empty_removal() {
        let mut visibility = ClientVisibility::blacklist();
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Blacklist(list) = visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_removal() {
        let mut visibility = ClientVisibility::blacklist();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        visibility.update();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Blacklist(list) = &visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(visibility.removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityList::Blacklist(list) = &visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_insertion_removal() {
        let mut visibility = ClientVisibility::blacklist();

        // Insert and remove from the list.
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Blacklist(list) = visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn blacklist_duplicate_insertion() {
        let mut visibility = ClientVisibility::blacklist();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        visibility.update();

        // Duplicate insertion.
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Blacklist(list) = visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_insertion() {
        let mut visibility = ClientVisibility::whitelist();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Whitelist(list) = &visibility.list else {
            panic!("filter should be a whitelist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityList::Whitelist(list) = &visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_empty_removal() {
        let mut visibility = ClientVisibility::whitelist();
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Whitelist(list) = visibility.list else {
            panic!("filter should be a whitelist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_removal() {
        let mut visibility = ClientVisibility::whitelist();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        visibility.update();
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Whitelist(list) = &visibility.list else {
            panic!("filter should be a whitelist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(visibility.removed.contains(&Entity::PLACEHOLDER));

        visibility.update();

        let VisibilityList::Whitelist(list) = &visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_insertion_removal() {
        let mut visibility = ClientVisibility::whitelist();

        // Insert and remove from the list.
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        visibility.set_visibility(Entity::PLACEHOLDER, false);
        assert!(!visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Whitelist(list) = visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(!list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }

    #[test]
    fn whitelist_duplicate_insertion() {
        let mut visibility = ClientVisibility::whitelist();
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        visibility.update();

        // Duplicate insertion.
        visibility.set_visibility(Entity::PLACEHOLDER, true);
        assert!(visibility.is_visible(Entity::PLACEHOLDER));

        let VisibilityList::Whitelist(list) = visibility.list else {
            panic!("filter should be a blacklist");
        };

        assert!(list.contains_key(&Entity::PLACEHOLDER));
        assert!(!visibility.added.contains(&Entity::PLACEHOLDER));
        assert!(!visibility.removed.contains(&Entity::PLACEHOLDER));
    }
}
