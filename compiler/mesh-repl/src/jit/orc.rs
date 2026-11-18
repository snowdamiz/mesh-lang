//! ORC uses JITLink to register unwind information for generated Mach-O code.
//! MCJIT's Mach-O loader can register a frame whose PC range misses the
//! generated function on Apple Silicon. JITLink lets a panic in that
//! function reach the runtime's actor panic handler.

use inkwell::llvm_sys::error::{
    LLVMConsumeError, LLVMDisposeErrorMessage, LLVMErrorRef, LLVMGetErrorMessage,
};
use inkwell::llvm_sys::orc2::lljit::*;
use inkwell::llvm_sys::orc2::*;
use inkwell::llvm_sys::prelude::{LLVMContextRef, LLVMModuleRef};
use std::ffi::{CStr, CString};
use std::ptr;

extern "C" {
    // LLVM 21 replaced the thread-safe context getter with this constructor.
    // llvm-sys 211.0.0 still declares the earlier API.
    fn LLVMOrcCreateNewThreadSafeContextFromLLVMContext(
        context: LLVMContextRef,
    ) -> LLVMOrcThreadSafeContextRef;
}

pub(super) struct ExecutionEngine(LLVMOrcLLJITRef);

impl ExecutionEngine {
    pub(super) fn new() -> Result<Self, String> {
        let mut jit = ptr::null_mut();
        // LLJIT's default process-symbol search resolves libcalls emitted by
        // LLVM, such as memcpy. Register Mesh runtime symbols explicitly.
        check(unsafe { LLVMOrcCreateLLJIT(&mut jit, ptr::null_mut()) })
            .map_err(|error| format!("Failed to create ORC JIT engine: {error}"))?;
        let engine = Self(jit);
        engine.define_runtime_symbols()?;
        Ok(engine)
    }

    fn define_runtime_symbols(&self) -> Result<(), String> {
        use LLVMJITSymbolGenericFlags::{
            LLVMJITSymbolGenericFlagsCallable, LLVMJITSymbolGenericFlagsExported,
        };

        let mut symbols: Vec<_> = super::runtime_symbols()
            .into_iter()
            .map(|(name, pointer)| {
                let name = CString::new(name).unwrap();
                LLVMOrcCSymbolMapPair {
                    Name: unsafe { LLVMOrcLLJITMangleAndIntern(self.0, name.as_ptr()) },
                    Sym: LLVMJITEvaluatedSymbol {
                        Address: pointer as usize as u64,
                        Flags: LLVMJITSymbolFlags {
                            GenericFlags: LLVMJITSymbolGenericFlagsCallable as u8
                                | LLVMJITSymbolGenericFlagsExported as u8,
                            TargetFlags: 0,
                        },
                    },
                }
            })
            .collect();

        unsafe {
            let definitions = LLVMOrcAbsoluteSymbols(symbols.as_mut_ptr(), symbols.len());
            let dylib = LLVMOrcLLJITGetMainJITDylib(self.0);
            let error = LLVMOrcJITDylibDefine(dylib, definitions);
            if !error.is_null() {
                LLVMOrcDisposeMaterializationUnit(definitions);
            }
            check(error).map_err(|error| format!("Failed to register JIT runtime symbols: {error}"))
        }
    }

    /// Takes ownership of a module and its dedicated context.
    ///
    /// Both pointers must be live, and the caller must transfer sole ownership.
    pub(super) unsafe fn add_module(
        &self,
        module: LLVMModuleRef,
        context: LLVMContextRef,
    ) -> Result<(), String> {
        let context = LLVMOrcCreateNewThreadSafeContextFromLLVMContext(context);
        let module = LLVMOrcCreateNewThreadSafeModule(module, context);
        LLVMOrcDisposeThreadSafeContext(context);
        let dylib = LLVMOrcLLJITGetMainJITDylib(self.0);
        check(LLVMOrcLLJITAddLLVMIRModule(self.0, dylib, module))
            .map_err(|error| format!("Failed to add module to ORC JIT: {error}"))
    }

    pub(super) fn get_function_address(&self, name: &str) -> Result<usize, String> {
        let name = CString::new(name).map_err(|error| error.to_string())?;
        let mut address = 0;
        check(unsafe { LLVMOrcLLJITLookup(self.0, &mut address, name.as_ptr()) })?;
        Ok(address as usize)
    }
}

impl Drop for ExecutionEngine {
    fn drop(&mut self) {
        unsafe { LLVMConsumeError(LLVMOrcDisposeLLJIT(self.0)) };
    }
}

fn check(error: LLVMErrorRef) -> Result<(), String> {
    if error.is_null() {
        return Ok(());
    }
    unsafe {
        let raw = LLVMGetErrorMessage(error);
        let message = CStr::from_ptr(raw).to_string_lossy().into_owned();
        LLVMDisposeErrorMessage(raw);
        Err(message)
    }
}
