/* Untimed Linux fixture: record fresh spawn and exact consuming waits. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

typedef int (*spawn_call)(pid_t *, const char *, const posix_spawn_file_actions_t *,
                          const posix_spawnattr_t *, char *const [], char *const []);
static spawn_call spawn_original;
static pid_t (*wait_original)(pid_t, int *, int);
static int (*waitid_original)(idtype_t, id_t, siginfo_t *, int);
static int output = -1;

__attribute__((constructor)) static void setup(void) {
    spawn_original = dlsym(RTLD_NEXT, "posix_spawnp");
    wait_original = dlsym(RTLD_NEXT, "waitpid");
    waitid_original = dlsym(RTLD_NEXT, "waitid");
    if (!spawn_original || !wait_original || !waitid_original) _exit(91);
    const char *path = getenv("MESH_ADMISSION_WAIT_RECEIPT");
    if (path) {
        output = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_APPEND, 0600);
        if (output < 0) _exit(92);
    }
}

static void record(const char *event, pid_t pid, int status) {
    if (output >= 0) {
        int saved_errno = errno;
        struct timespec now;
        char line[96];
        if (clock_gettime(CLOCK_REALTIME, &now)) _exit(93);
        long long stamp = (long long)now.tv_sec * 1000000000 + now.tv_nsec;
        int length = snprintf(line, sizeof line, "%s %d %d %lld\n", event, pid, status, stamp);
        if (length <= 0 || length >= (int)sizeof line || write(output, line, length) != length)
            _exit(94);
        errno = saved_errno;
    }
}

int posix_spawnp(pid_t *pid, const char *path, const posix_spawn_file_actions_t *actions,
                 const posix_spawnattr_t *attributes, char *const args[], char *const env[]) {
    int result = spawn_original(pid, path, actions, attributes, args, env);
    if (result == 0) record("exec", *pid, 0);
    return result;
}

pid_t waitpid(pid_t pid, int *status, int flags) {
    pid_t result = wait_original(pid, status, flags);
    if (result > 0 && status && (WIFEXITED(*status) || WIFSIGNALED(*status))) {
        if (pid != result) _exit(95);
        record("wait", result, *status);
    }
    return result;
}

int waitid(idtype_t type, id_t id, siginfo_t *info, int flags) {
    int result = waitid_original(type, id, info, flags);
    if (result == 0 && !(flags & WNOWAIT) && info && info->si_pid > 0 &&
        (info->si_code == CLD_EXITED || info->si_code == CLD_KILLED || info->si_code == CLD_DUMPED)) {
        if (type == P_PID && (id_t)info->si_pid != id) _exit(96);
        record("wait", info->si_pid, info->si_status | ((info->si_code - CLD_EXITED) << 16));
    }
    return result;
}
