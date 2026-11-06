target triple = "arm64-apple-darwin"
declare ptr @llvm.stacksave.p0()
declare i64 @stack_limit()
declare void @fail_stack(i64) noreturn
declare void @runtime_bridge(i64)

define void @checked_direct(i64 %indirect) #0 {
entry:
  %frame = alloca [4096 x i8], align 16
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  %limit = call i64 @stack_limit()
  %low = icmp ult i64 %n, %limit
  br i1 %low, label %fail, label %ok
fail:
  call void @fail_stack(i64 %n)
  unreachable
ok:
  call void asm sideeffect "", "r,~{memory}"(ptr %frame)
  store volatile i8 1, ptr %frame
  %use_indirect = icmp ne i64 %indirect, 0
  br i1 %use_indirect, label %via_pointer, label %via_symbol
via_pointer:
  %fp = call ptr asm sideeffect "", "=r,0"(ptr @runtime_bridge)
  call void %fp(i64 %indirect)
  br label %done
via_symbol:
  call void @runtime_bridge(i64 %indirect)
  br label %done
done:
  call void asm sideeffect "", "r,~{memory}"(ptr %frame)
  ret void
}

attributes #0 = { noinline uwtable "warn-stack-size"="8192" "probe-stack"="inline-asm" "stack-probe-size"="4096" }
