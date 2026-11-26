#include <float.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>

typedef struct {
    uint64_t len;
    unsigned char data[];
} MeshString;

extern void mesh_rt_init(void);
extern MeshString *mesh_string_new(const unsigned char *, uint64_t);
extern void *mesh_json_from_string(const MeshString *);
extern void *mesh_json_from_float(double);
extern MeshString *mesh_json_encode(void *);

static int emit(void *value) {
    MeshString *encoded = mesh_json_encode(value);
    if (fwrite(encoded->data, 1, encoded->len, stdout) != encoded->len) {
        return 1;
    }
    return fputc('\n', stdout) == EOF;
}

static int check_encoding(const unsigned char *data, size_t length) {
    return emit(mesh_json_from_string(mesh_string_new(data, length)));
}

int main(void) {
    mesh_rt_init();
    const unsigned char short_value[] = { 'a', 0xff, 0, '\n', '"', '\\', 0x80 };
    if (check_encoding(short_value, sizeof(short_value))) {
        return 1;
    }
    unsigned char every_byte[1027];
    for (size_t index = 0; index < sizeof(every_byte); ++index) {
        every_byte[index] = (unsigned char)index;
    }
    if (check_encoding(every_byte, sizeof(every_byte))) {
        return 1;
    }
    const double floats[] = { NAN, INFINITY, -INFINITY, -0.0, DBL_MIN, DBL_MAX, 0x1p-1074, 1.0e20, -1.0e-20 };
    for (size_t index = 0; index < sizeof(floats)/sizeof(*floats); ++index) {
        if (emit(mesh_json_from_float(floats[index]))) {
            return 1;
        }
    }
    return 0;
}
