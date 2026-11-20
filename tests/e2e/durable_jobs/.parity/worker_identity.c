#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

__attribute__((constructor))
static void record_worker_start(void) {
    const char *enabled = getenv("JOBS_PARITY_MARK_WORKER_START");
    if (enabled == NULL || strcmp(enabled, "1") != 0) {
        return;
    }
    // The coordinator uses 2/2. WorkerProcess sets both limits to 1 before exec.
    const char *min_workers = getenv("MESH_SCHEDULER_MIN_WORKERS");
    const char *max_workers = getenv("MESH_SCHEDULER_MAX_WORKERS");
    if (min_workers != NULL && strcmp(min_workers, "1") == 0 &&
        max_workers != NULL && strcmp(max_workers, "1") == 0) {
        dprintf(STDERR_FILENO, "PARITY_WORKER_START %ld\n", (long)getpid());
    }
}

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
