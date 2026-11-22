#define _POSIX_C_SOURCE 200809L
#ifdef MESH_PARITY_J2_LANE_7
#define _DARWIN_C_SOURCE
#endif
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#ifdef MESH_PARITY_J2_LANE_7
#include <dlfcn.h>
#include <errno.h>
#include <spawn.h>
#include <stdatomic.h>

static int wait_for_gate(const char *path) {
    const struct timespec delay = {0, 10000000};
    for (int i = 0; i < 1500; i++) {
        if (access(path, F_OK) == 0) {
            return 0;
        }
        nanosleep(&delay, NULL);
    }
    return ETIMEDOUT;
}

// The Rust macOS launcher calls posix_spawnp. Hold its second worker call
// before libc sees the path so the driver can deny exec without a launch race.
int posix_spawnp(pid_t *pid, const char *path, const posix_spawn_file_actions_t *actions,
                 const posix_spawnattr_t *attrs, char *const argv[], char *const envp[]) {
    static atomic_int worker_calls;
    __typeof__(&posix_spawnp) spawn = dlsym(RTLD_NEXT, "posix_spawnp");
    if (spawn == NULL) {
        return ENOSYS;
    }
    const char *gate = getenv("JOBS_PARITY_SECOND_SPAWN_GATE");
    if (gate == NULL || argv[1] == NULL || strcmp(argv[1], "--mesh-internal-job-attempt-v1") != 0) {
        return spawn(pid, path, actions, attrs, argv, envp);
    }
    int call = atomic_fetch_add(&worker_calls, 1) + 1;
    if (call == 2) {
        dprintf(STDERR_FILENO, "PARITY_SECOND_SPAWN_WAIT\n");
        int error = wait_for_gate(gate);
        if (error != 0) {
            return error;
        }
    }
    int result = spawn(pid, path, actions, attrs, argv, envp);
    dprintf(STDERR_FILENO, "PARITY_SPAWN_RESULT %d %d\n", call, result);
    return result;
}
#endif

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
#ifdef MESH_PARITY_J2_LANE_7
        const char *gate = getenv("JOBS_PARITY_WORKER_START_GATE");
        if (gate != NULL && wait_for_gate(gate) != 0) {
            dprintf(STDERR_FILENO, "PARITY_WORKER_START_GATE_TIMEOUT\n");
            _exit(125);
        }
#endif
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
