use bevy::prelude::*;

/// Set of entities stored as a bitset over their indices.
///
/// Generations are ignored, so an entity reads as present while another
/// entity with the same index is.
#[derive(Default, Debug)]
pub(crate) struct EntityBits {
    words: Vec<u64>,
    len: usize,
}

impl EntityBits {
    #[inline]
    pub(crate) fn insert(&mut self, entity: Entity) {
        let (word, bit) = locate(entity);
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        let word = &mut self.words[word];
        if *word & bit == 0 {
            *word |= bit;
            self.len += 1;
        }
    }

    #[inline]
    pub(crate) fn remove(&mut self, entity: Entity) {
        let (word, bit) = locate(entity);
        if let Some(word) = self.words.get_mut(word)
            && *word & bit != 0
        {
            *word &= !bit;
            self.len -= 1;
        }
    }

    #[inline]
    pub(crate) fn contains(&self, entity: Entity) -> bool {
        let (word, bit) = locate(entity);
        self.words.get(word).is_some_and(|word| word & bit != 0)
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        if !self.is_empty() {
            self.words.fill(0);
            self.len = 0;
        }
    }

    /// Adds all entities from `other`.
    #[inline]
    pub(crate) fn union_with(&mut self, other: &Self) {
        if other.is_empty() {
            return;
        }
        if self.words.len() < other.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (word, &other) in self.words.iter_mut().zip(&other.words) {
            *word |= other;
        }
        self.len = self
            .words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum();
    }
}

#[inline]
fn locate(entity: Entity) -> (usize, u64) {
    let index = entity.index_u32() as usize;
    (index / 64, 1 << (index % 64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_counts_shared_entities_once() {
        let first = Entity::from_raw_u32(3).unwrap();
        let second = Entity::from_raw_u32(130).unwrap();
        let mut bits = EntityBits::default();
        bits.insert(first);
        let mut other = EntityBits::default();
        other.insert(first);
        other.insert(second);

        bits.union_with(&other);
        assert!(bits.contains(first));
        assert!(bits.contains(second));
        assert_eq!(bits.len, 2);

        bits.remove(first);
        bits.remove(first);
        assert!(!bits.contains(first));
        assert_eq!(bits.len, 1);

        bits.clear();
        assert!(bits.is_empty());
        assert!(!bits.contains(second));
    }
}
