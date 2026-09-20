#include <stdint.h>
#include <string.h>

typedef struct {
  uint8_t tag;
  void *value;
} MeshResult;

void *mesh_gc_alloc_actor(uint64_t size, uint64_t align);
void *mesh_string_new(const uint8_t *data, uint64_t length);

int64_t mesh_math_add(int64_t left, int64_t right) {
  return left + right;
}

MeshResult mesh_math_double(int64_t value) {
  if (value < 0) {
    const char *message = "negative";
    return (MeshResult){
        1, mesh_string_new((const uint8_t *)message, strlen(message))};
  }

  int64_t *result = mesh_gc_alloc_actor(sizeof(int64_t), _Alignof(int64_t));
  *result = value * 2;
  return (MeshResult){0, result};
}
