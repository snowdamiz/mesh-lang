#include "libmesh_gc.h"
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>

static uint64_t baseline;
static int warmup;
static int saw_reentry;
static const uint8_t request[] = {1, 0, 0xff, 7, 99};

static uint64_t peak_bytes(void) {
  struct rusage usage;
  if (getrusage(RUSAGE_SELF, &usage)) abort();
#ifdef __APPLE__
  return (uint64_t)usage.ru_maxrss;
#else
  return (uint64_t)usage.ru_maxrss * 1024;
#endif
}

static int32_t echo(void *context, const uint8_t *input, uint64_t input_len,
                    uint8_t *output, uint64_t capacity, uint64_t *output_len) {
  (void)context;
  MeshLibraryBytes nested = {0};
  if (mesh_gc_fixture(input, input_len, &nested) != MESH_LIBRARY_ERR_BUSY ||
      nested.data || nested.len) return 7;
  saw_reentry++;
  uint64_t peak = peak_bytes();
  uint64_t growth = peak > baseline ? peak - baseline : 0;
  printf("{\"checkpoint\":\"inside_export_callback\",\"peak_growth_bytes\":%llu}\n",
         (unsigned long long)growth);
  fflush(stdout);
  if (!warmup && growth > 8 * 1024 * 1024) return 7;
  if (input_len > capacity) return 7;
  memcpy(output, input, input_len);
  *output_len = input_len;
  return 0;
}

static void *invoke_on_worker(void *ignored) {
  (void)ignored;
  MeshLibraryBytes output = {0};
  baseline = peak_bytes();
  int status = mesh_gc_fixture(request, sizeof(request), &output);
  if (status != MESH_LIBRARY_OK || output.len != 2 * sizeof(request) + 3 ||
      memcmp(output.data, request, sizeof(request)) ||
      memcmp(output.data + sizeof(request), request, sizeof(request)) ||
      memcmp(output.data + 2 * sizeof(request), "123", 3)) {
    fprintf(stderr, "export failed status=%d length=%llu\n", status,
            (unsigned long long)output.len);
    mesh_library_free_returned_bytes(&output);
    return (void *)(uintptr_t)1;
  }
  mesh_library_free_returned_bytes(&output);
  for (uint8_t seed = 0; seed < 2; seed++) {
    const char *expected = seed ?
      "payload-1|capture-1|map-0|map-1|flat-0|flat-1|" :
      "payload-0|capture-0|map-0|map-1|flat-0|flat-1|";
    baseline = peak_bytes();
    status = mesh_gc_independent_roots(&seed, 1, &output);
    if (status || output.len != strlen(expected) ||
        memcmp(output.data, expected, output.len)) {
      fprintf(stderr, "independent roots failed seed=%d status=%d\n", seed, status);
      mesh_library_free_returned_bytes(&output);
      return (void *)(uintptr_t)2;
    }
    mesh_library_free_returned_bytes(&output);
  }
  return NULL;
}

int main(void) {
  if (mesh_library_init() != MESH_LIBRARY_OK) return 1;
  MeshLibraryHostCallbacksV1 callbacks = {0};
  callbacks.abi_version = MESH_LIBRARY_ABI_VERSION;
  callbacks.struct_size = sizeof(callbacks);
  callbacks.secure_store_get = echo;
  if (mesh_library_register_host_callbacks(&callbacks)) return 2;
  const uint8_t small[] = {0};
  MeshLibraryBytes output = {0};
  warmup = 1;
  if (mesh_gc_fixture(small, sizeof(small), &output)) return 3;
  mesh_library_free_returned_bytes(&output);
  warmup = 0;
  pthread_t worker;
  if (pthread_create(&worker, NULL, invoke_on_worker, NULL)) return 4;
  void *result;
  if (pthread_join(worker, &result) || result || saw_reentry != 4) return 5;
  if (mesh_library_shutdown()) return 6;
  puts("library safe-point pressure and live roots passed");
  return 0;
}
