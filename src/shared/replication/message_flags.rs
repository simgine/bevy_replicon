use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Types of data included in the update message if the bit is set.
    ///
    /// Serialized at the beginning of the message.
    #[derive(Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
    pub(crate) struct UpdateMessageFlags: u8 {
        const MAPPINGS = 0b00000001;
        const DESPAWNS = 0b00000010;
        const REMOVALS = 0b00000100;
        const CHANGES = 0b00001000;
    }
}

impl UpdateMessageFlags {
    /// Returns the last set flag in the message.
    pub(crate) fn last(self) -> UpdateMessageFlags {
        debug_assert!(!self.is_empty());
        let zeroes = u8::BITS - 1 - self.bits().leading_zeros();
        UpdateMessageFlags::from_bits_retain(1 << zeroes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last() {
        assert_eq!(
            UpdateMessageFlags::CHANGES.last(),
            UpdateMessageFlags::CHANGES
        );
        assert_eq!(
            UpdateMessageFlags::MAPPINGS.last(),
            UpdateMessageFlags::MAPPINGS
        );
        assert_eq!(
            UpdateMessageFlags::all().last(),
            UpdateMessageFlags::CHANGES
        );
        assert_eq!(
            (UpdateMessageFlags::DESPAWNS | UpdateMessageFlags::REMOVALS).last(),
            UpdateMessageFlags::REMOVALS
        );
    }
}
