#define _GNU_SOURCE
#include <pthread.h>
#include <stdint.h>
#include <stddef.h>
#include "libauthority.h"

_Static_assert(sizeof(MeshLibraryHostCallbacksV1) == 8 + 10 * sizeof(void *), "V1 layout changed");
_Static_assert(sizeof(MeshLibraryHostCallbacksV2) == 8 + sizeof(MeshLibraryHostCallbacksV1) + 2 * sizeof(void *), "V2 layout mismatch");

int native_stack_probe(void) {
  volatile unsigned char marker;
  uintptr_t address = (uintptr_t)&marker;
#ifdef __APPLE__
  uintptr_t top = (uintptr_t)pthread_get_stackaddr_np(pthread_self());
  size_t size = pthread_get_stacksize_np(pthread_self());
  return address >= top - size && address < top;
#else
  pthread_attr_t attr;
  void *base = NULL;
  size_t size = 0;
  if (pthread_getattr_np(pthread_self(), &attr)) return 0;
  int rc = pthread_attr_getstack(&attr, &base, &size);
  pthread_attr_destroy(&attr);
  return !rc && address >= (uintptr_t)base && address < (uintptr_t)base + size;
#endif
}
