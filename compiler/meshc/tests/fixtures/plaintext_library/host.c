#include "libmesh_plaintext.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

int main(void) {
  const uint8_t request[] = {'h', 'e', 'l', 'l', 'o'};
  const uint8_t expected[] = {'H', 'E', 'L', 'L', 'O'};
  MeshLibraryBytes response = {0};

  if (mesh_library_init() != MESH_LIBRARY_OK) {
    return 1;
  }
  if (mesh_plaintext_fixture_shout(request, sizeof(request), &response) !=
          MESH_LIBRARY_OK ||
      response.len != sizeof(expected) ||
      memcmp(response.data, expected, sizeof(expected)) != 0) {
    return 2;
  }
  mesh_library_free_returned_bytes(&response);
  fputs("display export passed\n", stderr);
  return mesh_library_shutdown() == MESH_LIBRARY_OK ? 0 : 3;
}
