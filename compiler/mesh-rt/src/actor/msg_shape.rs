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
//! | `QUEUE` | elem | pointer to `{front list, back list}` |
//! | `SHARED` | | a reference that cannot be copied by type |
//! | `CLOSURE` | | by-value `{fn, env}`; `env` points to an environment |
//!
//! A closure's type says nothing about what it captured, so an environment
//! describes itself: its first word points at the shape table the compiler
//! emitted for it (root: the environment by value), or is null when it holds
//! no references. An environment that is not an object of the sender's heap,
//! such as one the runtime made, is lent like any `SHARED` reference.

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

// Nodes the runtime supplies itself, for the self-describing JSON tree. They
// sit above any real table index.
const JSON_NODE: u32 = u32::MAX;
const JSON_ARRAY_NODE: u32 = u32::MAX - 1;
const JSON_OBJECT_NODE: u32 = u32::MAX - 2;
const LEAF_NODE: u32 = u32::MAX - 3;
/// A closure environment, which names its own table.
const ENV_NODE: u32 = u32::MAX - 4;

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
}

impl Captured {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.lend.is_empty()
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
    let Some(table) = Table::at(shape) else {
        return Captured::default();
    };
    let mut capture = Capture {
        heap,
        table,
        data,
        out: Captured::default(),
        seen: FxHashMap::default(),
        pending: Vec::new(),
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
            _ => self.word(node).unwrap_or(SCALAR),
        }
    }

    /// The `n`th node operand of `node`.
    fn operand(&self, node: u32, n: u32) -> u32 {
        match (node, n) {
            (JSON_ARRAY_NODE, _) | (JSON_OBJECT_NODE, 1) => JSON_NODE,
            (JSON_OBJECT_NODE, _) => LEAF_NODE,
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
        // this test and stay where they are.
        let size = self.heap.live_allocation_size(address as *const u8)?;
        let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, size) }.to_vec();
        let index = self.out.objects.len() as u32;
        self.out.objects.push(OwnedObject {
            bytes,
            relocs: Vec::new(),
        });
        self.seen.insert(address, index);
        if self.kind(node) != LEAF {
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
                // A queue is two lists of the same element type.
                for offset in [0, 8] {
                    let Some(word) = self.read_word(container, offset) else {
                        return;
                    };
                    if let Some(index) = self.list_of(word, self.operand(node, 0)) {
                        self.relocs(container).push((offset, index));
                    }
                }
            }
            JSON => {
                let tag = self.out.objects[object as usize].bytes.first().copied();
                let inner = match tag {
                    Some(JSON_TAG_STR) => LEAF_NODE,
                    Some(JSON_TAG_ARRAY) => JSON_ARRAY_NODE,
                    Some(JSON_TAG_OBJECT) => JSON_OBJECT_NODE,
                    _ => return,
                };
                self.value(container, inner, 8);
            }
            _ => {}
        }
    }

    /// Capture a list whose elements are `elem`-shaped, without a LIST node
    /// of its own in the table.
    fn list_of(&mut self, address: usize, elem: u32) -> Option<u32> {
        if address == 0 {
            return None;
        }
        let index = self.object(address, LEAF_NODE)?;
        if self.kind(elem) != SCALAR {
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
    fn transfer(sender: &ActorHeap, data: &[u8], shape: &[u32]) -> (ActorHeap, Vec<u8>, Captured) {
        assert_eq!(shape[0] as usize, shape.len(), "table length word");
        let captured = unsafe { capture(sender, data, 0, shape.as_ptr()) };
        let mut receiver = ActorHeap::new();
        let mut received = data.to_vec();
        unsafe { captured.materialize(&mut receiver, received.as_mut_ptr()) };
        (receiver, received, captured)
    }

    fn word(bytes: &[u8], offset: usize) -> usize {
        usize::from_ne_bytes(bytes[offset..offset + 8].try_into().unwrap())
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
