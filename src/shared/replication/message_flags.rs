use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Types of data included in the update message if the bit is set.
    ///
    /// Serialized at the beginning of the message.
    #[derive(Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
    pub(crate) struct UpdateFlags: u8 {
        const MAPPINGS = 0b00000001;
        const DESPAWNS = 0b00000010;
        const REMOVALS = 0b00000100;
        const CHANGES = 0b00001000;
    }
}

impl UpdateFlags {
    /// Returns the last set flag in the message.
    pub(crate) fn last(self) -> UpdateFlags {
        if self.is_empty() {
            Self::empty()
        } else {
            let zeroes = u8::BITS - 1 - self.bits().leading_zeros();
            UpdateFlags::from_bits_retain(1 << zeroes)
        }
    }
}

bitflags! {
    /// Like [`UpdateFlags`], but for mutate messages.
    #[derive(Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
    pub(crate) struct MutateFlags: u8 {
        const MESSAGES_COUNT = 0b00000001;
        const MUTATIONS = 0b00000010;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last() {
        assert_eq!(UpdateFlags::empty().last(), UpdateFlags::empty());
        assert_eq!(UpdateFlags::CHANGES.last(), UpdateFlags::CHANGES);
        assert_eq!(UpdateFlags::MAPPINGS.last(), UpdateFlags::MAPPINGS);
        assert_eq!(UpdateFlags::all().last(), UpdateFlags::CHANGES);
        assert_eq!(
            (UpdateFlags::DESPAWNS | UpdateFlags::REMOVALS).last(),
            UpdateFlags::REMOVALS
        );
    }
}
