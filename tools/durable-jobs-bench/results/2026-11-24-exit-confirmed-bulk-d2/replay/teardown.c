#include <fcntl.h>
#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>

extern uint64_t mesh_actor_self(void);
extern void mesh_actor_set_terminate(uint64_t, void (*)(const uint8_t *, const uint8_t *));

static void path_for(char path[PATH_MAX], const char *file) {
    const char *dir = getenv("COMBINED_TEARDOWN_DIR");
    if (!dir || snprintf(path, PATH_MAX, "%s/%s", dir, file) >= PATH_MAX) {
        abort();
    }
}

static void mark(const char *file) {
    char path[PATH_MAX];
    path_for(path, file);
    int fd = open(path, O_CREAT | O_EXCL | O_WRONLY, 0600);
    if (fd < 0 || close(fd)) {
        abort();
    }
}

static void terminate_probe(const uint8_t *state, const uint8_t *reason) {
    (void)state;
    if (*reason != 0) {
        mark("abnormal");
    }
    mark("entered");
    char path[PATH_MAX];
    path_for(path, "release");
    struct timespec wait = {.tv_sec = 0, .tv_nsec = 2000000};
    for (int tick = 0; tick < 5000; ++tick) {
        if (access(path, F_OK) == 0) {
            mark("finished");
            return;
        }
        nanosleep(&wait, NULL);
    }
    mark("timeout");
}

int64_t mesh_parity_arm_teardown(void) {
    uint64_t pid = mesh_actor_self();
    if (pid == UINT64_MAX) {
        return -1;
    }
    mesh_actor_set_terminate(pid, terminate_probe);
    return 0;
}
