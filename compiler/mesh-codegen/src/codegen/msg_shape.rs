//! Shape tables: the runtime's description of a value that crosses actors.
//!
//! A message must not carry pointers into the sender's heap, so the runtime
//! gives the receiver its own copy of whatever the message references. It can
//! only do that knowing where the references are. [`MsgShape`] says what a
//! value holds; this module maps that onto the value's actual representation
//! -- field offsets, boxed payloads, uniform collection slots -- using the
//! same LLVM types the rest of codegen builds values with, and emits the
//! result as a constant table.
//!
//! The table format is documented in `mesh-rt/src/actor/msg_shape.rs` and
//! must stay in sync with it. The runtime follows a slot only when it holds
//! the exact start of a live object in the sender's heap, so describing a
//! slot that turns out to hold an inline value is harmless.

use inkwell::types::{BasicTypeEnum, StructType};
use inkwell::values::PointerValue;
use rustc_hash::FxHashMap;

use super::types::variant_struct_type;
use super::CodeGen;
use crate::mir::{MirExpr, MirType, MsgShape};

const SCALAR: u32 = 0;
const LEAF: u32 = 1;
/// Lists may also be views (`{len, VIEW, parent, offset}`); the runtime
/// flattens those while capturing.
const LIST: u32 = 2;
const MAP: u32 = 3;
const TUPLE: u32 = 4;
const BOXED: u32 = 5;
const AGG: u32 = 6;
const SUM: u32 = 7;
const JSON: u32 = 8;
const QUEUE: u32 = 9;
const SHARED: u32 = 10;
const CLOSURE: u32 = 11;

impl<'ctx> CodeGen<'ctx> {
    /// The shape of the value `expr` evaluates to: what lowering worked out
    /// from its full type, or else the most its `MirType` alone reveals.
    pub(crate) fn message_shape(&self, expr: &MirExpr) -> MsgShape {
        match expr {
            MirExpr::Shaped { shape, .. } => shape.clone(),
            other => self.shape_of_mir_type(other.ty(), &mut Vec::new()),
        }
    }

    /// `MirType` erases what collections hold, so anything behind an opaque
    /// pointer is `Shared`: not copied, but kept alive by the heap that owns it.
    fn shape_of_mir_type(&self, ty: &MirType, open: &mut Vec<String>) -> MsgShape {
        match ty {
            MirType::Int
            | MirType::Float
            | MirType::Bool
            | MirType::Unit
            | MirType::Never
            | MirType::Pid(_)
            | MirType::FnPtr(..) => MsgShape::Scalar,
            MirType::String => MsgShape::Leaf,
            MirType::Ptr => MsgShape::Shared,
            MirType::Closure(..) => MsgShape::Closure,
            MirType::Tuple(elems) => MsgShape::Tuple(
                elems
                    .iter()
                    .map(|elem| self.shape_of_mir_type(elem, open))
                    .collect(),
            ),
            MirType::Struct(name) | MirType::SumType(name) if open.contains(name) => {
                MsgShape::Recur(name.clone())
            }
            MirType::Struct(name) => {
                let Some(fields) = self.mir_struct_defs.get(name).cloned() else {
                    return MsgShape::Shared;
                };
                open.push(name.clone());
                let shapes = fields
                    .iter()
                    .map(|(_, field)| self.shape_of_mir_type(field, open))
                    .collect();
                open.pop();
                MsgShape::Struct(name.clone(), shapes)
            }
            MirType::SumType(name) => {
                let Some(def) = self.lookup_sum_type_def(name).cloned() else {
                    return MsgShape::Shared;
                };
                open.push(name.clone());
                let variants = def
                    .variants
                    .iter()
                    .map(|variant| {
                        let shapes = variant
                            .fields
                            .iter()
                            .map(|field| self.shape_of_mir_type(field, open))
                            .collect();
                        (variant.name.clone(), shapes)
                    })
                    .collect();
                open.pop();
                MsgShape::Sum(name.clone(), variants)
            }
        }
    }

    /// Table for a value of LLVM type `ty` stored by value, such as a message
    /// buffer. `None` when the value holds no references.
    pub(crate) fn shape_table_for_value(
        &self,
        shape: &MsgShape,
        ty: BasicTypeEnum<'ctx>,
    ) -> Option<PointerValue<'ctx>> {
        let mut table = ShapeTable::new(self);
        table.root(|table| table.value(shape, ty));
        table.emit()
    }

    /// Table for a buffer of 8-byte argument slots (`coerce_to_i64`), such as
    /// service call arguments.
    pub(crate) fn shape_table_for_slots(&self, shapes: &[MsgShape]) -> Option<PointerValue<'ctx>> {
        let mut table = ShapeTable::new(self);
        table.root(|table| {
            let fields = shapes
                .iter()
                .enumerate()
                .map(|(index, shape)| (8 * index as u32, table.packed_slot(shape)))
                .collect();
            table.aggregate(fields)
        });
        table.emit()
    }

    /// Table for a closure environment of LLVM type `env`, whose field 0 is the
    /// pointer to this very table and whose field `i + 1` holds capture `i`.
    /// `None` when no capture holds a reference.
    pub(crate) fn shape_table_for_env(
        &self,
        captures: &[MsgShape],
        env: StructType<'ctx>,
    ) -> Option<PointerValue<'ctx>> {
        let mut table = ShapeTable::new(self);
        table.root(|table| {
            let fields = captures
                .iter()
                .enumerate()
                .map(|(index, shape)| {
                    let field = index as u32 + 1;
                    let ty = env.get_field_type_at_index(field).unwrap();
                    (table.offset_of(&env, field), table.value(shape, ty))
                })
                .collect();
            table.aggregate(fields)
        });
        table.emit()
    }

    /// Table for one word encoded the way collection elements are
    /// (`convert_to_list_element`), such as a job's result.
    pub(crate) fn shape_table_for_element(&self, shape: &MsgShape) -> Option<PointerValue<'ctx>> {
        let mut table = ShapeTable::new(self);
        table.root(|table| table.slot(shape));
        table.emit()
    }
}

struct ShapeTable<'a, 'ctx> {
    codegen: &'a CodeGen<'ctx>,
    /// Word 0 is the length; the root node starts at word 1.
    words: Vec<u32>,
    /// By-value nodes of named types, so recursive types refer back to them.
    named: FxHashMap<String, u32>,
    /// Kinds with no operands, emitted once.
    plain: FxHashMap<u32, u32>,
}

impl<'a, 'ctx> ShapeTable<'a, 'ctx> {
    fn new(codegen: &'a CodeGen<'ctx>) -> Self {
        ShapeTable {
            codegen,
            words: vec![0],
            named: FxHashMap::default(),
            plain: FxHashMap::default(),
        }
    }

    /// The runtime starts at word 1, but a node can only be written after the
    /// nodes it refers to. Reserve the root's place with a one-field aggregate
    /// at offset 0, which describes exactly what its field does.
    fn root(&mut self, build: impl FnOnce(&mut Self) -> u32) {
        self.words.extend([AGG, 1, 0, 0]);
        let node = build(self);
        self.words[4] = node;
    }

    fn emit(mut self) -> Option<PointerValue<'ctx>> {
        let root = self.words[4] as usize;
        if self.words[root] == SCALAR {
            return None;
        }
        self.words[0] = self.words.len() as u32;
        let i32_type = self.codegen.context.i32_type();
        let values: Vec<_> = self
            .words
            .iter()
            .map(|&word| i32_type.const_int(word as u64, false))
            .collect();
        let global = self.codegen.module.add_global(
            i32_type.array_type(values.len() as u32),
            None,
            "msg_shape",
        );
        global.set_initializer(&i32_type.const_array(&values));
        global.set_constant(true);
        global.set_unnamed_addr(true);
        global.set_linkage(inkwell::module::Linkage::Private);
        Some(global.as_pointer_value())
    }

    fn node(&mut self, words: &[u32]) -> u32 {
        let index = self.words.len() as u32;
        self.words.extend_from_slice(words);
        index
    }

    fn plain(&mut self, kind: u32) -> u32 {
        if let Some(&index) = self.plain.get(&kind) {
            return index;
        }
        let index = self.node(&[kind]);
        self.plain.insert(kind, index);
        index
    }

    fn aggregate(&mut self, fields: Vec<(u32, u32)>) -> u32 {
        let fields: Vec<(u32, u32)> = fields
            .into_iter()
            .filter(|&(_, node)| self.words[node as usize] != SCALAR)
            .collect();
        if fields.is_empty() {
            return self.plain(SCALAR);
        }
        let mut words = vec![AGG, fields.len() as u32];
        words.extend(fields.into_iter().flat_map(|(offset, node)| [offset, node]));
        self.node(&words)
    }

    /// A tuple field or an argument slot. Unlike a collection element, an
    /// aggregate that fits the word is stored in it, not in a box
    /// (`codegen_make_tuple`, `coerce_to_i64`).
    fn packed_slot(&mut self, shape: &MsgShape) -> u32 {
        if let MsgShape::Struct(name, _) | MsgShape::Sum(name, _) | MsgShape::Recur(name) = shape {
            let ty = self.codegen.llvm_type(&self.named_mir_type(shape, name));
            let target_data = self.codegen.target_machine.get_target_data();
            if target_data.get_store_size(&ty) <= 8 {
                return self.value(shape, ty);
            }
        }
        self.slot(shape)
    }

    /// A word that holds either plain bits or a pointer: a collection slot,
    /// or a pointer-typed field.
    fn slot(&mut self, shape: &MsgShape) -> u32 {
        match shape {
            MsgShape::Scalar => self.plain(SCALAR),
            MsgShape::Leaf => self.plain(LEAF),
            MsgShape::Json => self.plain(JSON),
            MsgShape::Shared => self.plain(SHARED),
            // Two words: in a slot it sits in a box, like any aggregate.
            MsgShape::Closure => {
                let closure = self.plain(CLOSURE);
                self.node(&[BOXED, closure])
            }
            MsgShape::List(elem) => {
                let elem = self.slot(elem);
                self.node(&[LIST, elem])
            }
            MsgShape::Queue(elem) => {
                let elem = self.slot(elem);
                self.node(&[QUEUE, elem])
            }
            MsgShape::Map(key, value) => {
                let (key, value) = (self.slot(key), self.slot(value));
                self.node(&[MAP, key, value])
            }
            MsgShape::Tuple(elems) => {
                let mut words = vec![TUPLE, elems.len() as u32];
                words.extend(
                    elems
                        .iter()
                        .map(|elem| self.packed_slot(elem))
                        .collect::<Vec<_>>(),
                );
                self.node(&words)
            }
            // Aggregates do not fit a word: they sit in a box on the heap.
            MsgShape::Struct(name, _) | MsgShape::Sum(name, _) | MsgShape::Recur(name) => {
                // Reserve first: a recursive type reaches its own box again.
                let boxed = self.node(&[BOXED, 0]);
                let ty = self.codegen.llvm_type(&self.named_mir_type(shape, name));
                let inner = self.value(shape, ty);
                self.words[boxed as usize + 1] = inner;
                boxed
            }
        }
    }

    fn named_mir_type(&self, shape: &MsgShape, name: &str) -> MirType {
        let is_struct = match shape {
            MsgShape::Struct(..) => true,
            MsgShape::Sum(..) => false,
            _ => self.codegen.struct_types.contains_key(name),
        };
        if is_struct {
            MirType::Struct(name.to_string())
        } else {
            MirType::SumType(name.to_string())
        }
    }

    /// A value stored by value as LLVM type `ty`.
    fn value(&mut self, shape: &MsgShape, ty: BasicTypeEnum<'ctx>) -> u32 {
        if ty.is_pointer_type() {
            return match shape {
                // Generic payloads box scalars: `Some(1)` points at an i64.
                MsgShape::Scalar => self.plain(LEAF),
                other => self.slot(other),
            };
        }
        let BasicTypeEnum::StructType(struct_ty) = ty else {
            return self.plain(SCALAR);
        };
        match shape {
            MsgShape::Struct(name, fields) => self.by_value_struct(name, fields, struct_ty),
            MsgShape::Sum(name, variants) => self.by_value_sum(name, variants),
            MsgShape::Recur(name) => match self.named.get(name) {
                Some(&node) => node,
                None => self.plain(SCALAR),
            },
            MsgShape::Scalar => self.plain(SCALAR),
            MsgShape::Closure => self.plain(CLOSURE),
            // An aggregate the shape does not describe: every pointer in it
            // is a reference to keep alive.
            _ => self.pointers_of(struct_ty),
        }
    }

    fn offset_of(&self, ty: &StructType<'ctx>, index: u32) -> u32 {
        self.codegen
            .target_machine
            .get_target_data()
            .offset_of_element(ty, index)
            .unwrap_or(0) as u32
    }

    fn by_value_struct(&mut self, name: &str, fields: &[MsgShape], ty: StructType<'ctx>) -> u32 {
        if let Some(&node) = self.named.get(name) {
            return node;
        }
        if fields.len() != ty.count_fields() as usize {
            return self.pointers_of(ty);
        }
        // Reserve the node so a field that recurs into this type finds it.
        let fixed = self.node(&vec![SCALAR; 2 + 2 * fields.len()]);
        self.named.insert(name.to_string(), fixed);
        let described: Vec<(u32, u32)> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let field_ty = ty.get_field_type_at_index(index as u32).unwrap();
                (
                    self.offset_of(&ty, index as u32),
                    self.value(field, field_ty),
                )
            })
            .collect();
        let mut words = vec![AGG, described.len() as u32];
        words.extend(
            described
                .into_iter()
                .flat_map(|(offset, node)| [offset, node]),
        );
        self.words[fixed as usize..fixed as usize + words.len()].copy_from_slice(&words);
        fixed
    }

    fn by_value_sum(&mut self, name: &str, variants: &[(String, Vec<MsgShape>)]) -> u32 {
        if let Some(&node) = self.named.get(name) {
            return node;
        }
        let Some(def) = self.codegen.lookup_sum_type_def(name).cloned() else {
            return self.plain(SCALAR);
        };
        let size = 2 + def
            .variants
            .iter()
            .map(|variant| 2 + 2 * variant.fields.len())
            .sum::<usize>();
        let fixed = self.node(&vec![SCALAR; size]);
        self.named.insert(name.to_string(), fixed);

        let mut words = vec![SUM, def.variants.len() as u32];
        for variant in &def.variants {
            // Fields live where construction put them: in the variant overlay.
            let overlay = variant_struct_type(
                self.codegen.context,
                &variant.fields,
                &self.codegen.struct_types,
                &self.codegen.sum_type_layouts,
            );
            let shapes = variants
                .iter()
                .find(|(variant_name, _)| *variant_name == variant.name)
                .map(|(_, shapes)| shapes.as_slice())
                .unwrap_or_default();
            words.extend([variant.tag as u32, variant.fields.len() as u32]);
            for index in 0..variant.fields.len() {
                let field_ty = overlay.get_field_type_at_index(index as u32 + 1).unwrap();
                let node = match shapes.get(index) {
                    Some(shape) => self.value(shape, field_ty),
                    None => self.value(&MsgShape::Shared, field_ty),
                };
                words.extend([self.offset_of(&overlay, index as u32 + 1), node]);
            }
        }
        self.words[fixed as usize..fixed as usize + words.len()].copy_from_slice(&words);
        fixed
    }

    /// Every pointer stored in `ty`, nested aggregates included, as `SHARED`.
    fn pointers_of(&mut self, ty: StructType<'ctx>) -> u32 {
        let fields = (0..ty.count_fields())
            .filter_map(|index| {
                let field_ty = ty.get_field_type_at_index(index)?;
                let node = match field_ty {
                    BasicTypeEnum::PointerType(_) => self.plain(SHARED),
                    BasicTypeEnum::StructType(nested) => self.pointers_of(nested),
                    _ => return None,
                };
                Some((self.offset_of(&ty, index), node))
            })
            .collect();
        self.aggregate(fields)
    }
}
