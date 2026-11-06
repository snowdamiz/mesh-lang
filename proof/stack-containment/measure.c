#include <stdint.h>
#include <stdio.h>

extern uintptr_t leaf(void), direct(void), indirect(void (*)(void *));
extern uintptr_t runtime_call(void), dynamic(uintptr_t), aligned(void);
extern uintptr_t small_leaf(void), loop_alloca(uintptr_t), body_check(uintptr_t);
void runtime_sink(void *p) { __asm__ volatile("" : : "r"(p) : "memory"); }

#define MEASURE(name, expression) do { \
  uintptr_t before; \
  __asm__ volatile("mov %0, sp" : "=r"(before) : : "memory"); \
  uintptr_t after = (expression); \
  printf("%s sp_delta=%lu\n", name, (unsigned long)(before - after)); \
} while (0)

int main(void) {
  MEASURE("leaf", leaf());
  MEASURE("direct", direct());
  MEASURE("indirect", indirect(runtime_sink));
  MEASURE("runtime_call", runtime_call());
  MEASURE("dynamic_32768", dynamic(32768));
  MEASURE("aligned", aligned());
  MEASURE("small_leaf", small_leaf());
  MEASURE("loop_alloca_32", loop_alloca(32));
  MEASURE("body_check", body_check(0));
}
