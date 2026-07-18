use std::sync::atomic::{AtomicU64, Ordering};

/// Runtime selection behavior for a caller-number pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerPoolStrategy {
    /// Pseudo-random selection with equal probability for eligible members.
    Random,
    /// Pseudo-random selection according to member weights.
    WeightedRandom,
    /// Sequential selection shared by all concurrent calls.
    RoundRobin,
    /// Deterministic selection based on the call selection key.
    StableHash,
    /// Always select the first member after priority and number sorting.
    Priority,
}

impl CallerPoolStrategy {
    /// Parses the persisted strategy name, including legacy aliases.
    pub fn from_config(value: &str) -> Option<Self> {
        match value {
            "random" => Some(Self::Random),
            "weighted_random" | "weighted" => Some(Self::WeightedRandom),
            "round_robin" => Some(Self::RoundRobin),
            "stable_hash" | "hash" => Some(Self::StableHash),
            "priority" => Some(Self::Priority),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct PoolSelectionCursor {
    next: AtomicU64,
}

impl PoolSelectionCursor {
    pub(crate) fn select_index(
        &self,
        strategy: CallerPoolStrategy,
        pool_id: &str,
        selection_key: &str,
        weights: &[u32],
    ) -> Option<usize> {
        if weights.is_empty() {
            return None;
        }
        match strategy {
            CallerPoolStrategy::Priority => Some(0),
            CallerPoolStrategy::RoundRobin => {
                let sequence = self.next.fetch_add(1, Ordering::Relaxed);
                Some((sequence % weights.len() as u64) as usize)
            }
            CallerPoolStrategy::StableHash => {
                Some((stable_hash(selection_key) % weights.len() as u64) as usize)
            }
            CallerPoolStrategy::Random => {
                let value = self.next_random(pool_id);
                Some((value % weights.len() as u64) as usize)
            }
            CallerPoolStrategy::WeightedRandom => {
                let total_weight = weights.iter().try_fold(0_u64, |total, weight| {
                    total.checked_add(u64::from((*weight).max(1)))
                })?;
                weighted_index(weights, self.next_random(pool_id) % total_weight)
            }
        }
    }

    fn next_random(&self, pool_id: &str) -> u64 {
        let sequence = self.next.fetch_add(1, Ordering::Relaxed);
        splitmix64(sequence ^ stable_hash(pool_id))
    }
}

fn weighted_index(weights: &[u32], mut choice: u64) -> Option<usize> {
    for (index, weight) in weights.iter().enumerate() {
        let weight = u64::from((*weight).max(1));
        if choice < weight {
            return Some(index);
        }
        choice -= weight;
    }
    None
}

fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        hash.wrapping_mul(0x100000001b3) ^ u64::from(byte)
    })
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_robin_advances_without_locking() {
        let cursor = PoolSelectionCursor::default();
        let selected = (0..5)
            .map(|_| {
                cursor.select_index(CallerPoolStrategy::RoundRobin, "pool", "call", &[1, 1, 1])
            })
            .collect::<Vec<_>>();
        assert_eq!(selected, vec![Some(0), Some(1), Some(2), Some(0), Some(1)]);
    }

    #[test]
    fn stable_hash_is_repeatable_and_does_not_advance_cursor() {
        let cursor = PoolSelectionCursor::default();
        let first = cursor.select_index(
            CallerPoolStrategy::StableHash,
            "pool",
            "same-call",
            &[1, 1, 1],
        );
        for _ in 0..10 {
            assert_eq!(
                cursor.select_index(
                    CallerPoolStrategy::StableHash,
                    "pool",
                    "same-call",
                    &[1, 1, 1]
                ),
                first
            );
        }
        assert_eq!(
            cursor.select_index(CallerPoolStrategy::RoundRobin, "pool", "call", &[1, 1]),
            Some(0)
        );
    }

    #[test]
    fn weighted_random_never_selects_zero_weight_as_zero_is_normalized() {
        let cursor = PoolSelectionCursor::default();
        for _ in 0..100 {
            let selected =
                cursor.select_index(CallerPoolStrategy::WeightedRandom, "pool", "call", &[0, 2]);
            assert!(matches!(selected, Some(0 | 1)));
        }
    }

    #[test]
    fn concurrent_round_robin_distributes_each_member_equally() {
        let cursor = std::sync::Arc::new(PoolSelectionCursor::default());
        let handles = (0..100)
            .map(|_| {
                let cursor = std::sync::Arc::clone(&cursor);
                std::thread::spawn(move || {
                    cursor
                        .select_index(
                            CallerPoolStrategy::RoundRobin,
                            "pool",
                            "call",
                            &[1, 1, 1, 1],
                        )
                        .expect("non-empty pool must select a member")
                })
            })
            .collect::<Vec<_>>();
        let mut counts = [0_u32; 4];
        for handle in handles {
            let index = handle.join().expect("selection thread must finish");
            counts[index] += 1;
        }
        assert_eq!(counts, [25, 25, 25, 25]);
    }
}
