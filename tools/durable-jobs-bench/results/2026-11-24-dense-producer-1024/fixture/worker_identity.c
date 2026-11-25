#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <time.h>
#include <unistd.h>

int64_t mesh_parity_realtime_us(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_REALTIME, &now) != 0) {
        return -1;
    }
    return (int64_t)now.tv_sec * 1000000 + now.tv_nsec / 1000;
}

int64_t mesh_parity_worker_pid(void) {
    return getpid();
}
