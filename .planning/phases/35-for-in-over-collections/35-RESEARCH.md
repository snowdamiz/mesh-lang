# Phase 35: For-In over Collections - Research

**Researched:** 2026-02-09
**Domain:** For-in collection iteration, list builder codegen, map/set indexed access, tuple destructuring in loop bindings, comprehension semantics
**Confidence:** HIGH

## Summary

This phase extends `for-in` from range-only iteration (Phase 34) to full collection iteration over List, Map, and Set, while simultaneously adding comprehension semantics where the for-in expression returns a `List<T>` of collected body results. There are five distinct pieces of work: (1) new MIR variants for collection iteration (`ForInList`, `ForInMap`, `ForInSet`), (2) typeck changes to detect collection types and infer element/entry types, (3) list builder pattern for O(N) result collection, (4) break returning the partial list, and (5) new runtime functions for indexed map/set access.

The existing codebase provides strong foundations. Phase 34 established the four-block loop structure (header/body/latch/merge), the `loop_stack` pattern with continue-to-latch semantics, and the `emit_reduction_check()` placement in the latch block. The runtime already has `snow_list_length`, `snow_list_get`, `snow_map_size`, `snow_map_keys`/`snow_map_values`, `snow_set_size`, and `snow_list_append`/`snow_list_new`/`snow_list_from_array` for list building. Collection types are opaque pointers (`MirType::Ptr`) at the LLVM level, with all element storage as uniform `u64` values.

The primary design challenge is the comprehension return semantics combined with break. The for-in must build a result list incrementally -- calling `snow_list_append` in each iteration -- and `break` must return the partially-built list. This requires the result list pointer to live in an alloca visible to both the loop body and the break/merge logic. Since `codegen_break` is a generic function that doesn't know about collection iteration context, we need a mechanism to finalize the result list at the merge block, using a phi node or an alloca that holds the current list pointer. The alloca approach is simpler and consistent with the existing alloca+mem2reg pattern.

**Primary recommendation:** Add `ForInList`, `ForInMap`, `ForInSet` MIR variants alongside the existing `ForInRange`. Each variant carries the collection expression, binding name(s), body, and result element type. In typeck, detect the iterable's type (List<T>, Map<K,V>, Set<T>) and bind the loop variable(s) accordingly. In codegen, use indexed iteration (counter from 0 to length) with runtime calls to get elements. Build the result list incrementally with `snow_list_new` + `snow_list_append` in an alloca, and return the list from the merge block. Also change `ForInRange` to use comprehension semantics (return `List<T>` instead of Unit).

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| inkwell | 0.8.0 | LLVM alloca, basic blocks, branches, calls to runtime | Already in workspace |
| snow-rt | workspace | snow_list_*, snow_map_*, snow_set_* runtime functions | All collection APIs exist |
| snow-parser | workspace | FOR_IN_EXPR already exists; needs destructuring binding support | Phase 34 added this |
| snow-typeck | workspace | infer_for_in needs collection type detection | Phase 34 added range-only path |
| snow-codegen | workspace | MIR variants, lowering, LLVM codegen | Phase 34 established ForInRange pattern |
| snow-fmt | workspace | walk_for_in_expr already exists from Phase 34 | No changes needed for collections |

### Not Needed
No new external dependencies. Two new runtime functions needed (`snow_map_entry_key_at`, `snow_map_entry_value_at` or equivalent indexed access for maps; `snow_set_element_at` for sets).

## Architecture Patterns

### Pipeline Flow for For-In over Collections

```
Source: for x in my_list do x * 2 end

1. Lexer:    [For, Ident(x), In, Ident(my_list), Do, ..., End]
2. Parser:   FOR_IN_EXPR { binding: NAME(x), iterable: NAME_REF(my_list), body: BLOCK }
3. AST:      ForInExpr { binding_name(), iterable(), body() }
4. Typeck:   - Iterable type is List<Int> (from type env)
             - Bind loop var `x` as Int (element type)
             - Body type: Int (x * 2)
             - For-in result: List<Int> (comprehension)
             - enter_loop()/exit_loop() for break/continue
5. MIR:      MirExpr::ForInList { var: "x", collection: Var("my_list"), body: BinOp(Mul, Var("x"), 2), elem_ty: Int, ty: Ptr }
6. Codegen:  - result_list = snow_list_new()
             - len = snow_list_length(collection)
             - counter = alloca i64 = 0
             - header: counter < len? -> body/merge
             - body: elem = snow_list_get(collection, counter)
                     bind x = elem
                     body_val = codegen body
                     result_list = snow_list_append(result_list, body_val)
             - latch: counter++ + reduction check
             - merge: return result_list
```

### For-In over Map (Destructuring)

```
Source: for {k, v} in my_map do process(k, v) end

1. Parser:   FOR_IN_EXPR { binding: TUPLE_PAT({k, v}), iterable: NAME_REF(my_map), body: BLOCK }
             ** NOTE: Need to extend parser to accept {k, v} or (k, v) as binding
2. Typeck:   - Iterable type is Map<String, Int>
             - Bind k as String, v as Int in body scope
             - Body type: T (whatever process returns)
             - For-in result: List<T>
3. MIR:      MirExpr::ForInMap { key_var: "k", val_var: "v", collection: ..., body: ..., key_ty: String, val_ty: Int, ty: Ptr }
4. Codegen:  - result_list = snow_list_new()
             - len = snow_map_size(collection)
             - counter = alloca i64 = 0
             - header: counter < len? -> body/merge
             - body: key = snow_map_entry_key_at(collection, counter)
                     val = snow_map_entry_value_at(collection, counter)
                     bind k = key, v = val
                     body_val = codegen body
                     result_list = snow_list_append(result_list, body_val)
             - latch: counter++ + reduction check
             - merge: return result_list
```

### LLVM Basic Block Structure for For-In Collection

```
entry:
  %collection = <codegen collection_expr>       ; e.g., list/map/set pointer
  %len = call i64 @snow_list_length(%collection) ; or snow_map_size/snow_set_size
  %counter = alloca i64
  store i64 0, %counter
  %result_list = alloca ptr                      ; holds current result list
  %init_list = call ptr @snow_list_new()
  store ptr %init_list, %result_list
  br forin_header

forin_header:
  %i = load i64, %counter
  %cond = icmp slt i64 %i, %len
  br i1 %cond, forin_body, forin_merge

forin_body:
  ; Get element: snow_list_get(collection, %i) / snow_set_element_at / snow_map_entry_*
  %elem = call i64 @snow_list_get(%collection, %i)
  ; Bind loop variable
  ; Codegen body
  %body_val = <codegen body>
  ; Convert body_val to u64 for list storage
  %body_u64 = <convert_to_list_element body_val>
  ; Append to result list
  %cur_list = load ptr, %result_list
  %new_list = call ptr @snow_list_append(%cur_list, %body_u64)
  store ptr %new_list, %result_list
  ; Fall through to latch (if not terminated by break/continue)
  br forin_latch

forin_latch:
  %i_cur = load i64, %counter
  %i_next = add i64 %i_cur, 1
  store i64 %i_next, %counter
  call void @snow_reduction_check()
  br forin_header

forin_merge:
  %final_list = load ptr, %result_list           ; partial list on break, full list on normal exit
  ; return %final_list as the for-in expression result
```

### Key Design: Result List as Alloca

The result list pointer is stored in an alloca (`%result_list`). This is critical because:
1. The body appends to it each iteration (load, append, store back)
2. On `break`, the existing `codegen_break` jumps to merge_bb. The merge block loads `%result_list` which contains whatever was accumulated before break.
3. On normal loop completion (header condition false), merge also loads `%result_list` which contains the fully collected list.

This means `break` automatically returns the partial list without any changes to `codegen_break` -- the result alloca already holds the partial list when break fires.

### Pattern: Destructuring Binding for Map Iteration (FORIN-03)

The requirement `for {k, v} in map do body end` needs destructuring bindings. Two approaches:

**Approach A: Extend parser to accept `{k, v}` as a special for-in binding.**
This uses curly braces which currently conflict with struct literal syntax (in postfix position after an identifier). However, in `for` context, `{` immediately after `for` is unambiguous since `for` can only be followed by a binding, not an expression.

**Approach B: Use `(k, v)` tuple destructuring instead.**
Tuples use parentheses in Snow: `(k, v)`. The tuple pattern (`TUPLE_PAT`) already exists in the parser. This would make the syntax `for (k, v) in map do body end`. However, the requirement explicitly says `{k, v}`.

**Recommendation: Support `{k, v}` as specified.** The parser change is localized to `parse_for_in_expr` -- after `for`, if we see `{`, parse a comma-separated list of identifiers until `}`. Create a new CST node `DESTRUCTURE_BINDING` or reuse the existing `TUPLE_PAT` parsing logic adapted for curly braces. Since this only occurs in for-in binding position, there's no ambiguity.

### Pattern: Changing ForInRange to Also Return List<T>

Phase 34's `ForInRange` returns `Unit`. Phase 35 requirement FORIN-05 says ALL for-in loops return `List<T>`. This means we need to update `ForInRange` as well to build a result list. The same alloca+append pattern works for range iteration too.

### Anti-Patterns to Avoid

- **Do NOT use `snow_list_append` in a loop without an alloca for the list pointer.** Each `snow_list_append` returns a NEW list (immutable semantics). The result must be stored back to the alloca.
- **Do NOT use O(N^2) approach for building the result.** `snow_list_append` already copies the existing list. This is O(N) per call, so N calls = O(N^2) total. However, this is the simplest approach and matches the requirement RTIM-02 for "list builder". A true O(N) builder would pre-allocate capacity, but that requires knowing the final size, which we DO know (it's the collection length). See the "List Builder Strategy" section below.
- **Do NOT change `codegen_break` to handle collection-specific logic.** The result alloca pattern means break doesn't need any changes -- it jumps to merge, merge loads the alloca, which holds the partial list.
- **Do NOT iterate maps by calling `snow_map_keys()` + `snow_map_values()` to get lists then iterating those.** That creates two intermediate lists. Instead, add indexed access functions to the runtime.

## List Builder Strategy (RTIM-02)

The requirement says "O(N) list builder allocation, not O(N^2) append chains."

### Analysis of snow_list_append Cost

Each `snow_list_append` call:
1. Allocates a new GC buffer of size `old_len + 1`
2. Copies all `old_len` elements from the old list
3. Appends the new element
4. Total: O(old_len) per call

If we call `snow_list_append` N times in a loop: 1 + 2 + 3 + ... + N = O(N^2) total copies.

### Solution: Pre-allocate + Fill via Runtime Function

Add a new runtime function `snow_list_builder_new(capacity: i64) -> *mut u8` that allocates a list with the given capacity and length 0. Then add `snow_list_builder_push(list: *mut u8, element: u64)` that writes to the data region at the current length and increments length in-place. This is O(1) per push.

However, this breaks the immutability invariant of `SnowList` -- `snow_list_builder_push` mutates in-place. This is safe ONLY during construction before the list is returned to user code. The list builder is an internal codegen detail, not exposed to user programs.

**Alternative simpler approach:** Use a stack-allocated array (like `codegen_list_lit` does with `snow_list_from_array`). But we don't know the final size at compile time since `break` can terminate early.

**Recommended approach:** Add `snow_list_builder_new(cap)` and `snow_list_builder_push(list, elem)` to the runtime. The builder allocates once with full capacity, pushes are O(1), and the result is a valid SnowList when done. For break, the list's length field already reflects only the elements pushed so far.

```rust
// In snow-rt/src/collections/list.rs:

/// Create a list with pre-allocated capacity for N elements.
/// Length starts at 0. Used by for-in codegen for O(N) result building.
#[no_mangle]
pub extern "C" fn snow_list_builder_new(capacity: i64) -> *mut u8 {
    unsafe { alloc_list(capacity.max(0) as u64) }
}

/// Push an element to a list builder (in-place mutation).
/// SAFETY: Only valid during construction -- the list must not be shared yet.
/// Increments len and writes element at data[len].
#[no_mangle]
pub extern "C" fn snow_list_builder_push(list: *mut u8, element: u64) {
    unsafe {
        let len = list_len(list) as usize;
        let data = list_data_mut(list);
        *data.add(len) = element;
        *(list as *mut u64) = (len + 1) as u64; // increment length
    }
}
```

### LLVM Codegen with List Builder

```
entry:
  %len = call i64 @snow_list_length(%collection)  ; or snow_map_size/snow_set_size
  %result = call ptr @snow_list_builder_new(%len)  ; single allocation
  ...
forin_body:
  ...
  %body_u64 = <convert body_val to u64>
  call void @snow_list_builder_push(%result, %body_u64)  ; O(1) push, in-place
  ...
forin_merge:
  ; %result already has correct len (number of elements pushed)
  ; On break, len < capacity, which is fine -- extra capacity is harmless
```

This is O(N) total: one allocation + N O(1) pushes. On break, the list has len = number of iterations completed, which is correct for BRKC-03.

## New Runtime Functions Needed

### For Map Iteration (Indexed Access)

The map stores entries as `[u64; 2]` pairs in a contiguous buffer. We need indexed access:

```rust
/// Get the key at index i. Panics if out of bounds.
#[no_mangle]
pub extern "C" fn snow_map_entry_key(map: *mut u8, index: i64) -> u64 {
    unsafe {
        let len = map_len(map);
        if index < 0 || index as u64 >= len {
            panic!("snow_map_entry_key: index {} out of bounds (len {})", index, len);
        }
        let entries = map_entries(map);
        (*entries.add(index as usize))[0]
    }
}

/// Get the value at index i. Panics if out of bounds.
#[no_mangle]
pub extern "C" fn snow_map_entry_value(map: *mut u8, index: i64) -> u64 {
    unsafe {
        let len = map_len(map);
        if index < 0 || index as u64 >= len {
            panic!("snow_map_entry_value: index {} out of bounds (len {})", index, len);
        }
        let entries = map_entries(map);
        (*entries.add(index as usize))[1]
    }
}
```

### For Set Iteration (Indexed Access)

The set stores elements as `u64` values in a contiguous buffer. We need indexed access:

```rust
/// Get the element at index i. Panics if out of bounds.
#[no_mangle]
pub extern "C" fn snow_set_element_at(set: *mut u8, index: i64) -> u64 {
    unsafe {
        let len = set_len(set);
        if index < 0 || index as u64 >= len {
            panic!("snow_set_element_at: index {} out of bounds (len {})", index, len);
        }
        let data = set_data(set);
        *data.add(index as usize)
    }
}
```

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| O(N) list building | N calls to `snow_list_append` | `snow_list_builder_new` + `snow_list_builder_push` | Append is O(N) per call = O(N^2) total; builder push is O(1) |
| Map indexed access | Extracting keys list + list_get | `snow_map_entry_key` / `snow_map_entry_value` | Avoids creating intermediate list |
| Set indexed access | Converting set to list first | `snow_set_element_at` | Avoids creating intermediate list |
| Break partial list | Custom break handler per loop type | Result alloca pattern | Break jumps to merge; merge loads alloca which has partial list |
| Loop counter management | Manual phi nodes | alloca + mem2reg pattern | Consistent with existing if/while/for-in-range codegen |
| Reduction counting | Per-loop counter | Existing `emit_reduction_check()` | Already handles thread-local counter, coroutine yielding |

## Common Pitfalls

### Pitfall 1: O(N^2) List Building with snow_list_append
**What goes wrong:** Using `snow_list_append` in a loop creates N copies of increasing size, totaling O(N^2) work.
**Why it happens:** `snow_list_append` is immutable -- it creates a new list with all old elements plus the new one. Each call copies old_len elements.
**How to avoid:** Use the list builder pattern: `snow_list_builder_new(capacity)` pre-allocates, `snow_list_builder_push` writes in-place at O(1).
**Warning signs:** For-in loops over large collections are noticeably slow.

### Pitfall 2: Break Not Returning Partial List
**What goes wrong:** `break` inside `for x in list do ... end` returns Unit instead of the partial list.
**Why it happens:** Using a separate code path for break that doesn't access the result alloca.
**How to avoid:** The result list pointer lives in an alloca. Both normal completion and break arrive at the merge block, which loads from the alloca. The alloca holds whatever was accumulated up to that point. No special break handling needed.
**Warning signs:** Break returns wrong type or empty list.

### Pitfall 3: Collection Type Detection at Wrong Level
**What goes wrong:** Trying to detect List/Map/Set types in the parser or MIR lowerer instead of typeck.
**Why it happens:** The parser only sees syntax, not types. The iterable `x` in `for i in x do...end` could be any expression.
**How to avoid:** Detect collection types in typeck using the resolved type from `infer_expr`. Then store the type information (is it List<T>? Map<K,V>? Set<T>?) in the types map. The MIR lowerer reads this information to choose the right MIR variant.
**Warning signs:** Wrong MIR variant selected for an expression whose type is only known after inference.

### Pitfall 4: Map Destructuring {k, v} Conflicts with Struct Literal
**What goes wrong:** Parser confuses `{k, v}` with a struct literal body.
**Why it happens:** `{` in expression position (after an identifier) starts a struct literal.
**How to avoid:** In `parse_for_in_expr`, `{` comes directly after `for` keyword (not after an identifier in expression position). Parse the destructuring binding as a special case within `parse_for_in_expr` only. The general expression parser never sees this `{`.
**Warning signs:** Parse errors on `for {k, v} in map do...end`.

### Pitfall 5: Converting Body Value to u64 for List Storage
**What goes wrong:** Body value is a pointer (String, Struct) but stored directly as u64, or vice versa.
**Why it happens:** SnowList stores all elements as u64. Pointers need `ptrtoint`, floats need `bitcast`, bools need `zext`.
**How to avoid:** Use the existing `convert_to_list_element` function (already implemented in `codegen_list_lit`). It handles all type conversions correctly.
**Warning signs:** LLVM type errors, segfaults when reading list elements back.

### Pitfall 6: ForInRange Also Needs Comprehension Semantics
**What goes wrong:** Only collection for-in returns List<T>, but `for i in 0..5 do i * 2 end` still returns Unit.
**Why it happens:** Forgetting that FORIN-05 applies to ALL for-in, including range.
**How to avoid:** Update `codegen_for_in_range` to also use the list builder pattern and return `List<T>`.
**Warning signs:** Range for-in returns Unit while collection for-in returns List.

### Pitfall 7: Empty Collection Returns Wrong Result
**What goes wrong:** For-in over empty list/map/set crashes or returns the wrong type.
**Why it happens:** Edge case in list builder: `snow_list_builder_new(0)` creates an empty list. The header check (0 < 0) is false, so the loop never executes. The merge block loads the result alloca which holds the empty list. This should work correctly.
**How to avoid:** Ensure `snow_list_builder_new(0)` creates a valid empty list (it does -- `alloc_list(0)` sets len=0, cap=0). Verify with tests.
**Warning signs:** Crash or panic when iterating empty collections.

## Code Examples

### Typeck: infer_for_in (Extended for Collections)

```rust
fn infer_for_in(
    ctx: &mut InferCtx,
    env: &mut TypeEnv,
    for_in: &ForInExpr,
    types: &mut FxHashMap<TextRange, Ty>,
    type_registry: &TypeRegistry,
    trait_registry: &TraitRegistry,
    fn_constraints: &FxHashMap<String, FnConstraints>,
) -> Result<Ty, TypeError> {
    // Infer the iterable expression.
    let iter_ty = if let Some(iterable) = for_in.iterable() {
        let ty = infer_expr(ctx, env, &iterable, types, type_registry, trait_registry, fn_constraints)?;
        ctx.apply(ty)
    } else {
        return Ok(Ty::Tuple(vec![])); // Error: no iterable
    };

    // Determine element type based on iterable type.
    let (var_name, elem_ty) = determine_binding_and_elem_type(for_in, &iter_ty, ctx)?;

    // Push scope, bind loop variable(s).
    env.push_scope();
    // For map: bind both key and value variables
    // For list/set/range: bind single variable
    match &elem_ty {
        BindingType::Single(name, ty) => {
            env.insert(name.clone(), Scheme::mono(ty.clone()));
        }
        BindingType::MapEntry(key_name, key_ty, val_name, val_ty) => {
            env.insert(key_name.clone(), Scheme::mono(key_ty.clone()));
            env.insert(val_name.clone(), Scheme::mono(val_ty.clone()));
        }
    }

    ctx.enter_loop();

    // Infer body -- its type becomes the List element type.
    let body_ty = if let Some(body) = for_in.body() {
        infer_block(ctx, env, &body, types, type_registry, trait_registry, fn_constraints)?
    } else {
        Ty::Tuple(vec![])
    };

    ctx.exit_loop();
    env.pop_scope();

    // For-in returns List<body_ty> (comprehension semantics).
    Ok(Ty::list(body_ty))
}
```

### MIR: ForInList, ForInMap, ForInSet variants

```rust
/// For-in loop over a List: `for var in list do body end`.
/// Desugared to indexed iteration with list builder.
ForInList {
    var: String,
    collection: Box<MirExpr>,
    body: Box<MirExpr>,
    elem_ty: MirType,    // element type (for conversion)
    body_ty: MirType,    // body result type (for list element conversion)
    ty: MirType,         // always Ptr (List pointer)
},

/// For-in loop over a Map: `for {k, v} in map do body end`.
ForInMap {
    key_var: String,
    val_var: String,
    collection: Box<MirExpr>,
    body: Box<MirExpr>,
    key_ty: MirType,
    val_ty: MirType,
    body_ty: MirType,
    ty: MirType,         // always Ptr (List pointer)
},

/// For-in loop over a Set: `for var in set do body end`.
ForInSet {
    var: String,
    collection: Box<MirExpr>,
    body: Box<MirExpr>,
    elem_ty: MirType,
    body_ty: MirType,
    ty: MirType,         // always Ptr (List pointer)
},
```

### Runtime: List Builder Functions

```rust
/// Create a list with pre-allocated capacity. Length starts at 0.
#[no_mangle]
pub extern "C" fn snow_list_builder_new(capacity: i64) -> *mut u8 {
    unsafe { alloc_list(capacity.max(0) as u64) }
}

/// Push an element to a list builder (in-place mutation, O(1)).
/// SAFETY: Only valid during construction before the list is shared.
#[no_mangle]
pub extern "C" fn snow_list_builder_push(list: *mut u8, element: u64) {
    unsafe {
        let len = list_len(list) as usize;
        let data = list_data_mut(list);
        *data.add(len) = element;
        *(list as *mut u64) = (len + 1) as u64;
    }
}
```

### Codegen: codegen_for_in_list (Sketch)

```rust
fn codegen_for_in_list(
    &mut self,
    var: &str,
    collection: &MirExpr,
    body: &MirExpr,
    elem_ty: &MirType,
    body_ty: &MirType,
    _ty: &MirType,
) -> Result<BasicValueEnum<'ctx>, String> {
    let fn_val = self.current_function();
    let i64_ty = self.context.i64_type();
    let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());

    // 1. Codegen collection expression.
    let collection_val = self.codegen_expr(collection)?.into_pointer_value();

    // 2. Get collection length.
    let len_fn = get_intrinsic(&self.module, "snow_list_length");
    let len = self.builder.build_call(len_fn, &[collection_val.into()], "len")?
        .try_as_basic_value().left().unwrap().into_int_value();

    // 3. Create result list builder with capacity = len.
    let builder_new_fn = get_intrinsic(&self.module, "snow_list_builder_new");
    let result_list_init = self.builder.build_call(builder_new_fn, &[len.into()], "result")?
        .try_as_basic_value().left().unwrap().into_pointer_value();

    // 4. Alloca for result list pointer (so break can access partial result).
    let result_alloca = self.builder.build_alloca(ptr_ty, "result_list")?;
    self.builder.build_store(result_alloca, result_list_init)?;

    // 5. Counter alloca.
    let counter = self.builder.build_alloca(i64_ty, "forin_i")?;
    self.builder.build_store(counter, i64_ty.const_int(0, false))?;

    // 6. Create four basic blocks.
    let header_bb = self.context.append_basic_block(fn_val, "forin_header");
    let body_bb = self.context.append_basic_block(fn_val, "forin_body");
    let latch_bb = self.context.append_basic_block(fn_val, "forin_latch");
    let merge_bb = self.context.append_basic_block(fn_val, "forin_merge");

    self.loop_stack.push((latch_bb, merge_bb));
    self.builder.build_unconditional_branch(header_bb)?;

    // Header: check counter < len.
    self.builder.position_at_end(header_bb);
    let i_val = self.builder.build_load(i64_ty, counter, "i")?.into_int_value();
    let cond = self.builder.build_int_compare(IntPredicate::SLT, i_val, len, "cond")?;
    self.builder.build_conditional_branch(cond, body_bb, merge_bb)?;

    // Body: get element, bind variable, codegen body, append to result.
    self.builder.position_at_end(body_bb);
    let get_fn = get_intrinsic(&self.module, "snow_list_get");
    let elem = self.builder.build_call(get_fn, &[collection_val.into(), i_val.into()], "elem")?
        .try_as_basic_value().left().unwrap();

    // Convert elem from u64 to the proper type (e.g., inttoptr for strings).
    let typed_elem = self.convert_from_list_element(elem, elem_ty)?;

    // Save/bind loop variable.
    let elem_alloca = self.builder.build_alloca(/* type for elem_ty */, var)?;
    self.builder.build_store(elem_alloca, typed_elem)?;
    let old_alloca = self.locals.insert(var.to_string(), elem_alloca);
    let old_type = self.local_types.insert(var.to_string(), elem_ty.clone());

    // Codegen body.
    let body_val = self.codegen_expr(body)?;

    // Append body_val to result list.
    if let Some(bb) = self.builder.get_insert_block() {
        if bb.get_terminator().is_none() {
            let body_as_u64 = self.convert_to_list_element(body_val, body_ty)?;
            let push_fn = get_intrinsic(&self.module, "snow_list_builder_push");
            let cur_result = self.builder.build_load(ptr_ty, result_alloca, "cur_result")?;
            self.builder.build_call(push_fn, &[cur_result.into(), body_as_u64.into()], "")?;
            self.builder.build_unconditional_branch(latch_bb)?;
        }
    }

    // Latch: increment counter, reduction check, branch to header.
    self.builder.position_at_end(latch_bb);
    let cur = self.builder.build_load(i64_ty, counter, "i_cur")?.into_int_value();
    let next = self.builder.build_int_add(cur, i64_ty.const_int(1, false), "i_next")?;
    self.builder.build_store(counter, next)?;
    self.emit_reduction_check();
    self.builder.build_unconditional_branch(header_bb)?;

    // Cleanup.
    self.loop_stack.pop();
    // Restore locals...

    // Merge: return the result list.
    self.builder.position_at_end(merge_bb);
    let final_result = self.builder.build_load(ptr_ty, result_alloca, "final_list")?;
    Ok(final_result)
}
```

### Element Type Conversion (from u64 to typed value)

```rust
/// Convert a u64 list element back to a typed value.
/// This is the inverse of convert_to_list_element.
fn convert_from_list_element(
    &mut self,
    val: BasicValueEnum<'ctx>,
    target_ty: &MirType,
) -> Result<BasicValueEnum<'ctx>, String> {
    let i64_val = val.into_int_value();
    match target_ty {
        MirType::Int => Ok(i64_val.into()),
        MirType::Bool => {
            let truncated = self.builder.build_int_truncate(
                i64_val, self.context.bool_type(), "i64_to_bool"
            ).map_err(|e| e.to_string())?;
            Ok(truncated.into())
        }
        MirType::Float => {
            let cast = self.builder.build_bit_cast(
                i64_val, self.context.f64_type(), "i64_to_float"
            ).map_err(|e| e.to_string())?;
            Ok(cast)
        }
        MirType::String | MirType::Ptr | MirType::Struct(_) | MirType::SumType(_)
        | MirType::Pid(_) | MirType::Closure(_, _) | MirType::FnPtr(_, _) => {
            let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
            let ptr = self.builder.build_int_to_ptr(
                i64_val, ptr_ty, "i64_to_ptr"
            ).map_err(|e| e.to_string())?;
            Ok(ptr.into())
        }
        MirType::Unit => {
            Ok(self.context.struct_type(&[], false).const_zero().into())
        }
        _ => Ok(i64_val.into()),
    }
}
```

## MIR Lowering: Detecting Collection Type

The MIR lowerer needs to know whether the iterable is a range, list, map, or set. This information comes from the typeck results.

```rust
fn lower_for_in_expr(&mut self, for_in: &ForInExpr) -> MirExpr {
    let var_name = for_in.binding_name()
        .and_then(|n| n.text())
        .unwrap_or_else(|| "_".to_string());

    // Get the type of the iterable from typeck results.
    let iterable_ty = for_in.iterable()
        .and_then(|e| self.get_ty(e.syntax().text_range()))
        .cloned();

    // Check if iterable is a DotDot range (keep existing ForInRange behavior).
    if let Some(Expr::BinaryExpr(bin)) = for_in.iterable().as_ref() {
        if bin.op().map(|t| t.kind()) == Some(SyntaxKind::DOT_DOT) {
            return self.lower_for_in_range(for_in, &var_name);
        }
    }

    // Determine collection type from iterable_ty.
    match &iterable_ty {
        Some(ty) if is_list_type(ty) => self.lower_for_in_list(for_in, &var_name, ty),
        Some(ty) if is_map_type(ty) => self.lower_for_in_map(for_in, ty),
        Some(ty) if is_set_type(ty) => self.lower_for_in_set(for_in, &var_name, ty),
        _ => {
            // Fallback: treat as list iteration.
            // This handles cases where the type is not fully resolved.
            let collection = for_in.iterable()
                .map(|e| self.lower_expr(&e))
                .unwrap_or(MirExpr::Unit);
            // ... build ForInList with best-effort types
        }
    }
}
```

## Parser Changes for {k, v} Destructuring

The parser's `parse_for_in_expr` currently only accepts a single `IDENT` as the binding. For map destructuring, extend it to also accept `{ ident, ident }`:

```rust
fn parse_for_in_expr(p: &mut Parser) -> MarkClosed {
    let m = p.open();
    p.advance(); // FOR_KW

    // Parse binding: either a single IDENT (NAME) or {k, v} (DESTRUCTURE_BINDING).
    if p.at(SyntaxKind::L_BRACE) {
        // Destructuring binding: {k, v}
        let dm = p.open();
        p.advance(); // {
        // Parse comma-separated identifiers.
        if p.at(SyntaxKind::IDENT) {
            let n = p.open();
            p.advance();
            p.close(n, SyntaxKind::NAME);
        }
        while p.eat(SyntaxKind::COMMA) {
            if p.at(SyntaxKind::R_BRACE) { break; }
            if p.at(SyntaxKind::IDENT) {
                let n = p.open();
                p.advance();
                p.close(n, SyntaxKind::NAME);
            }
        }
        p.expect(SyntaxKind::R_BRACE);
        p.close(dm, SyntaxKind::DESTRUCTURE_BINDING);
    } else if p.at(SyntaxKind::IDENT) {
        // Single binding: x
        let name = p.open();
        p.advance();
        p.close(name, SyntaxKind::NAME);
    } else {
        p.error("expected loop variable name or {key, value} destructuring after `for`");
    }

    // ... rest unchanged
}
```

The AST `ForInExpr` needs a new accessor for the destructuring case:

```rust
impl ForInExpr {
    /// Simple binding name (NAME child).
    pub fn binding_name(&self) -> Option<Name> { ... }

    /// Destructured binding names (DESTRUCTURE_BINDING child).
    pub fn destructure_binding(&self) -> Option<DestructureBinding> {
        child_node(&self.syntax)
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `List.map(list, fn x -> x * 2 end)` | `for x in list do x * 2 end` | Phase 35 | Natural comprehension syntax |
| Manual Map.keys + List iteration | `for {k, v} in map do ... end` | Phase 35 | Direct map entry iteration |
| No Set iteration | `for x in set do ... end` | Phase 35 | Set traversal with for-in |
| For-in returns Unit | For-in returns `List<T>` | Phase 35 | Expression semantics, list comprehensions |
| O(N^2) append chains | O(N) list builder | Phase 35 | Performance for large collections |

## Key Design Decisions

### 1. Three New MIR Variants vs. Single Generic ForInCollection

Using three separate variants (`ForInList`, `ForInMap`, `ForInSet`) instead of one generic `ForInCollection` with a discriminator field. Rationale: each has different numbers of binding variables (1 for list/set, 2 for map), different runtime calls (list_get vs map_entry_key/value vs set_element_at), and different element types. Separate variants make the codegen cleaner and each match arm self-contained.

### 2. Comprehension Semantics for All For-In (Including Range)

FORIN-05 requires ALL for-in expressions return `List<T>`. This includes the existing `ForInRange`. We need to update `codegen_for_in_range` from Phase 34 to also use the list builder pattern and return a list instead of Unit.

### 3. List Builder with In-Place Mutation

Using `snow_list_builder_new(cap)` + `snow_list_builder_push(list, elem)` for O(N) construction. The in-place mutation is safe because the list is not yet shared -- it's still being constructed by the codegen. Once returned from the for-in expression, it becomes immutable (no code path mutates it after).

### 4. Break Returns Partial List via Alloca

The result list pointer lives in an alloca. On break, the existing `codegen_break` jumps to merge_bb. The merge block loads the alloca, which contains whatever was accumulated. The list builder's length field already reflects the number of pushes, so the returned list is valid with `len = iterations_completed`.

### 5. Curly-Brace Destructuring {k, v} for Maps

Using `{k, v}` syntax as specified in the requirements. This is unambiguous in for-in binding position because `for` can only be followed by a binding, not a general expression (which is where `{` would start a struct literal). The parser handles this in `parse_for_in_expr` only.

### 6. Indexed Iteration (Counter-Based)

All collection for-in uses the same four-block pattern as range for-in: a counter from 0 to N, with runtime calls to get elements by index. This reuses the established loop infrastructure (loop_stack, break, continue, reduction check) without modification.

## Open Questions

1. **Should `continue` in a collection for-in skip appending to the result list?**
   - What we know: `continue` jumps to the latch (increment + reduction check). It does NOT execute the append that follows the body codegen.
   - Current behavior: If the body contains `if cond do continue end; expr`, then when `continue` fires, the `snow_list_builder_push` call (which comes after the body codegen) is skipped because the block was already terminated by `continue`'s branch. The element is NOT added to the result. The counter still increments.
   - Recommendation: This seems correct -- `continue` means "skip this element" in comprehension terms. However, this means the result list may be shorter than the collection length, and the list builder was pre-allocated with full capacity. The list has len < cap, which is fine -- extra capacity is harmless.

2. **Should ForInRange return List<Int> or List<T> where T is body type?**
   - The body of `for i in 0..5 do i * 2 end` evaluates to Int (since i * 2 is Int).
   - The body of `for i in 0..5 do to_string(i) end` evaluates to String.
   - The result type should be `List<body_type>`, not `List<Int>`.
   - Recommendation: Use `List<body_type>` as the return type. The element type T in the result list is determined by the body's type, not the iterable's element type.

3. **What about for-in over a Range variable (not literal ..)?**
   - E.g., `let r = Range.new(0, 10); for i in r do ... end`
   - Phase 34 deferred this. Phase 35 could handle it by detecting `Range` type in the iterable and using `snow_range_to_list` then iterating, or by reading start/end fields from the Range struct.
   - Recommendation: Defer. Focus on List, Map, Set for Phase 35. Range variable iteration can be a future enhancement.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `snow-rt/src/collections/list.rs` -- SnowList layout (len, cap, data), alloc_list, snow_list_new, snow_list_length, snow_list_get, snow_list_from_array, snow_list_append
- Codebase analysis: `snow-rt/src/collections/map.rs` -- SnowMap layout (len, cap|key_type, entries[key,val]), snow_map_size, snow_map_keys, snow_map_values; NO indexed entry access exists
- Codebase analysis: `snow-rt/src/collections/set.rs` -- SnowSet layout (len, cap, data), snow_set_size; NO indexed element access exists
- Codebase analysis: `snow-codegen/src/codegen/expr.rs` -- codegen_for_in_range (line 1726), codegen_break (1825), codegen_continue (1840), codegen_list_lit (2670), convert_to_list_element (2716)
- Codebase analysis: `snow-codegen/src/mir/mod.rs` -- MirExpr::ForInRange (line 306), MirType enum (no List/Map/Set types -- all are Ptr)
- Codebase analysis: `snow-codegen/src/mir/lower.rs` -- lower_for_in_expr (line 3977), extract_list_elem_type (line 36)
- Codebase analysis: `snow-codegen/src/mir/types.rs` -- resolve_type maps List/Map/Set to MirType::Ptr (line 76, 102)
- Codebase analysis: `snow-typeck/src/infer.rs` -- infer_for_in (line 3176), currently returns Ty::Tuple(vec![]) (Unit)
- Codebase analysis: `snow-typeck/src/ty.rs` -- Ty::list(inner), Ty::map(key,val), Ty::set(inner) constructors
- Codebase analysis: `snow-codegen/src/codegen/intrinsics.rs` -- snow_list_builder_new/push NOT yet declared; snow_list_new, snow_list_append, snow_list_get, snow_map_size, snow_set_size ARE declared
- Codebase analysis: `snow-parser/src/parser/expressions.rs` -- parse_for_in_expr (line 1191), currently only accepts single IDENT binding
- Codebase analysis: `snow-parser/src/ast/expr.rs` -- ForInExpr with binding_name(), iterable(), body()
- Codebase analysis: Phase 34 research and summaries -- four-block pattern, latch-based continue, alloca+mem2reg, loop_stack

### Secondary (MEDIUM confidence)
- LLVM alloca+mem2reg: verified by existing codegen patterns (if-expression, while, for-in-range all use allocas)
- O(N) list builder approach: follows standard compiler optimization pattern for known-size array construction

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all libraries already in workspace, runtime functions inspected
- Architecture: HIGH -- builds directly on Phase 34's verified four-block pattern with well-understood extensions
- Runtime additions: HIGH -- SnowList/SnowMap/SnowSet layouts inspected, indexed access functions are trivial
- Pitfalls: HIGH -- each identified from actual code inspection and runtime semantics
- Code examples: HIGH -- modeled on actual existing codegen (codegen_for_in_range, codegen_list_lit)
- Parser changes: MEDIUM -- {k,v} destructuring is a new pattern, but localized to parse_for_in_expr

**Research date:** 2026-02-09
**Valid until:** indefinite (codebase-specific research, not library-version-dependent)
