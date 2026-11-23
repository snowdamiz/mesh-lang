#include <stdint.h>

void *mesh_string_new(const uint8_t *data, uint64_t len);
void *mesh_bytes_new(const uint8_t *data, uint64_t len);
int64_t mesh_bytes_copy_to(const void *, int64_t, uint8_t *, uint64_t);

static int64_t text_calls;

void *gate_native_text(void) {
    const uint8_t source[] = {0xc3, 0xa9, 0, 0xf0, 0x9f, 0x99, 0x82};
    text_calls++;
    return mesh_string_new(source, sizeof(source));
}

int64_t gate_native_text_calls(void) { return text_calls; }

void *gate_native_invalid_utf8(void) {
    const uint8_t source[] = {255, 0, 192};
    return mesh_string_new(source, sizeof(source));
}

void *gate_native_convert(const void *text) {
    const uint8_t source[] = {255, 0, 128};
    if (text == 0) return 0;
    return mesh_bytes_new(source, sizeof(source));
}

int64_t gate_native_consume(const void *bytes) {
    uint8_t data[7];
    if (mesh_bytes_copy_to(bytes, 0, data, sizeof(data)) != sizeof(data)) return -1;
    return data[0] + data[2] + data[6];
}
