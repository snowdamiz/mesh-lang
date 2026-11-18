use crossbeam_utils::CachePadded;
use parking_lot::{Mutex, MutexGuard};

const SHARDS: usize = 128;
const NORMAL_LIMIT: Quota = Quota {
    items: 65_536,
    bytes: 128 * 1024 * 1024,
};
const CONTROL_LIMIT: Quota = Quota {
    items: NORMAL_LIMIT.items + 16_384,
    bytes: NORMAL_LIMIT.bytes + 8 * 1024 * 1024,
};

#[derive(Clone, Copy, Debug, Default)]
struct Quota {
    items: usize,
    bytes: usize,
}

impl Quota {
    fn admits(self, used: Self, bytes: usize) -> bool {
        used.items < self.items && used.bytes <= self.bytes && bytes <= self.bytes - used.bytes
    }
}

#[derive(Debug)]
pub(super) struct QuotaShard {
    used: Quota,
    limit: Quota,
}

impl QuotaShard {
    pub(super) fn release(&mut self, items: usize, bytes: usize) {
        self.used.items -= items;
        self.used.bytes -= bytes;
    }
}

/// Live mailbox accounting and admission budgets. Mailboxes keep their shard
/// for their lifetime, including scheduler migrations and cross-thread drops.
/// The lock order is mailbox state, then quota shards in ascending order.
#[derive(Debug)]
pub(crate) struct MailboxUsage {
    shards: [CachePadded<Mutex<QuotaShard>>; SHARDS],
    normal_limit: Quota,
    control_limit: Quota,
}

impl Default for MailboxUsage {
    fn default() -> Self {
        Self::new(NORMAL_LIMIT, CONTROL_LIMIT)
    }
}

impl MailboxUsage {
    fn new(normal_limit: Quota, control_limit: Quota) -> Self {
        Self {
            shards: std::array::from_fn(|shard| {
                CachePadded::new(Mutex::new(QuotaShard {
                    used: Quota::default(),
                    limit: Quota {
                        items: normal_limit.items / SHARDS
                            + usize::from(shard < normal_limit.items % SHARDS),
                        bytes: normal_limit.bytes / SHARDS
                            + usize::from(shard < normal_limit.bytes % SHARDS),
                    },
                }))
            }),
            normal_limit,
            control_limit,
        }
    }

    pub(super) fn shard_for(local_pid: u64) -> usize {
        local_pid as usize % SHARDS
    }

    #[inline]
    pub(super) fn lock(&self, shard: usize) -> MutexGuard<'_, QuotaShard> {
        self.shards[shard].lock()
    }

    // The caller keeps the guard through the queue mutation so a gauge read
    // holding every shard sees a single point in time.
    #[inline]
    pub(super) fn reserve(
        &self,
        index: usize,
        bytes: usize,
        control: bool,
    ) -> Option<MutexGuard<'_, QuotaShard>> {
        let mut shard = self.lock(index);
        if shard.limit.admits(shard.used, bytes) {
            shard.used.items += 1;
            shard.used.bytes += bytes;
            return Some(shard);
        }
        drop(shard);
        self.reserve_slow(index, bytes, control)
    }

    #[cold]
    fn reserve_slow(
        &self,
        index: usize,
        bytes: usize,
        control: bool,
    ) -> Option<MutexGuard<'_, QuotaShard>> {
        let mut shards = self.lock_all();
        let mut total = Self::total(&shards);
        let limit = if control {
            self.control_limit
        } else {
            self.normal_limit
        };
        if !limit.admits(total, bytes) {
            return None;
        }

        total.items += 1;
        total.bytes += bytes;
        shards[index].used.items += 1;
        shards[index].used.bytes += bytes;

        // Within the normal cap, every limit covers its shard's actual usage
        // and their sum is the normal cap. Give the requesting shard the
        // balance after leaving a share of spare capacity on the other shards.
        // In control headroom, close all fast paths until exact admission can
        // establish that usage fits the normal cap again.
        if total.items <= self.normal_limit.items && total.bytes <= self.normal_limit.bytes {
            let free_items = self.normal_limit.items - total.items;
            let free_bytes = self.normal_limit.bytes - total.bytes;
            let share_items = free_items / (2 * SHARDS);
            let share_bytes = free_bytes / (2 * SHARDS);
            for shard in &mut shards {
                shard.limit = Quota {
                    items: shard.used.items + share_items,
                    bytes: shard.used.bytes + share_bytes,
                };
            }
            shards[index].limit.items += free_items - share_items * SHARDS;
            shards[index].limit.bytes += free_bytes - share_bytes * SHARDS;
        } else {
            for shard in &mut shards {
                shard.limit = Quota::default();
            }
        }
        shards.into_iter().nth(index)
    }

    fn lock_all(&self) -> [MutexGuard<'_, QuotaShard>; SHARDS] {
        std::array::from_fn(|index| self.lock(index))
    }

    fn total(shards: &[MutexGuard<'_, QuotaShard>; SHARDS]) -> Quota {
        let mut total = Quota::default();
        for shard in shards {
            total.items += shard.used.items;
            total.bytes += shard.used.bytes;
        }
        total
    }

    pub(crate) fn messages(&self) -> u64 {
        Self::total(&self.lock_all()).items as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    fn limits(
        items: usize,
        bytes: usize,
        control_items: usize,
        control_bytes: usize,
    ) -> MailboxUsage {
        MailboxUsage::new(
            Quota { items, bytes },
            Quota {
                items: items + control_items,
                bytes: bytes + control_bytes,
            },
        )
    }

    #[test]
    fn a_shard_can_use_capacity_assigned_to_other_shards() {
        let usage = limits(2, 7, 0, 0);
        assert!(usage.reserve(SHARDS - 1, 7, false).is_some());
        assert!(usage.reserve(0, 1, false).is_none());
        assert!(usage.reserve(0, 0, false).is_some());
        assert!(usage.reserve(1, 0, false).is_none());
        assert_eq!(usage.messages(), 2);

        usage.lock(SHARDS - 1).release(1, 7);
        assert!(usage.reserve(1, usize::MAX, false).is_none());
        assert!(usage.reserve(1, 7, false).is_some());
        usage.lock(0).release(1, 0);
        usage.lock(1).release(1, 7);
        assert_eq!(usage.messages(), 0);
    }

    #[test]
    fn control_headroom_blocks_normal_sends_until_both_caps_allow_them() {
        let usage = limits(3, 7, 1, 3);
        assert!(usage.reserve(0, 6, false).is_some());
        assert!(usage.reserve(1, 1, false).is_some());
        assert!(usage.reserve(2, 3, true).is_some());
        assert!(usage.reserve(3, 1, true).is_none());
        assert!(usage.reserve(3, 0, true).is_some());
        assert!(usage.reserve(4, 0, true).is_none());

        usage.lock(3).release(1, 0);
        usage.lock(1).release(1, 1);
        // One item slot is free even though byte usage remains above the cap.
        assert!(usage.reserve(4, 0, false).is_none());
        usage.lock(2).release(1, 3);
        assert!(usage.reserve(4, 1, false).is_some());
        assert!(usage.reserve(3, 0, false).is_some());
        assert!(usage.reserve(5, 0, false).is_none());
        assert_eq!(usage.messages(), 3);
    }

    #[test]
    fn concurrent_fast_and_slow_admission_share_one_byte_limit() {
        let byte_limit = 2 * SHARDS;
        let large_message = byte_limit - 6;
        let usage = limits(SHARDS, byte_limit, 0, 0);
        let start = Barrier::new(8);
        let accepted = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|index| {
                    let usage = &usage;
                    let start = &start;
                    let bytes = if index == 0 { large_message } else { 2 };
                    scope.spawn(move || {
                        start.wait();
                        usage
                            .reserve(index, bytes, false)
                            .is_some()
                            .then_some((index, bytes))
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let large_accepted = accepted.iter().any(|&(index, _)| index == 0);
        assert_eq!(accepted.len(), if large_accepted { 4 } else { 7 });
        assert_eq!(usage.messages(), accepted.len() as u64);
        for (index, bytes) in accepted {
            usage.lock(index).release(1, bytes);
        }
        assert_eq!(usage.messages(), 0);
        assert!(usage.reserve(SHARDS - 1, byte_limit, false).is_some());
    }
}
