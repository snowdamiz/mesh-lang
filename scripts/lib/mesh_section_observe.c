/* Capture this exact child image's file-backed PTEs before exit tears down its mm. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ptrace.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static void fail(const char *reason) {
    fprintf(stderr, "observe: %s\n", reason);
    exit(125);
}

static int wait_child(pid_t child) {
    int status;
    pid_t got;
    do {
        got = waitpid(child, &status, 0);
    } while (got < 0 && errno == EINTR);
    if (got != child) {
        fail("waitpid");
    }
    return status;
}

struct mappings {
    FILE *maps;
    int pagemap;
    ino_t inode;
};

static struct mappings open_mappings(pid_t child, const char *image) {
    char proc[64];
    struct stat original, actual;
    snprintf(proc, sizeof(proc), "/proc/%d/exe", child);
    if (stat(image, &original) < 0 || stat(proc, &actual) < 0 ||
        original.st_dev != actual.st_dev || original.st_ino != actual.st_ino) {
        fail("training executable mismatch at exec");
    }
    snprintf(proc, sizeof(proc), "/proc/%d/maps", child);
    FILE *maps = fopen(proc, "r");
    snprintf(proc, sizeof(proc), "/proc/%d/pagemap", child);
    int pte = open(proc, O_RDONLY | O_CLOEXEC);
    if (!maps || pte < 0) {
        fail("expected readable maps and pagemap at exec");
    }
    return (struct mappings){.maps = maps, .pagemap = pte, .inode = original.st_ino};
}

static void pages(struct mappings child, const char *image, const char *output) {
    FILE *report = fopen(output, "w");
    long pg = sysconf(_SC_PAGESIZE);
    if (!report || pg <= 0) {
        fail("cannot open page report");
    }
    char line[4096];
    unsigned entries = 0;
    while (fgets(line, sizeof(line), child.maps)) {
        uint64_t lo, hi, offset, inode;
        unsigned major, minor;
        int name = 0;
        char mode[5];
        if (sscanf(line, "%" SCNx64 "-%" SCNx64 " %4s %" SCNx64
                   " %x:%x %" SCNu64 " %n", &lo, &hi, mode, &offset,
                   &major, &minor, &inode, &name) != 7 || !name) {
            continue;
        }
        line[strcspn(line, "\n")] = 0;
        if (strcmp(line + name, image) != 0 || inode != child.inode ||
            (strcmp(mode, "r-xp") != 0 && strcmp(mode, "r--p") != 0)) {
            continue;
        }
        for (uint64_t address = lo; address < hi; address += (uint64_t)pg) {
            if (++entries > 65536) {
                fail("too many executable pages");
            }
            uint64_t entry;
            if (pread(child.pagemap, &entry, sizeof(entry), address / pg * sizeof(entry)) != (ssize_t)sizeof(entry)) {
                fail("incomplete pagemap entry");
            }
            const uint64_t mask = (UINT64_C(1) << 63) | (UINT64_C(1) << 61);
            if ((entry & mask) == mask) {
                fprintf(report, "%" PRIu64 "\n", offset + address - lo);
            }
        }
    }
    if (ferror(child.maps) || !entries) {
        fail("no readable executable mappings");
    }
    if (fclose(report) != 0) {
        fail("cannot write the page report");
    }
    close(child.pagemap);
    fclose(child.maps);
}

int main(int argc, char **argv) {
    if (argc < 3 || argv[2][0] != '/') {
        fail("expected an output file and an absolute executable path");
    }
    pid_t child = fork();
    if (child < 0) {
        fail("fork");
    }
    if (child == 0) {
        if (ptrace(PTRACE_TRACEME, 0, NULL, NULL) < 0) {
            _exit(126);
        }
        execv(argv[2], &argv[2]);
        _exit(126);
    }
    int status = wait_child(child);
    if (!WIFSTOPPED(status) || WSTOPSIG(status) != SIGTRAP ||
        ptrace(PTRACE_SETOPTIONS, child, NULL,
               (void *)(uintptr_t)(PTRACE_O_TRACEEXIT | PTRACE_O_EXITKILL)) < 0) {
        kill(child, SIGKILL);
        fail("cannot attest exec stop");
    }
    /* Bind the executed inode and open the metadata views before runtime init.
       The application keeps its normal dumpability and other protections. */
    struct mappings mappings = open_mappings(child, argv[2]);
    if (ptrace(PTRACE_CONT, child, NULL, NULL) < 0) {
        fail("cannot resume executable");
    }
    status = wait_child(child);
    if (!WIFSTOPPED(status) || WSTOPSIG(status) != SIGTRAP ||
        (unsigned)status >> 16 != PTRACE_EVENT_EXIT) {
        fail("missing exit stop");
    }
    pages(mappings, argv[2], argv[1]);
    if (ptrace(PTRACE_CONT, child, NULL, NULL) < 0) {
        fail("cannot consume exit");
    }
    status = wait_child(child);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
