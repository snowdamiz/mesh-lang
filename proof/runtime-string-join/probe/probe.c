#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <time.h>

struct text { uint64_t len; unsigned char bytes[]; };

int64_t probe_cpu_ns(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts) != 0) return -1;
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

int64_t probe_checksum(const struct text *value, int64_t acc) {
    uint64_t len = value->len;
    uint64_t sum = (uint64_t)acc * 1000003 + len;
    if (len) {
        sum = sum * 257 + value->bytes[0];
        sum = sum * 257 + value->bytes[len / 4];
        sum = sum * 257 + value->bytes[len / 2];
        sum = sum * 257 + value->bytes[len - 1];
    }
    return (int64_t)(sum & INT64_MAX);
}
