//! LLVM IR generation from MIR.
//!
//! This module implements the core code generation pass that transforms
//! desugared, monomorphized MIR into LLVM IR using Inkwell 0.8.0.
//!
//! ## Architecture
//!
//! - [`CodeGen`]: Main codegen struct holding LLVM context, module, builder
//! - [`types`]: MirType to LLVM type mapping
//! - [`intrinsics`]: Runtime function declarations
//! - [`expr`]: Expression codegen (implemented in Task 2)
//! - [`pattern`]: Decision tree codegen (implemented in Task 2)

pub mod expr;
pub mod intrinsics;
pub mod pattern;
pub mod types;

use std::path::Path;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple,
};
use inkwell::types::StructType;
use inkwell::values::{FunctionValue, PointerValue};
use inkwell::OptimizationLevel;
use rustc_hash::FxHashMap;

use crate::mir::{MirFunction, MirModule, MirStructDef, MirSumTypeDef, MirType};

use self::types::{create_sum_type_layout, llvm_closure_fn_type, llvm_fn_type, llvm_type};

// ── CodeGen ──────────────────────────────────────────────────────────

/// The main LLVM code generation context.
///
/// Holds the LLVM context, module, builder, and all cached type/function
/// mappings needed during IR generation.
pub struct CodeGen<'ctx> {
    /// The LLVM context (lifetime anchor for all LLVM values).
    pub(crate) context: &'ctx Context,
    /// The LLVM module being built.
    pub(crate) module: Module<'ctx>,
    /// The LLVM IR builder.
    pub(crate) builder: Builder<'ctx>,
    /// The target machine (for optimization and object file emission).
    pub(crate) target_machine: TargetMachine,

    // ── Type caches ──────────────────────────────────────────────────

    /// Cache of named struct types (MIR struct name -> LLVM struct type).
    pub(crate) struct_types: FxHashMap<String, StructType<'ctx>>,
    /// Cache of sum type tagged union layouts (MIR sum type name -> LLVM struct type).
    pub(crate) sum_type_layouts: FxHashMap<String, StructType<'ctx>>,
    /// Cache of sum type definitions for variant lookup.
    pub(crate) sum_type_defs: FxHashMap<String, MirSumTypeDef>,

    // ── Function tracking ────────────────────────────────────────────

    /// Map from MIR function name to LLVM function value.
    pub(crate) functions: FxHashMap<String, FunctionValue<'ctx>>,
    /// The current function being compiled.
    pub(crate) current_fn: Option<FunctionValue<'ctx>>,

    // ── Local variable tracking ──────────────────────────────────────

    /// Local variable allocas in the current function (name -> alloca pointer).
    pub(crate) locals: FxHashMap<String, PointerValue<'ctx>>,
    /// Local variable types (name -> MirType) for loading with correct types.
    pub(crate) local_types: FxHashMap<String, MirType>,

    // ── MIR reference ────────────────────────────────────────────────

    /// The MIR module being compiled (borrowed for arm body lookup).
    pub(crate) mir_functions: Vec<MirFunction>,
}

impl<'ctx> CodeGen<'ctx> {
    /// Create a new CodeGen instance.
    ///
    /// # Arguments
    ///
    /// * `context` - The LLVM context (must outlive the CodeGen)
    /// * `module_name` - Name for the LLVM module
    /// * `opt_level` - Optimization level (0 = none, 2 = default)
    /// * `target_triple` - Optional target triple; None = host default
    ///
    /// # Errors
    ///
    /// Returns an error if target initialization fails or the triple is invalid.
    pub fn new(
        context: &'ctx Context,
        module_name: &str,
        opt_level: u8,
        target_triple: Option<&str>,
    ) -> Result<Self, String> {
        // Initialize target
        Target::initialize_native(&InitializationConfig::default())
            .map_err(|e| format!("Failed to initialize native target: {}", e))?;

        let triple = match target_triple {
            Some(triple_str) => TargetTriple::create(triple_str),
            None => TargetMachine::get_default_triple(),
        };

        let target = Target::from_triple(&triple)
            .map_err(|e| format!("Invalid target triple '{}': {}", triple, e))?;

        let opt = match opt_level {
            0 => OptimizationLevel::None,
            1 => OptimizationLevel::Less,
            _ => OptimizationLevel::Default,
        };

        let target_machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                opt,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| format!("Failed to create target machine for '{}'", triple))?;

        let module = context.create_module(module_name);
        module.set_triple(&triple);

        let builder = context.create_builder();

        Ok(CodeGen {
            context,
            module,
            builder,
            target_machine,
            struct_types: FxHashMap::default(),
            sum_type_layouts: FxHashMap::default(),
            sum_type_defs: FxHashMap::default(),
            functions: FxHashMap::default(),
            current_fn: None,
            locals: FxHashMap::default(),
            local_types: FxHashMap::default(),
            mir_functions: Vec::new(),
        })
    }

    /// Compile a MIR module to LLVM IR.
    ///
    /// This is the main compilation entry point. It:
    /// 1. Declares runtime intrinsics
    /// 2. Creates type layouts for structs and sum types
    /// 3. Forward-declares all functions
    /// 4. Compiles function bodies
    /// 5. Generates a main wrapper (if entry function exists)
    /// 6. Verifies the LLVM module
    pub fn compile(&mut self, mir: &MirModule) -> Result<(), String> {
        // Store MIR functions for arm body lookup during pattern codegen.
        self.mir_functions = mir.functions.clone();

        // Step 1: Declare runtime intrinsics.
        intrinsics::declare_intrinsics(&self.module);

        // Step 2: Create type layouts.
        self.create_struct_types(&mir.structs);
        self.create_sum_type_layouts(&mir.sum_types);

        // Step 3: Forward-declare all functions.
        self.declare_functions(&mir.functions);

        // Step 4: Compile function bodies.
        for func in &mir.functions {
            self.compile_function(func)?;
        }

        // Step 5: Generate main wrapper if entry function exists.
        if let Some(entry_name) = &mir.entry_function {
            self.generate_main_wrapper(entry_name)?;
        }

        // Step 6: Verify the module.
        self.module
            .verify()
            .map_err(|e| format!("LLVM module verification failed: {}", e))?;

        Ok(())
    }

    /// Run LLVM optimization passes on the module.
    pub fn run_optimization_passes(&self, opt_level: u8) -> Result<(), String> {
        let passes = match opt_level {
            0 => "default<O0>",
            1 => "default<O1>",
            _ => "default<O2>",
        };
        self.module
            .run_passes(passes, &self.target_machine, PassBuilderOptions::create())
            .map_err(|e| format!("Optimization passes failed: {}", e))
    }

    /// Emit the LLVM module as an object file.
    pub fn emit_object(&self, path: &Path) -> Result<(), String> {
        self.target_machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| format!("Failed to emit object file: {}", e))
    }

    /// Emit the LLVM module as human-readable LLVM IR (.ll file).
    pub fn emit_llvm_ir(&self, path: &Path) -> Result<(), String> {
        self.module.print_to_file(path).map_err(|e| format!("Failed to emit LLVM IR: {}", e))
    }

    /// Get the LLVM IR as a string (for testing).
    pub fn get_llvm_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    // ── Type layout creation ─────────────────────────────────────────

    fn create_struct_types(&mut self, structs: &[MirStructDef]) {
        for s in structs {
            let field_types: Vec<inkwell::types::BasicTypeEnum<'ctx>> = s
                .fields
                .iter()
                .map(|(_, ty)| llvm_type(self.context, ty, &self.struct_types, &self.sum_type_layouts))
                .collect();
            let struct_ty = self.context.opaque_struct_type(&s.name);
            struct_ty.set_body(&field_types, false);
            self.struct_types.insert(s.name.clone(), struct_ty);
        }
    }

    fn create_sum_type_layouts(&mut self, sum_types: &[MirSumTypeDef]) {
        for st in sum_types {
            let layout = create_sum_type_layout(
                self.context,
                st,
                &self.struct_types,
                &self.sum_type_layouts,
            );
            self.sum_type_layouts.insert(st.name.clone(), layout);
            self.sum_type_defs.insert(st.name.clone(), st.clone());
        }
    }

    // ── Function declaration and compilation ─────────────────────────

    fn declare_functions(&mut self, functions: &[MirFunction]) {
        for func in functions {
            let fn_type = if func.is_closure_fn {
                // Closure functions: (env_ptr, params...) -> ret
                // The first param in MIR is __env, skip it for user params
                let user_params: Vec<MirType> = func
                    .params
                    .iter()
                    .skip(1) // skip __env
                    .map(|(_, ty)| ty.clone())
                    .collect();
                llvm_closure_fn_type(
                    self.context,
                    &user_params,
                    &func.return_type,
                    &self.struct_types,
                    &self.sum_type_layouts,
                )
            } else {
                let param_types: Vec<MirType> =
                    func.params.iter().map(|(_, ty)| ty.clone()).collect();
                llvm_fn_type(
                    self.context,
                    &param_types,
                    &func.return_type,
                    &self.struct_types,
                    &self.sum_type_layouts,
                )
            };

            let fn_val = self.module.add_function(&func.name, fn_type, None);
            self.functions.insert(func.name.clone(), fn_val);
        }
    }

    fn compile_function(&mut self, func: &MirFunction) -> Result<(), String> {
        let fn_val = *self
            .functions
            .get(&func.name)
            .ok_or_else(|| format!("Function '{}' not declared", func.name))?;

        self.current_fn = Some(fn_val);
        self.locals.clear();
        self.local_types.clear();

        // Create entry basic block.
        let entry = self.context.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        // Alloca for each parameter and store incoming values.
        for (i, (name, ty)) in func.params.iter().enumerate() {
            let llvm_ty = llvm_type(
                self.context,
                ty,
                &self.struct_types,
                &self.sum_type_layouts,
            );
            let alloca = self.builder.build_alloca(llvm_ty, name).map_err(|e| e.to_string())?;
            let param_val = fn_val.get_nth_param(i as u32).ok_or_else(|| {
                format!("Missing parameter {} for function '{}'", i, func.name)
            })?;
            self.builder
                .build_store(alloca, param_val)
                .map_err(|e| e.to_string())?;
            self.locals.insert(name.clone(), alloca);
            self.local_types.insert(name.clone(), ty.clone());
        }

        // Compile the function body.
        let result = self.codegen_expr(&func.body)?;

        // Build return instruction (if not already terminated).
        let current_block = self.builder.get_insert_block().unwrap();
        if current_block.get_terminator().is_none() {
            match func.return_type {
                MirType::Unit => {
                    let unit_val = self.context.struct_type(&[], false).const_zero();
                    self.builder
                        .build_return(Some(&unit_val))
                        .map_err(|e| e.to_string())?;
                }
                MirType::Never => {
                    self.builder
                        .build_unreachable()
                        .map_err(|e| e.to_string())?;
                }
                _ => {
                    self.builder
                        .build_return(Some(&result))
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        Ok(())
    }

    /// Generate the C-level `main` function wrapper.
    ///
    /// Creates: `main(argc: i32, argv: ptr) -> i32` that calls
    /// `snow_rt_init()`, then calls the Snow entry function, then returns 0.
    fn generate_main_wrapper(&mut self, entry_name: &str) -> Result<(), String> {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let main_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
        let main_fn = self.module.add_function("main", main_type, None);

        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);

        // Call snow_rt_init()
        let rt_init = intrinsics::get_intrinsic(&self.module, "snow_rt_init");
        self.builder
            .build_call(rt_init, &[], "")
            .map_err(|e| e.to_string())?;

        // Call the Snow entry function
        let snow_main = self
            .functions
            .get(entry_name)
            .ok_or_else(|| format!("Entry function '{}' not found", entry_name))?;
        self.builder
            .build_call(*snow_main, &[], "")
            .map_err(|e| e.to_string())?;

        // Return 0
        self.builder
            .build_return(Some(&i32_type.const_int(0, false)))
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Get the LLVM type for a MIR type, using the cached struct/sum type layouts.
    pub(crate) fn llvm_type(&self, ty: &MirType) -> inkwell::types::BasicTypeEnum<'ctx> {
        llvm_type(
            self.context,
            ty,
            &self.struct_types,
            &self.sum_type_layouts,
        )
    }

    /// Get the current function being compiled.
    pub(crate) fn current_function(&self) -> FunctionValue<'ctx> {
        self.current_fn.expect("No current function during codegen")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirExpr, MirModule, MirType};

    fn empty_mir_module() -> MirModule {
        MirModule {
            functions: vec![],
            structs: vec![],
            sum_types: vec![],
            entry_function: None,
        }
    }

    fn hello_world_mir() -> MirModule {
        MirModule {
            functions: vec![MirFunction {
                name: "snow_main".to_string(),
                params: vec![],
                return_type: MirType::Unit,
                body: MirExpr::Unit,
                is_closure_fn: false,
                captures: vec![],
            }],
            structs: vec![],
            sum_types: vec![],
            entry_function: Some("snow_main".to_string()),
        }
    }

    #[test]
    fn test_empty_module_verifies() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test", 0, None).unwrap();
        let mir = empty_mir_module();
        codegen.compile(&mir).unwrap();
    }

    #[test]
    fn test_native_target_init() {
        let context = Context::create();
        let codegen = CodeGen::new(&context, "test", 0, None);
        assert!(codegen.is_ok(), "Native target should initialize");
    }

    #[test]
    fn test_invalid_triple_error() {
        let context = Context::create();
        let result = CodeGen::new(&context, "test", 0, Some("invalid-triple-xxx"));
        match result {
            Ok(_) => panic!("Invalid triple should return error"),
            Err(err) => {
                assert!(
                    err.contains("Invalid target triple") || err.contains("invalid-triple-xxx"),
                    "Error should mention the invalid triple, got: {}",
                    err
                );
            }
        }
    }

    #[test]
    fn test_hello_world_module_verifies() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test", 0, None).unwrap();
        let mir = hello_world_mir();
        codegen.compile(&mir).unwrap();

        // Check that main wrapper exists
        let ir = codegen.get_llvm_ir();
        assert!(ir.contains("define i32 @main"), "Should have main wrapper");
        assert!(ir.contains("snow_rt_init"), "Should call snow_rt_init");
    }

    #[test]
    fn test_emit_llvm_ir() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test", 0, None).unwrap();
        let mir = empty_mir_module();
        codegen.compile(&mir).unwrap();

        let tmp = std::env::temp_dir().join("snow_test.ll");
        codegen.emit_llvm_ir(&tmp).unwrap();
        assert!(tmp.exists());
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_emit_object_file() {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "test", 0, None).unwrap();
        let mir = empty_mir_module();
        codegen.compile(&mir).unwrap();

        let tmp = std::env::temp_dir().join("snow_test.o");
        codegen.emit_object(&tmp).unwrap();
        assert!(tmp.exists());
        std::fs::remove_file(&tmp).ok();
    }
}
