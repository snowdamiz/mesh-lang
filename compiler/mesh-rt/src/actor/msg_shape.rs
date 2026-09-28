//! Shape-guided deep copy of values that cross actors.
//!
//! Each actor's collector sees only its own heap, so a message must not carry
//! pointers into the sender's. The compiler describes where the references
//! sit inside a value -- its *shape*, built from the full static type -- and
//! this module copies whatever they reach out of the sender's heap at send
//! time ([`capture`]) and rebuilds it in the receiver's heap at receive time
//! ([`Captured::materialize`]).
//!
//! A wrong shape cannot corrupt memory. Objects are copied verbatim by their
//! allocation size, a slot is followed only when it holds the exact start of a
//! live object in the sender's heap, and every offset is bounds-checked. The
//! worst a mistake can do is leave a value shared, which is what happened to
//! every value before this existed. References the shape cannot describe
//! (closure environments, opaque runtime objects, anything outside the
//! sender's heap) are reported in [`Captured::lend`] so the owning heap can
//! keep them alive for the receiver instead.
//!
//! ## Shape tables
//!
//! A shape is a table of `u32` words emitted by the compiler as a constant.
//! Word 0 is the table's length in words and the root node starts at word 1.
//! A node is its kind followed by operands; operands naming other nodes are
//! word indexes into the same table, so recursive types are simply cycles.
//! Must stay in sync with `mesh-codegen/src/codegen/msg_shape.rs`.
//!
//! | kind | operands | the described slot or bytes |
//! |---|---|---|
//! | `SCALAR` | | plain bits |
//! | `LEAF` | | pointer to an object holding no references |
//! | `LIST` | elem | pointer to `{len, cap, [slot]}` (lists and sets) |
//! | `MAP` | key, value | pointer to `{len, cap, [(slot, slot)]}` |
//! | `TUPLE` | n, elem × n | pointer to `{len, [slot]}` |
//! | `BOXED` | inner | pointer to an object holding `inner` by value |
//! | `AGG` | n, (offset, node) × n | by-value aggregate |
//! | `SUM` | n, (tag, fields, (offset, node) × fields) × n | by-value tagged union, tag byte first |
//! | `JSON` | | pointer to a `MeshJson` tree |
//! | `QUEUE` | elem | pointer to `{buffer list, head, tail}` |
//! | `SHARED` | | a reference that cannot be copied by type |
//! | `CLOSURE` | | by-value `{fn, env}`; `env` points to an environment |
//! | `STRING` | | a `LEAF` that is a string `{len, bytes}`, perhaps a literal |
//! | `PID` | | a pid, which is plain bits on this node |
//!
//! A closure's type says nothing about what it captured, so an environment
//! describes itself: its first word points at the shape table the compiler
//! emitted for it (root: the environment by value), or is null when it holds
//! no references. An environment that is not an object of the sender's heap,
//! such as one the runtime made, is lent like any `SHARED` reference.
//!
//! ## Messages for another node
//!
//! [`capture_for_node`] copies what a message for another node references:
//! nothing can be lent across a network. It takes a string wherever it lives
//! (a literal in the program's constant data too, which another program
//! cannot read) and notes where the message's pids are, which only mean
//! something on the node that made them (see `dist::node`). A message that
//! references code or a runtime object cannot leave its node at all.
//! [`Captured::encode`] and [`Captured::decode`] carry the result.

use rustc_hash::FxHashMap;

use super::heap::ActorHeap;

pub(crate) const SCALAR: u32 = 0;
pub(crate) const LEAF: u32 = 1;
pub(crate) const LIST: u32 = 2;
pub(crate) const MAP: u32 = 3;
pub(crate) const TUPLE: u32 = 4;
pub(crate) const BOXED: u32 = 5;
pub(crate) const AGG: u32 = 6;
pub(crate) const SUM: u32 = 7;
pub(crate) const JSON: u32 = 8;
pub(crate) const QUEUE: u32 = 9;
pub(crate) const SHARED: u32 = 10;
pub(crate) const CLOSURE: u32 = 11;
pub(crate) const STRING: u32 = 12;
pub(crate) const PID: u32 = 13;

// Nodes the runtime supplies itself, for the self-describing JSON tree. They
// sit above any real table index.
const JSON_NODE: u32 = u32::MAX;
const JSON_ARRAY_NODE: u32 = u32::MAX - 1;
const JSON_OBJECT_NODE: u32 = u32::MAX - 2;
const LEAF_NODE: u32 = u32::MAX - 3;
/// A closure environment, which names its own table.
const ENV_NODE: u32 = u32::MAX - 4;
const STRING_NODE: u32 = u32::MAX - 5;

/// A shape table: `words[0]` is its length in words.
#[derive(Clone, Copy)]
struct Table {
    words: *const u32,
    len: usize,
}

impl Table {
    /// Longer than any table a type could need; a sanity bound on a length
    /// that, for an environment, is read through a pointer found in the heap.
    const MAX_WORDS: usize = 1 << 20;

    /// # Safety
    ///
    /// `words` must be null or point to a shape table.
    unsafe fn at(words: *const u32) -> Option<Table> {
        if words.is_null() || !words.is_aligned() {
            return None;
        }
        let len = *words as usize;
        (2..=Self::MAX_WORDS)
            .contains(&len)
            .then_some(Table { words, len })
    }

    fn get(&self, index: usize) -> Option<u32> {
        (index < self.len).then(|| unsafe { *self.words.add(index) })
    }
}

const JSON_TAG_STR: u8 = 3;
const JSON_TAG_ARRAY: u8 = 4;
const JSON_TAG_OBJECT: u8 = 5;

/// One object taken out of the sender's heap.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OwnedObject {
    pub(crate) bytes: Vec<u8>,
    /// `(offset in bytes, index of the object that slot points to)`.
    pub(crate) relocs: Vec<(usize, u32)>,
}

/// Everything a message references, detached from the sender's heap.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Captured {
    pub(crate) objects: Vec<OwnedObject>,
    /// Relocations for the message's own bytes.
    pub(crate) relocs: Vec<(usize, u32)>,
    /// References that were left in place and must be kept alive by whoever
    /// owns them; see `scheduler::lend_words`.
    pub(crate) lend: Vec<usize>,
    /// Where the message's pids are: in its own bytes (`None`) or in an
    /// object, at a byte offset. Only a capture for another node notes them.
    pub(crate) pids: Vec<(Option<u32>, usize)>,
}

impl Captured {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.lend.is_empty()
    }

    /// Replace each pid of the message whose own bytes are `data` with what
    /// `f` makes of it.
    pub(crate) fn map_pids(&mut self, data: &mut [u8], mut f: impl FnMut(u64) -> u64) {
        for &(object, offset) in &self.pids {
            let bytes = match object {
                None => &mut *data,
                Some(index) => &mut self.objects[index as usize].bytes,
            };
            if let Some(word) = bytes.get_mut(offset..offset + 8) {
                let pid = f(u64::from_le_bytes(word.try_into().unwrap()));
                word.copy_from_slice(&pid.to_le_bytes());
            }
        }
    }

    /// Append this capture of a message for another node, the node each of
    /// its pids is on in `pid_nodes`: its relocations, then its objects with
    /// theirs, then its pids, each count a `u32` and everything little-endian.
    pub(crate) fn encode(&self, out: &mut Vec<u8>, pid_nodes: &[String]) {
        fn relocs(out: &mut Vec<u8>, relocs: &[(usize, u32)]) {
            out.extend_from_slice(&(relocs.len() as u32).to_le_bytes());
            for &(offset, target) in relocs {
                out.extend_from_slice(&(offset as u64).to_le_bytes());
                out.extend_from_slice(&target.to_le_bytes());
            }
        }
        relocs(out, &self.relocs);
        out.extend_from_slice(&(self.objects.len() as u32).to_le_bytes());
        for object in &self.objects {
            out.extend_from_slice(&(object.bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(&object.bytes);
            relocs(out, &object.relocs);
        }
        out.extend_from_slice(&(self.pids.len() as u32).to_le_bytes());
        for (&(object, offset), node) in self.pids.iter().zip(pid_nodes) {
            out.extend_from_slice(&object.map_or(0, |index| index + 1).to_le_bytes());
            out.extend_from_slice(&(offset as u64).to_le_bytes());
            out.extend_from_slice(&(node.len() as u16).to_le_bytes());
            out.extend_from_slice(node.as_bytes());
        }
    }

    /// A capture `encode` wrote, for a message of `data_len` bytes, with the
    /// node of each pid. `None` unless every count, offset and index in it
    /// stays inside what it describes: it comes from another process.
    pub(crate) fn decode(bytes: &[u8], data_len: usize) -> Option<(Captured, Vec<String>)> {
        let mut input = WireReader { bytes, pos: 0 };
        let relocs = |input: &mut WireReader| -> Option<Vec<(usize, u32)>> {
            (0..input.u32()?)
                .map(|_| Some((input.u64()? as usize, input.u32()?)))
                .collect()
        };
        let mut captured = Captured {
            relocs: relocs(&mut input)?,
            ..Captured::default()
        };
        for _ in 0..input.u32()? {
            let len = input.u64()? as usize;
            let bytes = input.take(len)?.to_vec();
            captured.objects.push(OwnedObject {
                bytes,
                relocs: relocs(&mut input)?,
            });
        }
        let mut nodes = Vec::new();
        for _ in 0..input.u32()? {
            let object = input.u32()?.checked_sub(1);
            captured.pids.push((object, input.u64()? as usize));
            let len = input.u16()? as usize;
            nodes.push(std::str::from_utf8(input.take(len)?).ok()?.to_string());
        }
        let objects = &captured.objects;
        let fits = |len: usize, &(offset, target): &(usize, u32)| {
            offset.checked_add(8).is_some_and(|end| end <= len) && (target as usize) < objects.len()
        };
        let valid = input.pos == bytes.len()
            && captured.relocs.iter().all(|reloc| fits(data_len, reloc))
            && objects.iter().all(|object| {
                object
                    .relocs
                    .iter()
                    .all(|reloc| fits(object.bytes.len(), reloc))
            })
            && captured.pids.iter().all(|&(object, offset)| {
                let len = match object {
                    None => Some(data_len),
                    Some(index) => objects.get(index as usize).map(|object| object.bytes.len()),
                };
                len.is_some_and(|len| offset.checked_add(8).is_some_and(|end| end <= len))
            });
        valid.then_some((captured, nodes))
    }

    /// Rebuild the captured objects in `heap` and point `data` at them.
    ///
    /// # Safety
    ///
    /// `data` must be the receiver's writable copy of the bytes that were
    /// passed to [`capture`].
    pub(crate) unsafe fn materialize(&self, heap: &mut ActorHeap, data: *mut u8) {
        let placed: Vec<*mut u8> = self
            .objects
            .iter()
            .map(|object| {
                let copy = heap.alloc(object.bytes.len(), 8);
                std::ptr::copy_nonoverlapping(object.bytes.as_ptr(), copy, object.bytes.len());
                copy
            })
            .collect();
        for (object, &copy) in self.objects.iter().zip(&placed) {
            for &(offset, target) in &object.relocs {
                (copy.add(offset) as *mut u64).write_unaligned(placed[target as usize] as u64);
            }
        }
        for &(offset, target) in &self.relocs {
            (data.add(offset) as *mut u64).write_unaligned(placed[target as usize] as u64);
        }
    }
}

/// Capture everything the value at `data[base..]` references through `shape`
/// out of `heap`.
///
/// # Safety
///
/// `shape` must be null or point to a shape table as described in the module
/// docs. `heap` must be the heap of the actor that built `data`, and that
/// actor must not be running elsewhere.
pub(crate) unsafe fn capture(
    heap: &ActorHeap,
    data: &[u8],
    base: usize,
    shape: *const u32,
) -> Captured {
    capture_as(heap, data, base, shape, false)
}

/// [`capture`] for a message to another node (see the module docs): `None`
/// when it references something that cannot leave this one.
///
/// # Safety
///
/// As for [`capture`].
pub(crate) unsafe fn capture_for_node(
    heap: &ActorHeap,
    data: &[u8],
    shape: *const u32,
) -> Option<Captured> {
    let captured = capture_as(heap, data, 0, shape, true);
    captured.lend.is_empty().then_some(captured)
}

unsafe fn capture_as(
    heap: &ActorHeap,
    data: &[u8],
    base: usize,
    shape: *const u32,
    for_node: bool,
) -> Captured {
    // A null shape is a message of plain bits, which callers do not capture.
    let table = Table::at(shape).expect("the compiler emits a whole shape table");
    let mut capture = Capture {
        heap,
        table,
        data,
        out: Captured::default(),
        seen: FxHashMap::default(),
        pending: Vec::new(),
        for_node,
    };
    capture.value(None, 1, base);
    // Pointer chains are walked from this stack, not by recursion: a
    // million-cell structure must not overflow a 512 KiB actor stack.
    while let Some((object, table, node)) = capture.pending.pop() {
        capture.table = table;
        capture.body(object, node);
    }
    capture.out
}

struct Capture<'a> {
    heap: &'a ActorHeap,
    /// The table the node being visited belongs to: the message's, or a
    /// closure environment's own.
    table: Table,
    data: &'a [u8],
    out: Captured,
    /// Sender address -> index in `out.objects`, so shared structure stays shared.
    seen: FxHashMap<usize, u32>,
    /// Captured objects whose insides still have to be walked, each with the
    /// table its node is in.
    pending: Vec<(u32, Table, u32)>,
    /// For another node: nothing is lent, and pids are noted.
    for_node: bool,
}

/// Reads the little-endian words of a capture from another process.
struct WireReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> WireReader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let taken = self.bytes.get(self.pos..self.pos.checked_add(len)?)?;
        self.pos += len;
        Some(taken)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

impl Capture<'_> {
    fn word(&self, index: u32) -> Option<u32> {
        self.table.get(index as usize)
    }

    fn kind(&self, node: u32) -> u32 {
        match node {
            JSON_NODE => JSON,
            JSON_ARRAY_NODE => LIST,
            JSON_OBJECT_NODE => MAP,
            LEAF_NODE => LEAF,
            STRING_NODE => STRING,
            _ => self.word(node).unwrap_or(SCALAR),
        }
    }

    /// The `n`th node operand of `node`.
    fn operand(&self, node: u32, n: u32) -> u32 {
        match (node, n) {
            (JSON_ARRAY_NODE, _) | (JSON_OBJECT_NODE, 1) => JSON_NODE,
            (JSON_OBJECT_NODE, _) => STRING_NODE,
            _ => self.word(node + 1 + n).unwrap_or(0),
        }
    }

    fn bytes(&self, container: Option<u32>) -> &[u8] {
        match container {
            Some(object) => &self.out.objects[object as usize].bytes,
            None => self.data,
        }
    }

    fn read_word(&self, container: Option<u32>, offset: usize) -> Option<usize> {
        let bytes = self.bytes(container).get(offset..offset.checked_add(8)?)?;
        Some(u64::from_ne_bytes(bytes.try_into().unwrap()) as usize)
    }

    /// Visit the value described by `node` at `offset` inside `container`.
    ///
    /// Recursion here only follows by-value nesting, which a static type
    /// bounds; everything behind a pointer goes through `pending`.
    fn value(&mut self, container: Option<u32>, node: u32, offset: usize) {
        match self.kind(node) {
            AGG => {
                let fields = self.word(node + 1).unwrap_or(0);
                for field in 0..fields {
                    let at = node + 2 + 2 * field;
                    let (Some(field_offset), Some(field_node)) = (self.word(at), self.word(at + 1))
                    else {
                        return;
                    };
                    self.value(container, field_node, offset + field_offset as usize);
                }
            }
            SUM => self.sum(container, node, offset),
            SCALAR => {}
            PID => {
                if self.for_node {
                    self.out.pids.push((container, offset));
                }
            }
            // Code means nothing in another program.
            CLOSURE if self.for_node => self.out.lend.push(offset),
            CLOSURE => {
                // `{fn, env}` by value. Code is not data; the environment is
                // an object, unless the closure is a plain function (null).
                let Some(env) = self.read_word(container, offset + 8) else {
                    return;
                };
                if env == 0 {
                    return;
                }
                match self.object(env, ENV_NODE) {
                    Some(index) => self.relocs(container).push((offset + 8, index)),
                    None => self.out.lend.push(env),
                }
            }
            kind => {
                let Some(word) = self.read_word(container, offset) else {
                    return;
                };
                if word == 0 {
                    return;
                }
                match (kind != SHARED).then(|| self.object(word, node)).flatten() {
                    Some(index) => self.relocs(container).push((offset, index)),
                    None => self.out.lend.push(word),
                }
            }
        }
    }

    fn sum(&mut self, container: Option<u32>, node: u32, offset: usize) {
        let Some(&tag) = self.bytes(container).get(offset) else {
            return;
        };
        let variants = self.word(node + 1).unwrap_or(0);
        let mut at = node + 2;
        for _ in 0..variants {
            let (Some(variant_tag), Some(fields)) = (self.word(at), self.word(at + 1)) else {
                return;
            };
            if variant_tag == tag as u32 {
                for field in 0..fields {
                    let field_at = at + 2 + 2 * field;
                    let (Some(field_offset), Some(field_node)) =
                        (self.word(field_at), self.word(field_at + 1))
                    else {
                        return;
                    };
                    self.value(container, field_node, offset + field_offset as usize);
                }
                return;
            }
            at += 2 + 2 * fields;
        }
    }

    fn relocs(&mut self, container: Option<u32>) -> &mut Vec<(usize, u32)> {
        match container {
            Some(object) => &mut self.out.objects[object as usize].relocs,
            None => &mut self.out.relocs,
        }
    }

    /// Take the object at `address` if it is ours to take.
    fn object(&mut self, address: usize, node: u32) -> Option<u32> {
        if let Some(&index) = self.seen.get(&address) {
            return Some(index);
        }
        // Static literals, the global arena and other actors' heaps all fail
        // this test and stay where they are, unless the copy is for another
        // node: a string says how long it is.
        let size = match self.heap.live_allocation_size(address as *const u8) {
            Some(size) => size,
            None if self.for_node && self.kind(node) == STRING => {
                8 + unsafe { (address as *const u64).read_unaligned() } as usize
            }
            None => return None,
        };
        let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, size) }.to_vec();
        let index = self.out.objects.len() as u32;
        self.out.objects.push(OwnedObject {
            bytes,
            relocs: Vec::new(),
        });
        self.seen.insert(address, index);
        if !matches!(self.kind(node), LEAF | STRING) {
            self.pending.push((index, self.table, node));
        }
        Some(index)
    }

    /// Walk the insides of a captured object.
    fn body(&mut self, object: u32, node: u32) {
        let container = Some(object);
        if node == ENV_NODE {
            // The environment's first word is its table; its root describes
            // the environment by value. The rest of the walk is in that table.
            let table = self.read_word(container, 0).unwrap_or(0) as *const u32;
            if let Some(table) = unsafe { Table::at(table) } {
                self.table = table;
                self.value(container, 1, 0);
            }
            return;
        }
        // A view left as it is holds no entries of its own to walk.
        let holds_entries = match self.kind(node) {
            LIST => self.flatten_list_view(object),
            MAP => self.flatten_table::<2>(object),
            _ => true,
        };
        if !holds_entries {
            return;
        }
        let size = self.out.objects[object as usize].bytes.len();
        // `{len, ...}` headers are trusted only as far as the allocation goes.
        let count = |header: usize, stride: usize| {
            let len = self.read_word(container, 0).unwrap_or(0);
            len.min(size.saturating_sub(header) / stride)
        };
        match self.kind(node) {
            LIST => {
                let elem = self.operand(node, 0);
                if self.kind(elem) != SCALAR {
                    for index in 0..count(16, 8) {
                        self.value(container, elem, 16 + 8 * index);
                    }
                }
            }
            MAP => {
                let (key, value) = (self.operand(node, 0), self.operand(node, 1));
                for index in 0..count(16, 16) {
                    self.value(container, key, 16 + 16 * index);
                    self.value(container, value, 24 + 16 * index);
                }
            }
            TUPLE => {
                let elems = self.operand(node, 0) as usize;
                for index in 0..count(8, 8).min(elems) {
                    let elem = self.operand(node, 1 + index as u32);
                    self.value(container, elem, 8 + 8 * index);
                }
            }
            BOXED => self.value(container, self.operand(node, 0), 0),
            QUEUE => {
                // A queue is a buffer list and two indices into it.
                let Some(word) = self.read_word(container, 0) else {
                    return;
                };
                if let Some(index) = self.list_of(word, self.operand(node, 0)) {
                    self.relocs(container).push((0, index));
                }
            }
            JSON => {
                // A `Json` may be the JSON text of a tree, a string copied
                // whole, whose bytes are not a tree's to follow.
                let bytes = &self.out.objects[object as usize].bytes;
                if bytes.get(1..8) != Some(&crate::json::TREE_MARK[..]) {
                    return;
                }
                let inner = match bytes.first().copied() {
                    Some(JSON_TAG_STR) => STRING_NODE,
                    Some(JSON_TAG_ARRAY) => JSON_ARRAY_NODE,
                    Some(JSON_TAG_OBJECT) => JSON_OBJECT_NODE,
                    _ => return,
                };
                self.value(container, inner, 8);
            }
            _ => {}
        }
    }

    /// A list view (`{len, VIEW, parent, offset}`, see `collections::list`)
    /// shares its parent's buffer. The receiver gets an owned list holding
    /// just the elements the view covers, read from the parent as far as the
    /// parent's allocation goes. A parent that is not an object of this heap
    /// cannot be read; the view is left as it is and the parent lent. False
    /// when the object is left a view (or too short to be a list at all).
    fn flatten_list_view(&mut self, object: u32) -> bool {
        let container = Some(object);
        // A LIST node describes sets too.
        if self.read_word(container, 8)
            == Some(crate::collections::table::view_sentinel(1) as usize)
        {
            return self.flatten_table::<1>(object);
        }
        if self.read_word(container, 8) != Some(crate::collections::list::VIEW as usize) {
            return self.flatten_table::<1>(object);
        }
        let (Some(len), Some(parent), Some(offset)) = (
            self.read_word(container, 0),
            self.read_word(container, 16),
            self.read_word(container, 24),
        ) else {
            return false;
        };
        let Some(parent_size) = self.heap.live_allocation_size(parent as *const u8) else {
            self.out.lend.push(parent);
            return false;
        };
        let available = parent_size.saturating_sub(16) / 8;
        let take = len.min(available.saturating_sub(offset));
        let mut bytes = Vec::with_capacity(16 + 8 * take);
        bytes.extend_from_slice(&(take as u64).to_ne_bytes());
        bytes.extend_from_slice(&(take as u64).to_ne_bytes());
        let slots = unsafe {
            std::slice::from_raw_parts((parent as *const u8).add(16 + 8 * offset), 8 * take)
        };
        bytes.extend_from_slice(slots);
        self.out.objects[object as usize].bytes = bytes;
        true
    }

    /// A map (`W = 2`) or set (`W = 1`) value: a table, or a view of the
    /// first entries of one (see `collections::table`). The receiver gets a
    /// table of just the live entries. The table's index, the word after its
    /// entries, points into this heap: the copy has none (0). An owned list
    /// also comes here: its capacity word's high bits count slots appends
    /// wrote past it, which the copy does not have. A view whose table or
    /// index this heap cannot vouch for is left as it is, the table lent.
    /// False when the object is left a view (or too short for its header).
    fn flatten_table<const W: usize>(&mut self, object: u32) -> bool {
        use crate::collections::table;
        const CAP_MASK: usize = (1 << 32) - 1;
        const TAG_SHIFT: u32 = 56;
        let container = Some(object);
        let Some(capword) = self.read_word(container, 8) else {
            return false;
        };
        let read = |address: usize, offset: usize| unsafe {
            ((address + offset) as *const usize).read_unaligned()
        };
        if capword as u64 != table::view_sentinel(W) {
            let cap = capword & CAP_MASK;
            let bytes = &mut self.out.objects[object as usize].bytes;
            let keep = if W == 1 { CAP_MASK } else { usize::MAX };
            bytes[8..16].copy_from_slice(&(capword & keep).to_ne_bytes());
            let entries_end = (16 + 8 * W * cap).min(bytes.len());
            bytes[entries_end..].fill(0);
            return true;
        }
        let (Some(n), Some(table_address)) =
            (self.read_word(container, 0), self.read_word(container, 16))
        else {
            return false;
        };
        let lend = |this: &mut Self| {
            this.out.lend.push(table_address);
            false
        };
        let Some(table_size) = self.heap.live_allocation_size(table_address as *const u8) else {
            return lend(self);
        };
        let table_capword = read(table_address, 8);
        let cap = table_capword & CAP_MASK;
        if table_size < table::table_bytes::<W>(cap) || n > cap {
            return lend(self);
        }
        let index = unsafe { table::index_address::<W>(table_address as *const u8) };
        let index_ok = index != 0
            && self
                .heap
                .live_allocation_size(index as *const u8)
                .is_some_and(|size| {
                    let (written, slots) = (read(index, 0), read(index, 8));
                    size >= table::index_bytes(slots, cap) && n <= written && written <= cap
                });
        if !index_ok {
            return lend(self);
        }
        let entries = unsafe { table::live_entries_in::<W>(table_address as *const u8, n) };
        let count = entries.len();
        let mut bytes = Vec::with_capacity(table::table_bytes::<W>(count));
        bytes.extend_from_slice(&(count as u64).to_ne_bytes());
        let tag = (table_capword >> TAG_SHIFT) as u64;
        bytes.extend_from_slice(&((tag << TAG_SHIFT) | count as u64).to_ne_bytes());
        for entry in &entries {
            for word in entry {
                bytes.extend_from_slice(&word.to_ne_bytes());
            }
        }
        bytes.extend_from_slice(&0u64.to_ne_bytes());
        self.out.objects[object as usize].bytes = bytes;
        true
    }

    /// Capture a list whose elements are `elem`-shaped, without a LIST node
    /// of its own in the table.
    fn list_of(&mut self, address: usize, elem: u32) -> Option<u32> {
        if address == 0 {
            return None;
        }
        let index = self.object(address, LEAF_NODE)?;
        if self.flatten_list_view(index) && self.kind(elem) != SCALAR {
            let size = self.out.objects[index as usize].bytes.len();
            let len = self.read_word(Some(index), 0).unwrap_or(0);
            for slot in 0..len.min(size.saturating_sub(16) / 8) {
                self.value(Some(index), elem, 16 + 8 * slot);
            }
        }
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(heap: &mut ActorHeap, text: &str) -> usize {
        let object = heap.alloc(8 + text.len(), 8);
        unsafe {
            (object as *mut u64).write(text.len() as u64);
            std::ptr::copy_nonoverlapping(text.as_ptr(), object.add(8), text.len());
        }
        object as usize
    }

    unsafe fn text(object: usize) -> String {
        let len = *(object as *const u64) as usize;
        String::from_utf8(std::slice::from_raw_parts((object + 8) as *const u8, len).to_vec())
            .unwrap()
    }

    fn list(heap: &mut ActorHeap, slots: &[usize]) -> usize {
        let object = heap.alloc(16 + 8 * slots.len(), 8) as *mut usize;
        unsafe {
            object.write(slots.len());
            object.add(1).write(slots.len());
            for (index, &slot) in slots.iter().enumerate() {
                object.add(2 + index).write(slot);
            }
        }
        object as usize
    }

    /// Send `data` from one heap to another and return the receiver's bytes.
    ///
    /// The receiver is leaked: the bytes point into it, and a caller that
    /// ignored it with `_` would otherwise read the copies after their pages
    /// were freed.
    fn transfer(
        sender: &ActorHeap,
        data: &[u8],
        shape: &[u32],
    ) -> (&'static ActorHeap, Vec<u8>, Captured) {
        assert_eq!(shape[0] as usize, shape.len(), "table length word");
        let captured = unsafe { capture(sender, data, 0, shape.as_ptr()) };
        let receiver = Box::leak(Box::new(ActorHeap::new()));
        let mut received = data.to_vec();
        unsafe { captured.materialize(receiver, received.as_mut_ptr()) };
        (receiver, received, captured)
    }

    fn word(bytes: &[u8], offset: usize) -> usize {
        usize::from_ne_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    fn word_at(address: usize, offset: usize) -> usize {
        unsafe { ((address + offset) as *const usize).read_unaligned() }
    }

    /// `{literal, owned, pid}`: strings in and outside the heap, and a pid.
    fn literal_owned_and_pid(sender: &mut ActorHeap) -> (Vec<u8>, [u32; 11]) {
        // A string literal: `{len, bytes}` in constant data, in no heap.
        static LITERAL: [u64; 2] = [5, u64::from_le_bytes(*b"hello\0\0\0")];
        let owned = string(sender, "owned");
        let mut data = Vec::new();
        for word in [LITERAL.as_ptr() as usize, owned, 7] {
            data.extend_from_slice(&word.to_ne_bytes());
        }
        (data, [11, AGG, 3, 0, 9, 8, 9, 16, 10, STRING, PID])
    }

    /// An object of `words` in `heap`, sized to exactly those words.
    fn object(heap: &mut ActorHeap, words: &[usize]) -> usize {
        let object = heap.alloc(8 * words.len(), 8) as *mut usize;
        for (index, &word) in words.iter().enumerate() {
            unsafe { object.add(index).write(word) };
        }
        object as usize
    }

    /// A shape table: `nodes` after its length word.
    fn table(nodes: &[u32]) -> Vec<u32> {
        [&[nodes.len() as u32 + 1][..], nodes].concat()
    }

    /// A table shorter than its nodes say, a sum tag past the value, an
    /// unknown kind: a wrong shape copies what it can describe and leaves
    /// the rest where it is, and never reads past what it was given.
    #[test]
    fn a_malformed_shape_copies_nothing_it_cannot_describe() {
        let mut sender = ActorHeap::new();
        let text = string(&mut sender, "text");
        let data = [text, text].map(usize::to_ne_bytes).concat();
        let capture = |data: &[u8], nodes: &[u32]| {
            let shape = table(nodes);
            unsafe { capture(&sender, data, 0, shape.as_ptr()) }
        };
        for (data, nodes) in [
            (&data[..], &[AGG, 2][..]),
            (&[][..], &[SUM, 1, 0, 0]),
            (&[1][..], &[SUM, 2]),
            (&[0][..], &[SUM, 1, 0, 2]),
            (&data[..8], &[CLOSURE]),
        ] {
            assert!(capture(data, nodes).is_empty(), "{nodes:?}");
        }
        let unknown = capture(&data[..8], &[99]);
        assert_eq!((unknown.objects.len(), unknown.relocs.len()), (1, 1));
        assert!(
            unknown.objects[0].relocs.is_empty(),
            "its insides are not read"
        );
    }

    /// Objects a shape reads a header from, cut short, are copied as they
    /// are; a closure environment's table must be there and aligned.
    #[test]
    fn objects_too_short_for_their_header_are_copied_as_they_are() {
        let mut sender = ActorHeap::new();
        let short = sender.alloc(4, 8) as usize;
        let short_view = object(&mut sender, &[2, crate::collections::list::VIEW as usize]);
        let short_map = object(&mut sender, &[1]);
        let sentinel = crate::collections::table::view_sentinel(2) as usize;
        let short_table_view = object(&mut sender, &[1, sentinel]);
        let captured_text = string(&mut sender, "captured");
        let env_without_table = object(&mut sender, &[0, captured_text]);
        let env_misaligned = object(&mut sender, &[1, 0]);
        let map = [MAP, 4, 4, LEAF];
        for (address, nodes) in [
            (short, &[QUEUE, 2][..]),
            (short_view, &[LIST, 3, LEAF]),
            (short_map, &map),
            (short_table_view, &map),
        ] {
            let shape = table(nodes);
            let data = address.to_ne_bytes();
            let captured = unsafe { capture(&sender, &data, 0, shape.as_ptr()) };
            assert_eq!(captured.objects.len(), 1, "{nodes:?}");
            assert!(captured.objects[0].relocs.is_empty() && captured.lend.is_empty());
        }
        for env in [env_without_table, env_misaligned] {
            let shape = table(&[CLOSURE]);
            let data = [1, env].map(usize::to_ne_bytes).concat();
            let captured = unsafe { capture(&sender, &data, 0, shape.as_ptr()) };
            assert_eq!(
                captured.objects.len(),
                1,
                "the environment, not what it holds"
            );
        }
    }

    /// A queue of plain values copies its buffer and reads none of its
    /// slots; one without a buffer copies just itself.
    #[test]
    fn queues_of_scalars_and_without_a_buffer() {
        let mut sender = ActorHeap::new();
        let buffer = list(&mut sender, &[1, 2]);
        let queue = object(&mut sender, &[buffer, 0, 2]);
        let empty = object(&mut sender, &[0, 0, 0]);
        let shape = table(&[QUEUE, 3, SCALAR]);
        let data = [queue, empty].map(usize::to_ne_bytes);
        let full = unsafe { capture(&sender, &data[0], 0, shape.as_ptr()) };
        assert_eq!(full.objects.len(), 2);
        assert_eq!(full.objects[0].relocs, [(0, 1)]);
        let bare = unsafe { capture(&sender, &data[1], 0, shape.as_ptr()) };
        assert_eq!(bare.objects.len(), 1);
        assert!(bare.objects[0].relocs.is_empty());
    }

    /// A list view or a map view whose storage this heap cannot vouch for is
    /// left as it is, and what it points at lent once. Its header words are
    /// not entries: before, they were walked as such, the storage lent twice
    /// or, when it was an object here, copied and the view pointed at that.
    #[test]
    fn views_of_storage_this_heap_cannot_vouch_for_are_lent() {
        let mut sender = ActorHeap::new();
        let elsewhere = 0x1000usize;
        let list_view = [1, crate::collections::list::VIEW as usize, elsewhere, 0];
        let list_view = object(&mut sender, &list_view);
        let sentinel = crate::collections::table::view_sentinel(2) as usize;
        let map_view = |heap: &mut ActorHeap, n: usize, table: usize| {
            object(heap, &[n, sentinel, table, 0, 0])
        };
        // A table of one entry `{len, cap, key, value, index}` with no index.
        let unindexed = object(&mut sender, &[1, 1, 0, 0, 0]);
        let map = [MAP, 4, 4, LEAF];
        let cases = [
            (list_view, &[LIST, 3, LEAF][..], elsewhere),
            (map_view(&mut sender, 1, elsewhere), &map, elsewhere),
            (map_view(&mut sender, 2, unindexed), &map, unindexed),
            (map_view(&mut sender, 1, unindexed), &map, unindexed),
        ];
        for (address, nodes, lent) in cases {
            let shape = table(nodes);
            let data = address.to_ne_bytes();
            let captured = unsafe { capture(&sender, &data, 0, shape.as_ptr()) };
            assert_eq!(captured.lend, [lent], "{nodes:?}");
            assert_eq!(captured.objects.len(), 1, "the view alone: {nodes:?}");
        }
    }

    #[test]
    fn a_copy_for_another_node_takes_literals_too_and_notes_its_pids() {
        let mut sender = ActorHeap::new();
        let (data, shape) = literal_owned_and_pid(&mut sender);

        let here = unsafe { capture(&sender, &data, 0, shape.as_ptr()) };
        assert_eq!(here.objects.len(), 1, "a literal stays where it is");
        assert_eq!(here.lend, [word(&data, 0)]);
        assert!(here.pids.is_empty());

        let there = unsafe { capture_for_node(&sender, &data, shape.as_ptr()) }.unwrap();
        assert!(there.lend.is_empty());
        assert_eq!(there.pids, [(None, 16)]);
        let receiver = Box::leak(Box::new(ActorHeap::new()));
        let mut received = data.clone();
        unsafe { there.materialize(receiver, received.as_mut_ptr()) };
        assert_eq!(unsafe { text(word(&received, 0)) }, "hello");
        assert_eq!(unsafe { text(word(&received, 8)) }, "owned");
        assert!(receiver.is_live_allocation(word(&received, 0) as *const u8, 13));
    }

    #[test]
    fn code_and_runtime_objects_cannot_leave_their_node() {
        let sender = ActorHeap::new();
        let closure = [0usize.to_ne_bytes(), 0usize.to_ne_bytes()].concat();
        let shape = [6, AGG, 1, 0, 5, CLOSURE];
        assert_eq!(
            unsafe { capture_for_node(&sender, &closure, shape.as_ptr()) },
            None,
            "a named function is still code"
        );
        let handle = 0x1000usize.to_ne_bytes();
        let shape = [2, SHARED];
        assert_eq!(
            unsafe { capture_for_node(&sender, &handle, shape.as_ptr()) },
            None
        );
    }

    #[test]
    fn a_capture_round_trips_and_a_malformed_one_is_refused() {
        let mut sender = ActorHeap::new();
        let (data, shape) = literal_owned_and_pid(&mut sender);
        let mut captured = unsafe { capture_for_node(&sender, &data, shape.as_ptr()) }.unwrap();
        captured.objects[0].relocs.push((0, 1));
        let nodes = ["a@127.0.0.1:1".to_string()];
        let mut wire = Vec::new();
        captured.encode(&mut wire, &nodes);
        assert_eq!(
            Captured::decode(&wire, data.len()),
            Some((captured.clone(), nodes.to_vec()))
        );

        assert_eq!(Captured::decode(&wire[..wire.len() - 1], data.len()), None);
        assert_eq!(
            Captured::decode(&[wire.clone(), vec![0]].concat(), data.len()),
            None
        );
        assert_eq!(
            Captured::decode(&wire, 16),
            None,
            "a relocation or pid past the message"
        );
        let mut bad = captured.clone();
        bad.objects[0].relocs = vec![(0, 9)];
        let mut wire = Vec::new();
        bad.encode(&mut wire, &nodes);
        assert_eq!(Captured::decode(&wire, data.len()), None, "no object 9");
        let mut bad = captured;
        bad.pids = vec![(Some(0), 13)];
        let mut wire = Vec::new();
        bad.encode(&mut wire, &nodes);
        assert_eq!(
            Captured::decode(&wire, data.len()),
            None,
            "a pid past its object"
        );
    }

    #[test]
    fn map_pids_rewrites_each_where_it_sits() {
        let mut data = 7u64.to_le_bytes().to_vec();
        let mut captured = Captured {
            objects: vec![OwnedObject {
                bytes: [0u64, 9]
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .collect(),
                relocs: Vec::new(),
            }],
            pids: vec![(None, 0), (Some(0), 8)],
            ..Captured::default()
        };
        captured.map_pids(&mut data, |pid| pid * 10);
        assert_eq!(data, 70u64.to_le_bytes());
        assert_eq!(captured.objects[0].bytes[8..], 90u64.to_le_bytes());
    }

    #[test]
    fn a_string_is_copied_and_the_original_can_be_overwritten() {
        let mut sender = ActorHeap::new();
        let original = string(&mut sender, "payload-42");
        let shape = [2, LEAF];

        let (receiver, received, _) = transfer(&sender, &original.to_ne_bytes(), &shape);
        unsafe { std::ptr::write_bytes((original + 8) as *mut u8, b'x', 10) };

        let copy = word(&received, 0);
        assert_ne!(copy, original);
        assert!(receiver.is_live_allocation(copy as *const u8, 18));
        assert_eq!(unsafe { text(copy) }, "payload-42");
    }

    #[test]
    fn scalars_are_never_mistaken_for_pointers() {
        // An Int that happens to equal a live object's address must survive.
        let mut sender = ActorHeap::new();
        let address = string(&mut sender, "not a reference here");
        let shape = [2, SCALAR];

        let (_, received, captured) = transfer(&sender, &address.to_ne_bytes(), &shape);

        assert_eq!(word(&received, 0), address);
        assert!(captured.is_empty());
    }

    #[test]
    fn a_by_value_sum_follows_only_the_active_variant() {
        // type Note = Named(String) | Pair(String, Int) | Blank, laid out as
        // `{ i8 tag, [23 x i8] }` with payload fields at 8 and 16.
        let mut sender = ActorHeap::new();
        let name = string(&mut sender, "payload-42");
        #[rustfmt::skip]
        let shape = [
            12, SUM, 2,
            0, 1, 8, 11,        // Named(String)
            1, 1, 8, 11,        // Pair(String, Int): only the string is a reference
            LEAF,               // 11
        ];
        let mut pair = vec![0u8; 24];
        pair[0] = 1;
        pair[8..16].copy_from_slice(&name.to_ne_bytes());
        pair[16..24].copy_from_slice(&7usize.to_ne_bytes());

        let (_, received, _) = transfer(&sender, &pair, &shape);
        assert_ne!(word(&received, 8), name);
        assert_eq!(unsafe { text(word(&received, 8)) }, "payload-42");
        assert_eq!(word(&received, 16), 7);

        // Blank (tag 2) has no fields: payload bytes are left exactly as they were.
        let mut blank = pair.clone();
        blank[0] = 2;
        let (_, received, captured) = transfer(&sender, &blank, &shape);
        assert_eq!(received, blank);
        assert!(captured.is_empty());
    }

    #[test]
    fn list_views_are_captured_as_owned_lists() {
        // A view onto a parent's buffer is sent as a plain list of the
        // elements it covers; the receiver never sees the parent pointer.
        let mut sender = ActorHeap::new();
        let names = ["a", "b", "c", "d"].map(|text| string(&mut sender, text));
        let parent = list(&mut sender, &names);
        let view = sender.alloc(32, 8) as *mut usize;
        unsafe {
            view.write(2); // len
            view.add(1).write(crate::collections::list::VIEW as usize);
            view.add(2).write(parent);
            view.add(3).write(1); // offset: ["b", "c"]
        }
        let shape = [4, LIST, 3, LEAF];
        let data = (view as usize).to_ne_bytes();

        let (receiver, received, captured) = transfer(&sender, &data, &shape);
        let copy = word(&received, 0);
        assert_ne!(copy, view as usize);
        assert_eq!(
            receiver.live_allocation_size(copy as *const u8),
            Some(16 + 2 * 8)
        );
        assert_eq!(word_at(copy, 0), 2);
        assert_eq!(word_at(copy, 8), 2);
        assert_eq!(unsafe { text(word_at(copy, 16)) }, "b");
        assert_eq!(unsafe { text(word_at(copy, 24)) }, "c");
        assert!(captured.lend.is_empty());

        // A view whose length outruns its parent is clamped to the parent.
        unsafe { view.write(10) };
        let (_, received, _) = transfer(&sender, &data, &shape);
        assert_eq!(word_at(word(&received, 0), 0), 3);
    }

    #[test]
    fn collections_keep_shared_structure_shared() {
        // [s, s] inside a tuple with a boxed struct { String, Int }.
        let mut sender = ActorHeap::new();
        let shared = string(&mut sender, "twice");
        let items = list(&mut sender, &[shared, shared]);
        let boxed = sender.alloc(16, 8) as *mut usize;
        unsafe {
            boxed.write(shared);
            boxed.add(1).write(99);
        }
        let tuple = sender.alloc(24, 8) as *mut usize;
        unsafe {
            tuple.write(2);
            tuple.add(1).write(items);
            tuple.add(2).write(boxed as usize);
        }
        #[rustfmt::skip]
        let shape = [
            14, TUPLE, 2, 5, 7,
            LIST, 13,           // 5: List<String>
            BOXED, 9,           // 7: boxed struct
            AGG, 1, 0, 13,      // 9: { String at 0, Int at 8 }
            LEAF,               // 13
        ];

        let (_, received, captured) = transfer(&sender, &(tuple as usize).to_ne_bytes(), &shape);

        assert_eq!(captured.objects.len(), 4, "tuple, list, box and ONE string");
        let tuple = word(&received, 0) as *const usize;
        let (items, boxed) =
            unsafe { (*tuple.add(1) as *const usize, *tuple.add(2) as *const usize) };
        let (first, second, in_box) = unsafe { (*items.add(2), *items.add(3), *boxed) };
        assert_eq!(first, second);
        assert_eq!(first, in_box);
        assert_ne!(first, shared);
        assert_eq!(unsafe { text(first) }, "twice");
        assert_eq!(unsafe { *boxed.add(1) }, 99);
    }

    #[test]
    fn a_long_chain_does_not_recurse() {
        // type Chain = Link(Chain) | End, 200k links of boxed `{ i8, ptr }`.
        let mut sender = ActorHeap::new();
        let mut next = 0usize;
        for _ in 0..200_000 {
            let cell = sender.alloc(16, 8) as *mut usize;
            unsafe {
                cell.write(0);
                cell.add(1).write(next);
            }
            next = cell as usize;
        }
        let mut head = vec![0u8; 16];
        head[8..16].copy_from_slice(&next.to_ne_bytes());
        #[rustfmt::skip]
        let shape = [
            9, SUM, 1, 0, 1, 8, 7,   // 1: Link's field is a boxed Chain
            BOXED, 1,                // 7
        ];

        let (_, _, captured) = transfer(&sender, &head, &shape);
        assert_eq!(captured.objects.len(), 200_000);
    }

    #[test]
    fn a_json_tree_describes_itself() {
        let mut sender = ActorHeap::new();
        let json = |heap: &mut ActorHeap, tag: u8, value: usize| {
            let node = heap.alloc(16, 8);
            unsafe {
                node.write(tag);
                let mark = crate::json::TREE_MARK;
                std::ptr::copy_nonoverlapping(mark.as_ptr(), node.add(1), mark.len());
                (node.add(8) as *mut usize).write(value);
            }
            node as usize
        };
        let text_value = string(&mut sender, "deep");
        let leaf = json(&mut sender, JSON_TAG_STR, text_value);
        let number = json(&mut sender, 2, 5);
        let array = list(&mut sender, &[leaf, number]);
        let root = json(&mut sender, JSON_TAG_ARRAY, array);

        let (_, received, captured) = transfer(&sender, &root.to_ne_bytes(), &[2, JSON]);

        assert_eq!(captured.objects.len(), 5);
        let root = word(&received, 0) as *const usize;
        let array = unsafe { *root.add(1) as *const usize };
        let leaf = unsafe { *array.add(2) as *const usize };
        assert_eq!(unsafe { text(*leaf.add(1)) }, "deep");
        assert_eq!(unsafe { *(*array.add(3) as *const usize).add(1) }, 5);
    }

    /// A `Json` that is JSON text is a string, copied whole; its bytes are
    /// not read as a tree's even where they look like one (a length whose low
    /// byte is an array's tag, a word that is a live object's address).
    #[test]
    fn json_text_is_copied_as_a_string() {
        let mut sender = ActorHeap::new();
        let decoy = string(&mut sender, "decoy");
        let text = sender.alloc(16, 8) as *mut usize;
        unsafe {
            text.write(JSON_TAG_ARRAY as usize);
            text.add(1).write(decoy);
        }
        let (_, _, captured) = transfer(&sender, &(text as usize).to_ne_bytes(), &[2, JSON]);
        assert_eq!(captured.objects.len(), 1);
        assert!(captured.objects[0].relocs.is_empty());
    }

    #[test]
    fn a_closure_is_copied_through_the_table_its_environment_names() {
        let mut sender = ActorHeap::new();
        let captured = string(&mut sender, "captured by the closure");
        // What codegen emits for an environment holding one String: the table
        // pointer at offset 0, the capture at offset 8.
        let env_table: &'static [u32; 6] = Box::leak(Box::new([6, AGG, 1, 8, 5, LEAF]));
        let env = sender.alloc(16, 8) as *mut usize;
        unsafe {
            env.write(env_table.as_ptr() as usize);
            env.add(1).write(captured);
        }
        let code = 0x1234usize;
        let mut data = Vec::new();
        data.extend_from_slice(&code.to_ne_bytes());
        data.extend_from_slice(&(env as usize).to_ne_bytes());
        let shape = [6, AGG, 1, 0, 5, CLOSURE];

        let captured_message = unsafe { capture(&sender, &data, 0, shape.as_ptr()) };
        assert_eq!(captured_message.objects.len(), 2, "environment and string");
        assert!(captured_message.lend.is_empty(), "nothing is left to lend");

        drop(sender);
        let mut receiver = ActorHeap::new();
        let mut received = data.clone();
        unsafe { captured_message.materialize(&mut receiver, received.as_mut_ptr()) };
        assert_eq!(word(&received, 0), code, "the code pointer is not data");
        let new_env = word(&received, 8) as *const usize;
        assert_ne!(new_env as usize, env as usize);
        unsafe {
            assert_eq!(
                *new_env,
                env_table.as_ptr() as usize,
                "still describes itself"
            );
            assert_eq!(text(*new_env.add(1)), "captured by the closure");
        }
    }

    #[test]
    fn a_plain_function_or_a_foreign_environment_is_not_followed() {
        let sender = ActorHeap::new();
        let foreign = Box::leak(Box::new([0usize; 2])) as *const _ as usize;
        let mut data = Vec::new();
        for words in [[0x1234usize, 0], [0x1234, foreign]] {
            for word in words {
                data.extend_from_slice(&word.to_ne_bytes());
            }
        }
        let shape = [9, AGG, 2, 0, 7, 16, 7, CLOSURE, SCALAR];
        let captured = unsafe { capture(&sender, &data, 0, shape.as_ptr()) };
        assert!(captured.objects.is_empty());
        assert_eq!(
            captured.lend,
            vec![foreign],
            "kept alive by whoever owns it"
        );
    }

    #[test]
    fn what_cannot_be_copied_is_reported_for_lending() {
        let mut sender = ActorHeap::new();
        let environment = sender.alloc(16, 8) as usize;
        let literal = Box::leak(Box::new([5u64, 0])) as *const _ as usize;
        let mut data = Vec::new();
        data.extend_from_slice(&environment.to_ne_bytes());
        data.extend_from_slice(&literal.to_ne_bytes());
        let shape = [9, AGG, 2, 0, 7, 8, 8, SHARED, LEAF];

        let (_, received, captured) = transfer(&sender, &data, &shape);

        // Neither slot moved: one is shared by design, one is not ours to copy.
        assert_eq!(word(&received, 0), environment);
        assert_eq!(word(&received, 8), literal);
        assert_eq!(captured.lend, vec![environment, literal]);
        assert!(captured.objects.is_empty());
    }

    #[test]
    fn a_wrong_shape_cannot_read_out_of_bounds() {
        // A 16-byte string described as a list claiming 2^60 elements, and an
        // aggregate reaching past the end of the message.
        let mut sender = ActorHeap::new();
        let fake = sender.alloc(16, 8) as *mut usize;
        unsafe { fake.write(1 << 60) };
        let shape = [9, AGG, 2, 0, 7, 4096, 7, LIST, 7];

        let (_, received, captured) = transfer(&sender, &(fake as usize).to_ne_bytes(), &shape);

        assert_eq!(captured.objects.len(), 1);
        assert_eq!(unsafe { *(word(&received, 0) as *const usize) }, 1 << 60);
    }
}
