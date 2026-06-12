use bevy::prelude::*;
use serde::{Serialize, de::DeserializeOwned};

use crate::shared::{replication::delta::diffable::Diffable, replicon_tick::RepliconTick};

pub trait AppRuleExt {
    fn replicate_diff<C>(&mut self, interval: RepliconTick) -> &mut Self
    where
        C: Component + Diffable + Serialize + DeserializeOwned;
}

impl AppRuleExt for App {
    fn replicate_diff<C>(&mut self, interval: RepliconTick) -> &mut Self
    where
        C: Component + Diffable + Serialize + DeserializeOwned,
    {
        let _ = interval;
        todo!()
    }
}
