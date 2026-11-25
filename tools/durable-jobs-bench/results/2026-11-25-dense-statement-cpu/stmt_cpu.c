/* Diagnostic for an isolated, fork-based PostgreSQL 17.11 postmaster. */
#include "postgres.h"

#include <fcntl.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

#include "access/parallel.h"

#include "executor/executor.h"
#include "libpq/auth.h"
#include "libpq/libpq-be.h"
#include "miscadmin.h"
#include "optimizer/planner.h"
#include "port/atomics.h"
#include "storage/ipc.h"
#include "tcop/utility.h"
#include "utils/guc.h"

PG_MODULE_MAGIC;

#if PG_VERSION_NUM != 170011 || defined(EXEC_BACKEND)
#error This diagnostic requires fork-based PostgreSQL 17.11
#endif

#define SESSION_LIMIT 1024
#define KEY_LIMIT 256
#define REPORT "dense-cpu.jsonl"

enum Hook { START, RUN, FINISH, END, UTILITY, HOOKS };

typedef struct
{
    uint64 query;
    Oid role;
    Oid db;
    uint64 cpu;
    uint32 calls[HOOKS];
    uint32 errors;
} Counter;

typedef struct
{
    int pid;
    int auth_status;
    int exit_code;
    bool exited;
    bool clock_failed;
    char login[NAMEDATALEN];
    char database[NAMEDATALEN];
    uint64 cpu;
    uint64 nested;
    Counter overflow;
    Counter keys[KEY_LIMIT];
} Session;

typedef struct
{
    pg_atomic_uint32 next;
    pg_atomic_uint32 parallel_seen;
    Session sessions[SESSION_LIMIT];
} Shared;

typedef struct
{
    Counter *counter;
    uint64 start;
} Span;

static Shared *shared;
static Session *mine;
static int depth;
static bool enabled;
static ClientAuthentication_hook_type prev_auth;
static ExecutorStart_hook_type prev_start;
static ExecutorRun_hook_type prev_run;
static ExecutorFinish_hook_type prev_finish;
static ExecutorEnd_hook_type prev_end;
static ProcessUtility_hook_type prev_utility;
static planner_hook_type prev_planner;

static uint64
cpu_now(void)
{
    struct timespec now;

    if (clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &now) != 0)
    {
        mine->clock_failed = true;
        return 0;
    }
    return (uint64) now.tv_sec * UINT64CONST(1000000000) + now.tv_nsec;
}

static void
session_exit(int code, Datum arg)
{
    (void) arg;
    mine->exit_code = code;
    mine->cpu = cpu_now();
    mine->exited = true;
}

/* Only identifiers from this fixture are exported. */
static void
copy_name(char *dest, const char *name)
{
    if (name && strspn(name, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_") == strlen(name))
        strlcpy(dest, name, NAMEDATALEN);
    else
        strlcpy(dest, "unknown", NAMEDATALEN);
}

static void
authenticate(Port *port, int status)
{
    uint32 index = pg_atomic_fetch_add_u32(&shared->next, 1);

    if (index < SESSION_LIMIT)
    {
        mine = &shared->sessions[index];
        mine->pid = MyProcPid;
        mine->auth_status = status;
        copy_name(mine->login, port->user_name);
        copy_name(mine->database, port->database_name);
        on_proc_exit(session_exit, 0);
    }
    if (prev_auth)
        prev_auth(port, status);
}

static Span
begin_span(uint64 query, enum Hook hook)
{
    Span span = {NULL, 0};

    if (!mine)
    {
        if (IsParallelWorker())
            pg_atomic_write_u32(&shared->parallel_seen, 1);
        return span;
    }
    if (depth++ > 0)
    {
        mine->nested++;
        return span;
    }

    Oid role = GetUserId();
    Oid db = MyDatabaseId;
    uint32 hash = (uint32) query ^ (uint32) (query >> 32) ^ role ^ db;
    Counter *counter = &mine->overflow;

    for (uint32 step = 0; step < KEY_LIMIT; step++)
    {
        Counter *entry = &mine->keys[(hash + step) % KEY_LIMIT];

        if (!OidIsValid(entry->role))
        {
            entry->query = query;
            entry->role = role;
            entry->db = db;
        }
        if (entry->query == query && entry->role == role && entry->db == db)
        {
            counter = entry;
            break;
        }
    }
    span.counter = counter;
    counter->calls[hook]++;
    span.start = cpu_now();
    return span;
}

static void
end_span(Span span, bool failed)
{
    if (mine)
        depth--;
    if (span.counter)
    {
        uint64 end = cpu_now();

        if (end < span.start)
            mine->clock_failed = true;
        if (!mine->clock_failed)
            span.counter->cpu += end - span.start;
        if (failed)
            span.counter->errors++;
    }
}

/* Scope the clock around each hook call, including error unwinding. A portal
 * can survive arbitrary protocol activity between these calls. */
#define CHARGE(query, kind, call) \
    do { \
        Span span = begin_span(query, kind); \
        PG_TRY(); \
        { \
            call; \
        } \
        PG_CATCH(); \
        { \
            end_span(span, true); \
            PG_RE_THROW(); \
        } \
        PG_END_TRY(); \
        end_span(span, false); \
    } while (0)

static void
start(QueryDesc *desc, int flags)
{
    CHARGE(desc->plannedstmt->queryId, START,
           (prev_start ? prev_start : standard_ExecutorStart)(desc, flags));
}

static void
run_query(QueryDesc *desc, ScanDirection direction, uint64 count, bool once)
{
    CHARGE(desc->plannedstmt->queryId, RUN,
           (prev_run ? prev_run : standard_ExecutorRun)(desc, direction, count, once));
}

static void
finish(QueryDesc *desc)
{
    CHARGE(desc->plannedstmt->queryId, FINISH,
           (prev_finish ? prev_finish : standard_ExecutorFinish)(desc));
}

static void
end(QueryDesc *desc)
{
    CHARGE(desc->plannedstmt->queryId, END,
           (prev_end ? prev_end : standard_ExecutorEnd)(desc));
}

static void
utility(PlannedStmt *stmt, const char *sql, bool read_only, ProcessUtilityContext context,
        ParamListInfo params, QueryEnvironment *env, DestReceiver *dest, QueryCompletion *qc)
{
    CHARGE(stmt->queryId, UTILITY,
           (prev_utility ? prev_utility : standard_ProcessUtility)
           (stmt, sql, read_only, context, params, env, dest, qc));
}

/* Planner CPU remains in the residual. SPI invoked by a planner callback
 * must not acquire its own top-level executor charge. */
static PlannedStmt *
plan(Query *query, const char *sql, int options, ParamListInfo params)
{
    PlannedStmt *result;

    depth++;
    PG_TRY();
    {
        result = (prev_planner ? prev_planner : standard_planner)(query, sql, options, params);
    }
    PG_FINALLY();
    {
        depth--;
    }
    PG_END_TRY();
    return result;
}

static uint64
rusage_ns(struct rusage *usage)
{
    return ((uint64) usage->ru_utime.tv_sec + usage->ru_stime.tv_sec) * UINT64CONST(1000000000)
        + ((uint64) usage->ru_utime.tv_usec + usage->ru_stime.tv_usec) * 1000;
}

static void
write_counter(FILE *out, uint32 slot, const Counter *row)
{
    fprintf(out, "{\"slot\":%u,\"query\":\"" INT64_FORMAT "\",\"role\":%u,\"db\":%u,"
            "\"cpu_ns\":" UINT64_FORMAT ",\"errors\":%u,\"calls\":[%u,%u,%u,%u,%u]}\n",
            slot, (int64) row->query, row->role, row->db, row->cpu, row->errors,
            row->calls[START], row->calls[RUN], row->calls[FINISH], row->calls[END], row->calls[UTILITY]);
}

/* The parent reads the arena only at shutdown, after waiting for children. */
static void
publish(int code, Datum arg)
{
    int fd;
    FILE *out;
    uint32 total;
    struct rusage children;
    struct rusage parent;

    (void) arg;
    if (IsUnderPostmaster || code != 0 || getrusage(RUSAGE_CHILDREN, &children) != 0
        || getrusage(RUSAGE_SELF, &parent) != 0)
        return;
    fd = open(REPORT ".partial", O_WRONLY | O_CREAT | O_EXCL, 0600);
    if (fd < 0)
        return;
    out = fdopen(fd, "w");
    if (!out)
    {
        close(fd);
        return;
    }

    total = pg_atomic_read_u32(&shared->next);
    fprintf(out, "{\"version\":1,\"enabled\":%d,\"postmaster\":%d,\"registered\":%u,"
            "\"parallel_seen\":%u,\"children_cpu_ns\":" UINT64_FORMAT
            ",\"postmaster_cpu_ns\":" UINT64_FORMAT "}\n", enabled, (int) getpid(), total,
            pg_atomic_read_u32(&shared->parallel_seen), rusage_ns(&children), rusage_ns(&parent));
    for (uint32 i = 0; i < Min(total, SESSION_LIMIT); i++)
    {
        const Session *s = &shared->sessions[i];

        fprintf(out, "{\"session\":%u,\"pid\":%d,\"auth\":%d,\"exit\":%d,\"exited\":%d,"
                "\"login\":\"%s\",\"database\":\"%s\",\"clock_failed\":%d,"
                "\"cpu_ns\":" UINT64_FORMAT ",\"nested\":" UINT64_FORMAT "}\n",
                i, s->pid, s->auth_status, s->exit_code, s->exited, s->login, s->database,
                s->clock_failed, s->cpu, s->nested);
        for (uint32 key = 0; key < KEY_LIMIT; key++)
            if (OidIsValid(s->keys[key].role))
                write_counter(out, i, &s->keys[key]);
        for (int hook = 0; hook < HOOKS; hook++)
            if (s->overflow.calls[hook])
            {
                write_counter(out, i, &s->overflow);
                break;
            }
    }
    bool written = !ferror(out);

    if (fclose(out) == 0 && written)
    {
        int result = rename(REPORT ".partial", REPORT);
        if (result != 0)
            write_stderr("dense statement CPU report rename failed: %m\n");
    }
}

void
_PG_init(void)
{
    if (!process_shared_preload_libraries_in_progress)
        elog(ERROR, "dense statement CPU must be shared-preloaded");

    DefineCustomBoolVariable("dense_stmt_cpu.enabled", "Charge top-level statement hooks.", NULL,
                             &enabled, false, PGC_POSTMASTER, 0, NULL, NULL, NULL);
    shared = mmap(NULL, sizeof(*shared), PROT_READ | PROT_WRITE, MAP_ANONYMOUS | MAP_SHARED, -1, 0);
    if (shared == MAP_FAILED)
        elog(ERROR, "dense statement CPU arena mmap failed: %m");
    pg_atomic_init_u32(&shared->next, 0);
    pg_atomic_init_u32(&shared->parallel_seen, 0);
    on_proc_exit(publish, 0);
    prev_auth = ClientAuthentication_hook;
    ClientAuthentication_hook = authenticate;
    if (!enabled)
        return;

    prev_start = ExecutorStart_hook;
    ExecutorStart_hook = start;
    prev_run = ExecutorRun_hook;
    ExecutorRun_hook = run_query;
    prev_finish = ExecutorFinish_hook;
    ExecutorFinish_hook = finish;
    prev_end = ExecutorEnd_hook;
    ExecutorEnd_hook = end;
    prev_planner = planner_hook;
    planner_hook = plan;
    prev_utility = ProcessUtility_hook;
    ProcessUtility_hook = utility;
}
