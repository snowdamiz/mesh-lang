#include <llvm-c/Core.h>
#include <llvm-c/ExecutionEngine.h>
#include <llvm-c/IRReader.h>
#include <llvm-c/Target.h>
#include <llvm-c/TargetMachine.h>
#include <llvm/IR/DiagnosticInfo.h>
#include <llvm/IR/Function.h>
#include <llvm/IR/LLVMContext.h>
#include <llvm/Support/CBindingWrapping.h>
#include <cstdio>
#include <cstring>

struct Diagnostics {
  unsigned frames = 0;
  unsigned errors = 0;
  LLVMContextRef context;
  bool promote;
};
static void runtime_sink(void *) {}

static void diagnostic(LLVMDiagnosticInfoRef info, void *opaque) {
  auto &state = *static_cast<Diagnostics *>(opaque);
  auto *native = llvm::unwrap(info);
  if (auto *frame = llvm::dyn_cast<llvm::DiagnosticInfoStackSize>(native)) {
    ++state.frames;
    std::printf("frame=%s bytes=%llu severity=%d\n",
                frame->getFunction().getName().str().c_str(),
                static_cast<unsigned long long>(frame->getStackSize()),
                LLVMGetDiagInfoSeverity(info));
    if (state.promote) llvm::unwrap(state.context)->emitError("stack cap exceeded");
  }
  if (LLVMGetDiagInfoSeverity(info) == LLVMDSError) ++state.errors;
}

int main(int argc, char **argv) {
  if (argc != 4) return 64;
  LLVMInitializeNativeTarget();
  LLVMInitializeNativeAsmPrinter();
  LLVMLinkInMCJIT();
  auto context = LLVMContextCreate();
  Diagnostics state{0, 0, context, std::strstr(argv[1], "error") != nullptr};
  LLVMContextSetDiagnosticHandler(context, diagnostic, &state);
  LLVMMemoryBufferRef input;
  LLVMModuleRef module;
  char *error = nullptr;
  if (LLVMCreateMemoryBufferWithContentsOfFile(argv[2], &input, &error) ||
      LLVMParseIRInContext(context, input, &module, &error)) {
    std::fprintf(stderr, "%s\n", error);
    return 65;
  }
  char *triple = LLVMGetDefaultTargetTriple();
  LLVMSetTarget(module, triple);
  unsigned level = std::strcmp(argv[3], "O0") == 0 ? 0 : 2;
  bool rejected = false;
  if (std::strncmp(argv[1], "jit", 3) == 0) {
    LLVMMCJITCompilerOptions options;
    LLVMInitializeMCJITCompilerOptions(&options, sizeof(options));
    options.OptLevel = level;
    LLVMExecutionEngineRef engine;
    int rc = LLVMCreateMCJITCompilerForModule(&engine, module, &options,
                                            sizeof(options), &error);
    std::printf("create_rc=%d diagnostics_before_lookup=%u\n", rc, state.frames);
    if (rc) return 66;
    if (auto f = LLVMGetNamedFunction(module, "runtime_sink"))
      LLVMAddGlobalMapping(engine, f, reinterpret_cast<void *>(runtime_sink));
    if (auto f = LLVMGetNamedFunction(module, "many_args"))
      LLVMAddGlobalMapping(engine, f, reinterpret_cast<void *>(runtime_sink));
    auto address = LLVMGetFunctionAddress(engine,
        LLVMGetNamedFunction(module, "leaf") ? "leaf" : "dynamic");
    rejected = state.frames || state.errors;
    std::printf("address_nonzero=%d diagnostics_after_lookup=%u rejected=%d executed=0\n",
                address != 0, state.frames, rejected);
    LLVMDisposeExecutionEngine(engine);
  } else {
    LLVMTargetRef target;
    if (LLVMGetTargetFromTriple(triple, &target, &error)) return 67;
    auto machine = LLVMCreateTargetMachine(target, triple, "generic", "",
        level == 0 ? LLVMCodeGenLevelNone : LLVMCodeGenLevelDefault,
        LLVMRelocPIC, LLVMCodeModelDefault);
    auto layout = LLVMCreateTargetDataLayout(machine);
    LLVMSetModuleDataLayout(module, layout);
    LLVMMemoryBufferRef output = nullptr;
    int rc = LLVMTargetMachineEmitToMemoryBuffer(machine, module, LLVMObjectFile,
                                                &error, &output);
    rejected = state.frames || state.errors;
    std::printf("emit_rc=%d output_bytes=%zu rejected=%d published=0\n", rc,
                output ? LLVMGetBufferSize(output) : 0, rejected);
    if (output) LLVMDisposeMemoryBuffer(output);
    LLVMDisposeTargetData(layout);
    LLVMDisposeTargetMachine(machine);
    LLVMDisposeModule(module);
  }
  std::printf("stack_diagnostics=%u error_diagnostics=%u context_has_errors=%d\n",
              state.frames, state.errors,
              llvm::unwrap(context)->getDiagHandlerPtr()->HasErrors);
  LLVMDisposeMessage(triple);
  LLVMContextDispose(context);
  return rejected ? 2 : 0;
}
