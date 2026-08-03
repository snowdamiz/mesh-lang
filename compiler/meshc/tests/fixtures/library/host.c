#include "libmesh_library.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef struct {
  uint8_t key[64];
  uint64_t key_len;
  uint8_t value[64];
  uint64_t value_len;
} SecureRecord;

static SecureRecord records[2] = {0};

static int32_t secure_store_get(void *context, const uint8_t *input,
                                uint64_t input_len, uint8_t *output,
                                uint64_t output_capacity,
                                uint64_t *output_len) {
  (void)context;
  for (size_t i = 0; i < 2; i++) {
    if (records[i].key_len == input_len &&
        memcmp(records[i].key, input, (size_t)input_len) == 0) {
      if (records[i].value_len > output_capacity) {
        return 1;
      }
      memcpy(output, records[i].value, (size_t)records[i].value_len);
      *output_len = records[i].value_len;
      return 0;
    }
  }
  const uint8_t echo_key[] = {'m', 'e', 's', 'h', 0, 'o', 'k'};
  if (input_len != sizeof(echo_key) ||
      memcmp(input, echo_key, sizeof(echo_key)) != 0) {
    return 2;
  }
  if (input_len > output_capacity) {
    return 1;
  }
  memcpy(output, input, (size_t)input_len);
  *output_len = input_len;
  return 0;
}

static int32_t secure_store_put(void *context, const uint8_t *input,
                                uint64_t input_len, uint8_t *output,
                                uint64_t output_capacity,
                                uint64_t *output_len) {
  (void)context;
  (void)output;
  (void)output_capacity;
  *output_len = 0;
  if (input_len < 4) {
    return 1;
  }
  uint64_t key_len = ((uint64_t)input[0] << 24) | ((uint64_t)input[1] << 16) |
                     ((uint64_t)input[2] << 8) | (uint64_t)input[3];
  if (key_len > sizeof(records[0].key) || key_len > input_len - 4) {
    return 1;
  }
  uint64_t value_len = input_len - 4 - key_len;
  if (value_len > sizeof(records[0].value)) {
    return 1;
  }
  SecureRecord *record = NULL;
  for (size_t i = 0; i < 2; i++) {
    if (records[i].key_len == 0 ||
        (records[i].key_len == key_len &&
         memcmp(records[i].key, input + 4, (size_t)key_len) == 0)) {
      record = &records[i];
      break;
    }
  }
  if (record == NULL) {
    return 1;
  }
  memcpy(record->key, input + 4, (size_t)key_len);
  memcpy(record->value, input + 4 + key_len, (size_t)value_len);
  record->key_len = key_len;
  record->value_len = value_len;
  return 0;
}

static int register_callbacks(void) {
  MeshLibraryHostCallbacksV1 callbacks = {0};
  callbacks.abi_version = MESH_LIBRARY_ABI_VERSION;
  callbacks.struct_size = sizeof(callbacks);
  callbacks.secure_store_get = secure_store_get;
  callbacks.secure_store_put = secure_store_put;
  return mesh_library_register_host_callbacks(&callbacks);
}

int main(void) {
  const uint8_t request[] = {'m', 'e', 's', 'h', 0, 'o', 'k'};
  MeshLibraryBytes response = {0};

  if (mesh_library_init() != MESH_LIBRARY_OK ||
      mesh_library_init() != MESH_LIBRARY_OK) {
    return 1;
  }
  if (register_callbacks() != MESH_LIBRARY_OK) {
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
  if (mesh_fixture_storage_roundtrip(request, sizeof(request), &response) !=
          MESH_LIBRARY_OK ||
      response.len != sizeof(request) ||
      memcmp(response.data, request, sizeof(request)) != 0) {
    return 7;
  }
  mesh_library_free_returned_bytes(&response);
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
