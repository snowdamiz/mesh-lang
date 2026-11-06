declare ptr @llvm.stacksave.p0()
declare void @runtime_sink(ptr)

define void @callee(ptr %p) #0 {
  call void asm sideeffect "", "r,~{memory}"(ptr %p)
  ret void
}

define i64 @leaf() #0 {
  %p = alloca [32768 x i8], align 16
  call void asm sideeffect "", "r,~{memory}"(ptr %p)
  store volatile i8 1, ptr %p
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @direct() #0 {
  %p = alloca [32768 x i8], align 16
  call void @callee(ptr %p)
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @indirect(ptr %f) #0 {
  %p = alloca [32768 x i8], align 16
  call void %f(ptr %p)
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @runtime_call() #0 {
  %p = alloca [32768 x i8], align 16
  call void @runtime_sink(ptr %p)
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @dynamic(i64 %size) #0 {
  %p = alloca i8, i64 %size, align 16
  call void asm sideeffect "", "r,~{memory}"(ptr %p)
  store volatile i8 1, ptr %p
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @aligned() #0 {
  %p = alloca [16 x i8], align 65536
  call void asm sideeffect "", "r,~{memory}"(ptr %p)
  store volatile i8 1, ptr %p
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @small_leaf() #0 {
  %p = alloca [64 x i8], align 16
  call void asm sideeffect "", "r,~{memory}"(ptr %p)
  store volatile i8 1, ptr %p
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @loop_alloca(i64 %count) #0 {
entry:
  br label %loop
loop:
  %i = phi i64 [0, %entry], [%next, %loop]
  %p = alloca [1024 x i8], align 16
  call void asm sideeffect "", "r,~{memory}"(ptr %p)
  store volatile i8 1, ptr %p
  %next = add i64 %i, 1
  %again = icmp ult i64 %next, %count
  br i1 %again, label %loop, label %done
done:
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  ret i64 %n
}

define i64 @body_check(i64 %limit) #0 {
  %p = alloca [32768 x i8], align 16
  %sp = call ptr @llvm.stacksave.p0()
  %n = ptrtoint ptr %sp to i64
  %low = icmp ult i64 %n, %limit
  br i1 %low, label %fail, label %ok
fail:
  ret i64 0
ok:
  call void asm sideeffect "", "r,~{memory}"(ptr %p)
  store volatile i8 1, ptr %p
  ret i64 %n
}

; OUTGOING

attributes #0 = { noinline uwtable "warn-stack-size"="4096" "probe-stack"="inline-asm" "stack-probe-size"="4096" }
