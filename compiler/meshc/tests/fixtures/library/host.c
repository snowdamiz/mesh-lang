#include "libmesh_library.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int32_t secure_store_get(void *context, const uint8_t *input,
                                uint64_t input_len, uint8_t *output,
                                uint64_t output_capacity,
                                uint64_t *output_len) {
  (void)context;
  if (input_len > output_capacity) {
    return 1;
  }
  memcpy(output, input, (size_t)input_len);
  *output_len = input_len;
  return 0;
}

int main(void) {
  const uint8_t request[] = {'m', 'e', 's', 'h', 0, 'o', 'k'};
  MeshLibraryBytes response = {0};

  if (mesh_library_init() != MESH_LIBRARY_OK ||
      mesh_library_init() != MESH_LIBRARY_OK) {
    return 1;
  }
  MeshLibraryHostCallbacksV1 callbacks = {0};
  callbacks.abi_version = MESH_LIBRARY_ABI_VERSION;
  callbacks.struct_size = sizeof(callbacks);
  callbacks.secure_store_get = secure_store_get;
  if (mesh_library_register_host_callbacks(&callbacks) != MESH_LIBRARY_OK) {
    return 6;
  }
  if (mesh_fixture_echo(request, sizeof(request), &response) !=
          MESH_LIBRARY_OK ||
      response.len != sizeof(request) ||
      memcmp(response.data, request, sizeof(request)) != 0) {
    return 2;
  }
  mesh_library_free_returned_bytes(&response);
  if (response.data != NULL || response.len != 0) {
    return 3;
  }
  if (mesh_library_shutdown() != MESH_LIBRARY_OK ||
      mesh_library_shutdown() != MESH_LIBRARY_OK) {
    return 4;
  }
  if (mesh_fixture_echo(NULL, 0, &response) != 2) {
    return 5;
  }

  puts("mesh library host proof passed");
  return 0;
}
