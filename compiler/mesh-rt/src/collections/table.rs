//! The storage of maps (`W = 2` words per entry: key, value) and sets
//! (`W = 1`: the element is the key).
//!
//! A *table* is `{len, capword, entries[cap], meta}`. Its first `len` entries
//! are its own, one per key, in insertion order; `capword` holds the key type
//! tag (maps: 0 Int, 1 String) above bit 56 and the capacity below bit 32.
//! `meta`, the word after the entries, points to the table's index or is 0.
//!
//! A table of up to [`SMALL`] entries is copied on every change, as a plain
//! array. A larger one grows in place: a change appends an entry to it (a
//! `put` of a key already there appends the new value, a delete appends a
//! tombstone) and returns a *view* `{n, sentinel, table, size, compact}`: the
//! table's first `n` entries, holding `size` live keys. Only the newest
//! version of a table appends (its `n` is the number of entries written);
//! any other version copies, so no value ever sees an entry added after it.
//! When a table is full, a change copies the live entries into a new table
//! with room for as many again. Memory stays proportional to the size, and
//! lookups, puts, adds and deletes take amortized O(1).
//!
//! The index (`meta`) holds the number of entries written, an open-addressing
//! table from each key's hash to its latest entry, and, per entry, a link to
//! the same key's previous entry and whether it is a tombstone. A lookup in
//! an older version follows the links back past the entries newer than it.
//! It holds no pointers, only positions.
//!
//! Every read of all the entries (iteration, display, equality, sending)
//! goes through [`compact`], which lays a view's live entries out in a table
//! of their own, once: the view keeps it.

use crate::gc::mesh_gc_alloc_actor;
use std::ptr;

/// Tables of up to this many entries are plain arrays, copied on a change.
pub(crate) const SMALL: usize = 8;

const TAG_SHIFT: u64 = 56;
const CAP_MASK: u64 = (1 << 32) - 1;
/// A link's "no previous entry"; positions are below it.
const NONE: u32 = 0x7FFF_FFFF;
/// A link's flag for an entry that deletes its key.
const TOMB: u32 = 0x8000_0000;
/// The largest capacity a link's position can address.
const MAX_CAP: usize = NONE as usize;

/// The word after a view's length, telling a map view (`W = 2`) or a set view
/// (`W = 1`) from a table (whose capword it would be) and from a list view.
pub(crate) const fn view_sentinel(width: usize) -> u64 {
    u64::MAX - width as u64
}
const VIEW_SIZE: u64 = 40;

pub(crate) type KeyEq = unsafe extern "C-unwind" fn(u64, u64) -> i8;
pub(crate) type KeyHash = unsafe extern "C-unwind" fn(u64) -> i64;

/// How keys compare and hash: as words (Int, Bool, Float), as strings, or by
/// the key type's Eq and Hash, which compiled code passes for other keys.
#[derive(Clone, Copy)]
pub(crate) struct Keys {
    pub string: bool,
    pub eq: Option<KeyEq>,
    pub hash: Option<KeyHash>,
}

impl Keys {
    pub(crate) const WORDS: Keys = Keys {
        string: false,
        eq: None,
        hash: None,
    };

    /// Keys compared by `key_eq` and hashed by `key_hash` (bare function
    /// pointers; null for none), else as the table's tag says.
    pub(crate) unsafe fn new(string: bool, key_eq: *mut u8, key_hash: *mut u8) -> Keys {
        Keys {
            string,
            eq: (!key_eq.is_null()).then(|| std::mem::transmute::<*mut u8, KeyEq>(key_eq)),
            hash: (!key_hash.is_null()).then(|| std::mem::transmute::<*mut u8, KeyHash>(key_hash)),
        }
    }

    pub(crate) unsafe fn equal(&self, a: u64, b: u64) -> bool {
        if let Some(eq) = self.eq {
            return eq(a, b) != 0;
        }
        if self.string {
            crate::string::mesh_string_eq(
                a as *const crate::string::MeshString,
                b as *const crate::string::MeshString,
            ) != 0
        } else {
            a == b
        }
    }

    /// Keys with an Eq and no hash to match it cannot be indexed: their
    /// tables stay arrays, copied on a change.
    fn indexable(&self) -> bool {
        self.eq.is_none() || self.hash.is_some()
    }

    unsafe fn hash(&self, key: u64) -> u64 {
        let raw = match (self.eq, self.hash) {
            (Some(_), Some(hash)) => hash(key) as u64,
            _ if self.string => {
                crate::hash::mesh_hash_string(key as *const crate::string::MeshString) as u64
            }
            _ => key,
        };
        mix(raw)
    }
}

/// splitmix64's finalizer: spreads keys that differ in a few bits.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// ── Tables ────────────────────────────────────────────────────────────

unsafe fn word(p: *const u8, index: usize) -> *mut u64 {
    (p as *mut u64).add(index)
}

pub(crate) unsafe fn len(table: *const u8) -> usize {
    *word(table, 0) as usize
}

pub(crate) unsafe fn tag(table: *const u8) -> u64 {
    *word(table, 1) >> TAG_SHIFT
}

pub(crate) unsafe fn cap(table: *const u8) -> usize {
    (*word(table, 1) & CAP_MASK) as usize
}

/// The `W` words of entry `index`.
pub(crate) unsafe fn entry<const W: usize>(table: *const u8, index: usize) -> *mut u64 {
    word(table, 2 + W * index)
}

unsafe fn key_at<const W: usize>(table: *const u8, index: usize) -> u64 {
    *entry::<W>(table, index)
}

unsafe fn meta_word<const W: usize>(table: *const u8) -> *mut u64 {
    word(table, 2 + W * cap(table))
}

/// A table with room for `cap` entries, holding none.
pub(crate) unsafe fn alloc_table<const W: usize>(cap: usize, tag: u64) -> *mut u8 {
    if cap > MAX_CAP {
        crate::panic::raise(format_args!("a map or set holds at most {MAX_CAP} entries"));
    }
    let size = 16 + 8 * (W * cap + 1);
    let table = mesh_gc_alloc_actor(size as u64, 8);
    *word(table, 0) = 0;
    *word(table, 1) = (tag << TAG_SHIFT) | cap as u64;
    *meta_word::<W>(table) = 0;
    table
}

/// A table holding `entries`, keys unique, in order.
pub(crate) unsafe fn table_from<const W: usize>(entries: &[[u64; W]], tag: u64) -> *mut u8 {
    let n = entries.len();
    let table = alloc_table::<W>(capacity_for(n), tag);
    ptr::copy_nonoverlapping(entries.as_ptr() as *const u64, entry::<W>(table, 0), W * n);
    *word(table, 0) = n as u64;
    table
}

/// Room for `n` entries: exactly, for a small table; twice that, for one
/// that grows in place.
fn capacity_for(n: usize) -> usize {
    if n <= SMALL {
        n
    } else {
        (2 * n).clamp(2 * SMALL, MAX_CAP).max(n)
    }
}

// ── Views ─────────────────────────────────────────────────────────────

pub(crate) unsafe fn is_view<const W: usize>(value: *const u8) -> bool {
    *word(value, 1) == view_sentinel(W)
}

/// The table behind a map or set value, how many of its entries the value
/// holds, and how many live keys that is.
pub(crate) unsafe fn state<const W: usize>(value: *const u8) -> (*mut u8, usize, usize) {
    if is_view::<W>(value) {
        (
            *word(value, 2) as *mut u8,
            *word(value, 0) as usize,
            *word(value, 3) as usize,
        )
    } else {
        (value as *mut u8, len(value), len(value))
    }
}

pub(crate) unsafe fn size<const W: usize>(value: *const u8) -> usize {
    state::<W>(value).2
}

unsafe fn new_view<const W: usize>(table: *mut u8, n: usize, size: usize) -> *mut u8 {
    let view = mesh_gc_alloc_actor(VIEW_SIZE, 8);
    *word(view, 0) = n as u64;
    *word(view, 1) = view_sentinel(W);
    *word(view, 2) = table as u64;
    *word(view, 3) = size as u64;
    *word(view, 4) = 0;
    view
}

// ── The index ─────────────────────────────────────────────────────────
//
// `{written: u64, slots: u64, index: [u32; slots], links: [u32; cap]}`: an
// index slot holds an entry's position + 1 (0 is empty), a link the
// position of the same key's previous entry (`NONE` for none) with `TOMB`
// set on a tombstone.

unsafe fn meta<const W: usize>(table: *const u8) -> *mut u8 {
    *meta_word::<W>(table) as *mut u8
}

/// The entries written in `table`: its own and those appended after them.
unsafe fn written<const W: usize>(table: *const u8) -> usize {
    let meta = meta::<W>(table);
    if meta.is_null() {
        len(table)
    } else {
        *word(meta, 0) as usize
    }
}

unsafe fn slots(meta: *const u8) -> usize {
    *word(meta, 1) as usize
}

unsafe fn index_slot(meta: *const u8, slot: usize) -> *mut u32 {
    (meta as *mut u32).add(4 + slot)
}

unsafe fn link(meta: *const u8, position: usize) -> *mut u32 {
    (meta as *mut u32).add(4 + slots(meta) + position)
}

unsafe fn previous(meta: *const u8, position: usize) -> Option<usize> {
    match *link(meta, position) & !TOMB {
        NONE => None,
        prev => Some(prev as usize),
    }
}

unsafe fn is_tombstone(meta: *const u8, position: usize) -> bool {
    *link(meta, position) & TOMB != 0
}

/// Index the `len` entries of a table that has none (they are its own, one
/// per key).
unsafe fn build_index<const W: usize>(table: *mut u8, keys: &Keys) -> *mut u8 {
    let cap = cap(table);
    let slots = (2 * cap).max(16).next_power_of_two();
    let bytes = 16 + 4 * (slots + cap);
    let meta = mesh_gc_alloc_actor(bytes.next_multiple_of(8) as u64, 8);
    *word(meta, 0) = len(table) as u64;
    *word(meta, 1) = slots as u64;
    ptr::write_bytes(index_slot(meta, 0), 0, slots);
    // Stored first: probing calls the key type's Eq, whose collection sees
    // the index only through the table.
    *meta_word::<W>(table) = meta as u64;
    for position in 0..len(table) {
        *link(meta, position) = NONE;
        let slot = match probe::<W>(table, meta, key_at::<W>(table, position), keys) {
            Ok(slot) | Err(slot) => slot,
        };
        *index_slot(meta, slot) = position as u32 + 1;
    }
    meta
}

/// The index slot of `key`'s latest entry (`Ok`), or the empty slot where it
/// would go (`Err`).
unsafe fn probe<const W: usize>(
    table: *const u8,
    meta: *const u8,
    key: u64,
    keys: &Keys,
) -> Result<usize, usize> {
    let mask = slots(meta) - 1;
    let mut slot = keys.hash(key) as usize & mask;
    loop {
        match *index_slot(meta, slot) {
            0 => return Err(slot),
            stored => {
                if keys.equal(key_at::<W>(table, stored as usize - 1), key) {
                    return Ok(slot);
                }
            }
        }
        slot = (slot + 1) & mask;
    }
}

// ── Operations ────────────────────────────────────────────────────────

/// The position of `key`'s live entry among the first `n` entries of
/// `table`.
pub(crate) unsafe fn find<const W: usize>(
    table: *mut u8,
    n: usize,
    key: u64,
    keys: &Keys,
) -> Option<usize> {
    let mut meta = meta::<W>(table);
    if meta.is_null() {
        // The table's own entries, one per key.
        if n <= SMALL || !keys.indexable() || !crate::gc::may_update_in_place(table) {
            return (0..n).find(|&i| keys.equal(key_at::<W>(table, i), key));
        }
        meta = build_index::<W>(table, keys);
    }
    if !keys.indexable() {
        // An Eq with no hash for an indexed table: the latest entry for the
        // key, scanning back.
        let position = (0..n)
            .rev()
            .find(|&i| keys.equal(key_at::<W>(table, i), key))?;
        return (!is_tombstone(meta, position)).then_some(position);
    }
    let slot = probe::<W>(table, meta, key, keys).ok()?;
    let mut position = *index_slot(meta, slot) as usize - 1;
    while position >= n {
        position = previous(meta, position)?;
    }
    (!is_tombstone(meta, position)).then_some(position)
}

/// Append an entry for `key` after the first `n` entries of `table`: `words`
/// (its `W` words), or a tombstone for `None`. Only when this is the newest
/// version of a table that grows in place and has room.
unsafe fn append<const W: usize>(
    table: *mut u8,
    n: usize,
    key: u64,
    words: Option<&[u64; W]>,
    keys: &Keys,
) -> bool {
    if cap(table) <= SMALL
        || !keys.indexable()
        || n != written::<W>(table)
        || n >= cap(table)
        || !crate::gc::may_update_in_place(table)
    {
        return false;
    }
    let mut meta = meta::<W>(table);
    if meta.is_null() {
        meta = build_index::<W>(table, keys);
    }
    let latest = match probe::<W>(table, meta, key, keys) {
        Ok(slot) => {
            let latest = *index_slot(meta, slot) - 1;
            *index_slot(meta, slot) = n as u32 + 1;
            latest
        }
        Err(slot) => {
            *index_slot(meta, slot) = n as u32 + 1;
            NONE
        }
    };
    let slot = entry::<W>(table, n);
    match words {
        Some(words) => {
            ptr::copy_nonoverlapping(words.as_ptr(), slot, W);
            *link(meta, n) = latest;
        }
        None => {
            ptr::write_bytes(slot, 0, W);
            *slot = key;
            *link(meta, n) = latest | TOMB;
        }
    }
    *word(meta, 0) = n as u64 + 1;
    true
}

/// `value` with `words` (an entry, its key first) added or, for a key it
/// holds, replacing that key's entry.
pub(crate) unsafe fn put<const W: usize>(value: *mut u8, words: [u64; W], keys: &Keys) -> *mut u8 {
    let (table, n, size) = state::<W>(value);
    let found = find::<W>(table, n, words[0], keys);
    let size = size + found.is_none() as usize;
    if append::<W>(table, n, words[0], Some(&words), keys) {
        return new_view::<W>(table, n + 1, size);
    }
    let mut entries = live_entries::<W>(value);
    match found {
        Some(position) => {
            // The key's entry in the table is at the same place among the
            // live ones as its key is.
            let at = entries
                .iter()
                .position(|e| keys.equal(e[0], *entry::<W>(table, position)))
                .unwrap_or(entries.len());
            if at < entries.len() {
                entries[at] = words;
            }
        }
        None => entries.push(words),
    }
    table_from::<W>(&entries, tag(table))
}

/// `value` without `key`: the value itself when it does not hold it.
pub(crate) unsafe fn delete<const W: usize>(value: *mut u8, key: u64, keys: &Keys) -> *mut u8 {
    let (table, n, size) = state::<W>(value);
    let Some(position) = find::<W>(table, n, key, keys) else {
        return value;
    };
    if append::<W>(table, n, key, None, keys) {
        return new_view::<W>(table, n + 1, size - 1);
    }
    let removed = *entry::<W>(table, position);
    let mut entries = live_entries::<W>(value);
    if let Some(at) = entries.iter().position(|e| keys.equal(e[0], removed)) {
        entries.remove(at);
    }
    table_from::<W>(&entries, tag(table))
}

/// The positions of the live entries among the first `n` of an indexed
/// table, in the order their keys were inserted: a `put` of a key that is
/// there keeps its place, one after a delete goes to the end.
unsafe fn live_positions(meta: *const u8, n: usize) -> Vec<u32> {
    let mut superseded = vec![false; n];
    // Where each entry's key was inserted, for the entry at a position.
    let mut inserted = vec![0u32; n];
    for position in 0..n {
        // A link points back; one that does not is not trusted.
        inserted[position] = match previous(meta, position).filter(|&prev| prev < position) {
            Some(prev) => {
                superseded[prev] = true;
                if is_tombstone(meta, prev) {
                    position as u32
                } else {
                    inserted[prev]
                }
            }
            None => position as u32,
        };
    }
    let mut by_insertion = vec![NONE; n];
    for position in 0..n {
        if !superseded[position] && !is_tombstone(meta, position) {
            by_insertion[inserted[position] as usize] = position as u32;
        }
    }
    by_insertion.retain(|&position| position != NONE);
    by_insertion
}

/// The live entries of a map or set value, in order.
pub(crate) unsafe fn live_entries<const W: usize>(value: *const u8) -> Vec<[u64; W]> {
    if is_view::<W>(value) {
        let cached = *word(value, 4) as *const u8;
        if !cached.is_null() {
            return live_entries::<W>(cached);
        }
    }
    let (table, n, _) = state::<W>(value);
    live_entries_in::<W>(table, n)
}

/// The live entries among the first `n` of `table`, in order.
pub(crate) unsafe fn live_entries_in<const W: usize>(table: *const u8, n: usize) -> Vec<[u64; W]> {
    let read = |position: usize| {
        let mut words = [0u64; W];
        ptr::copy_nonoverlapping(entry::<W>(table, position), words.as_mut_ptr(), W);
        words
    };
    let meta = meta::<W>(table);
    if meta.is_null() {
        return (0..n).map(read).collect();
    }
    live_positions(meta, n)
        .into_iter()
        .map(|position| read(position as usize))
        .collect()
}

/// For readers that check memory before trusting it (message capture): the
/// bytes a table with room for `cap` entries takes, the address of its index
/// (0 for none), and the bytes an index with `slots` slots takes.
pub(crate) const fn table_bytes<const W: usize>(cap: usize) -> usize {
    16 + 8 * (W * cap + 1)
}

pub(crate) unsafe fn index_address<const W: usize>(table: *const u8) -> usize {
    *meta_word::<W>(table) as usize
}

pub(crate) const fn index_bytes(slots: usize, cap: usize) -> usize {
    16 + 4 * (slots + cap)
}

/// A value's live entries as a table of their own: the value itself unless
/// it is a view, whose compact table is built once and kept in it.
pub(crate) unsafe fn compact<const W: usize>(value: *mut u8) -> *mut u8 {
    if !is_view::<W>(value) {
        return value;
    }
    let cached = *word(value, 4) as *mut u8;
    if !cached.is_null() {
        return cached;
    }
    let table = state::<W>(value).0;
    let compacted = table_from::<W>(&live_entries::<W>(value), tag(table));
    if crate::gc::may_update_in_place(value) {
        *word(value, 4) = compacted as u64;
    }
    compacted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::mesh_rt_init;

    /// A reference model of one version: its entries in insertion order.
    type Model = Vec<(u64, u64)>;

    fn model_put(model: &Model, key: u64, value: u64) -> Model {
        let mut next = model.clone();
        match next.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => next.push((key, value)),
        }
        next
    }

    fn model_delete(model: &Model, key: u64) -> Model {
        model.iter().copied().filter(|(k, _)| *k != key).collect()
    }

    unsafe fn check(value: *mut u8, model: &Model) {
        assert_eq!(size::<2>(value), model.len());
        let entries: Vec<(u64, u64)> = live_entries::<2>(value)
            .into_iter()
            .map(|[k, v]| (k, v))
            .collect();
        assert_eq!(&entries, model);
        let (table, n, _) = state::<2>(value);
        for key in 0..40u64 {
            let found =
                find::<2>(table, n, key, &Keys::WORDS).map(|p| *entry::<2>(table, p).add(1));
            let expected = model.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
            assert_eq!(found, expected, "key {key}");
        }
        let compacted = compact::<2>(value);
        assert_eq!(live_entries::<2>(compacted).len(), model.len());
    }

    #[test]
    fn versions_keep_their_entries_through_puts_and_deletes() {
        // Random puts and deletes on random earlier versions, each checked
        // against a model of that version, after all of them.
        mesh_rt_init();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        unsafe {
            let empty = table_from::<2>(&[], 0);
            let mut versions: Vec<(*mut u8, Model)> = vec![(empty, Vec::new())];
            for step in 0..4000 {
                // Mostly the newest version (in place), sometimes an older one.
                let pick = if next(5) == 0 {
                    next(versions.len() as u64) as usize
                } else {
                    versions.len() - 1
                };
                let (value, model) = versions[pick].clone();
                let key = next(40);
                let (value, model) = if next(4) == 0 {
                    (
                        delete::<2>(value, key, &Keys::WORDS),
                        model_delete(&model, key),
                    )
                } else {
                    (
                        put::<2>(value, [key, step], &Keys::WORDS),
                        model_put(&model, key, step),
                    )
                };
                check(value, &model);
                versions.push((value, model));
            }
            for (value, model) in &versions {
                check(*value, model);
            }
        }
    }

    #[test]
    fn a_table_grows_in_place_and_compacts_when_full() {
        mesh_rt_init();
        unsafe {
            let mut value = table_from::<2>(&[], 0);
            for i in 0..1000u64 {
                value = put::<2>(value, [i % 50, i], &Keys::WORDS);
            }
            assert_eq!(size::<2>(value), 50);
            let (table, _, _) = state::<2>(value);
            // Updates of 50 keys stay in a table sized for them.
            assert!(cap(table) <= 4 * 50, "{}", cap(table));
            let entries = live_entries::<2>(value);
            assert_eq!(entries[0], [0, 950]);
            assert_eq!(entries[49], [49, 999]);
        }
    }
}
