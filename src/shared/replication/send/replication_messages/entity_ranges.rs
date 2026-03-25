use alloc::vec::Vec;
use core::ops::Range;

use bevy::prelude::*;
use postcard::experimental::serialized_size;

/// Component insertions, mutations or removals for an entity in form of serialized ranges
/// from [`SerializedData`](super::serialized_data::SerializedData).
///
/// Used inside [`Updates`](super::updates::Updates) and
/// [`Mutations`](super::mutations::Mutations).
///
/// For data, we serialize the size in bytes rather than the number of elements to
/// allow entities to be skipped during deserialization. For example, received mutations
/// might be outdated, or the entity might have been despawned via client-side prediction.
pub(super) struct EntityRanges {
    pub(super) entity: Range<usize>,
    pub(super) data: Vec<Range<usize>>,
}

impl EntityRanges {
    /// Returns serialized size.
    pub(super) fn size(&self) -> Result<usize> {
        let data_size = self.data_size();
        let len_size = serialized_size(&data_size)?;
        Ok(self.entity.len() + len_size + data_size)
    }

    pub(super) fn data_size(&self) -> usize {
        self.data.iter().map(|range| range.len()).sum()
    }

    pub(super) fn add_data(&mut self, data: Range<usize>) {
        if let Some(last) = self.data.last_mut() {
            // Append to previous range if possible.
            if last.end == data.start {
                last.end = data.end;
                return;
            }
        }

        self.data.push(data);
    }

    pub(super) fn extend(&mut self, other: &Self) {
        self.data.extend(other.data.iter().cloned());
    }
}
