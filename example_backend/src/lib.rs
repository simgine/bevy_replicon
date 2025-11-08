//! A simple transport intended only for examples.
//! This transport does not implement any reliability or security features.
//! DO NOT USE in a real project
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "client")]
mod client;
mod link_conditioner;
#[cfg(feature = "server")]
mod server;
mod tcp;

#[cfg(feature = "client")]
pub use client::*;
pub use link_conditioner::*;
#[cfg(feature = "server")]
pub use server::*;

use bevy::{app::PluginGroupBuilder, prelude::*};

/// Plugin group for all replicon example backend plugins.
///
/// Contains the following:
/// * [`RepliconExampleServerPlugin`] - with feature `server`.
/// * [`RepliconExampleClientPlugin`] - with feature `client`.
pub struct RepliconExampleBackendPlugins;

impl PluginGroup for RepliconExampleBackendPlugins {
    fn build(self) -> PluginGroupBuilder {
        let mut group = PluginGroupBuilder::start::<Self>().add(LinkConditionerPlugin);

        #[cfg(feature = "server")]
        {
            group = group.add(RepliconExampleServerPlugin);
        }

        #[cfg(feature = "client")]
        {
            group = group.add(RepliconExampleClientPlugin);
        }

        group
    }
}
