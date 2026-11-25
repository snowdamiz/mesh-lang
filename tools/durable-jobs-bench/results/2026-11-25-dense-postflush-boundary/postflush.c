#include "postgres.h"

#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "access/xact.h"
#include "miscadmin.h"
#include "storage/ipc.h"
#include "utils/guc.h"

#if PG_VERSION_NUM != 170011
#error This diagnostic is qualified against PostgreSQL 17.11.
#endif

PG_MODULE_MAGIC;
void _PG_init(void);

#define OUTPUT_DIR "mesh-postflush"
#define CAPACITY 4096
#define NS_PER_SEC INT64CONST(1000000000)

typedef struct CommitSample
{
	uint64 xid;
	int64 realtime_ns;
} CommitSample;

static CommitSample samples[CAPACITY];
static unsigned int used;
static uint64 dropped;
static uint64 clock_errors;
static uint64 asynchronous;
static FullTransactionId pending;
static bool enabled;
static int64 started_ns;

static int64
realtime_ns(void)
{
	struct timespec now;

	if (clock_gettime(CLOCK_REALTIME, &now) != 0 ||
		now.tv_sec <= 0 || now.tv_sec >= PG_INT64_MAX / NS_PER_SEC ||
		now.tv_nsec < 0 || now.tv_nsec >= NS_PER_SEC)
		return -1;
	return (int64) now.tv_sec * NS_PER_SEC + now.tv_nsec;
}

static void
observe_xact(XactEvent event, void *arg)
{
	int64 stamp;

	switch (event)
	{
		case XACT_EVENT_PRE_COMMIT:
			pending = GetTopFullTransactionIdIfAny();
			return;
		case XACT_EVENT_COMMIT:
			if (!FullTransactionIdIsValid(pending))
				return;
			if (used == CAPACITY)
				dropped++;
			else
			{
				stamp = realtime_ns();
				if (stamp < 0 ||
					(used > 0 && stamp < samples[used - 1].realtime_ns))
					clock_errors++;
				else
				{
					samples[used].xid = U64FromFullTransactionId(pending);
					samples[used++].realtime_ns = stamp;
				}
				if (synchronous_commit != SYNCHRONOUS_COMMIT_ON)
					asynchronous++;
			}
			break;
		default:
			break;
	}
	pending = InvalidFullTransactionId;
}

static void
export_samples(int code, Datum arg)
{
	char path[MAXPGPATH];
	FILE *file;
	int64 ended_ns = realtime_ns();
	bool ok;
	unsigned int i;

	/* PostgreSQL invokes this during backend exit. The directory lives in PGDATA. */
	snprintf(path, sizeof(path), "%s/%s/%d.log", DataDir, OUTPUT_DIR, MyProcPid);
	file = fopen(path, "wx");
	if (file == NULL)
	{
		elog(LOG, "postflush evidence could not be opened: %m");
		return;
	}

	ok = fprintf(file, "postflush-v1 %d %d %d " INT64_FORMAT " " INT64_FORMAT "\n",
				 MyProcPid, enabled, code, started_ns, ended_ns) > 0;
	for (i = 0; ok && i < used; i++)
		ok = fprintf(file, UINT64_FORMAT " " INT64_FORMAT "\n",
					 samples[i].xid, samples[i].realtime_ns) > 0;
	if (ok)
		ok = fprintf(file, "end %u " UINT64_FORMAT " " UINT64_FORMAT " " UINT64_FORMAT "\n",
					 used, dropped, clock_errors, asynchronous) > 0;
	if (fclose(file) != 0 || !ok)
	{
		unlink(path);
		elog(LOG, "postflush evidence export failed");
	}
}

void
_PG_init(void)
{
	char path[MAXPGPATH];
	struct stat info;

	if (!IsUnderPostmaster || MyBackendType != B_BACKEND ||
		strcmp(GetUserNameFromId(GetSessionUserId(), false), "compiled_caller") != 0)
		return;

	snprintf(path, sizeof(path), "%s/%s", DataDir, OUTPUT_DIR);
	if (lstat(path, &info) != 0 || !S_ISDIR(info.st_mode) ||
		info.st_uid != geteuid() || (info.st_mode & 077) != 0)
		elog(ERROR, "postflush requires a private backend-owned evidence directory in PGDATA");

	/* The controller sets this at role/database scope before any app connects. */
	DefineCustomBoolVariable("mesh_postflush.enabled", "Sample application commits.",
							 NULL, &enabled, false, PGC_SUSET, 0, NULL, NULL, NULL);
	MarkGUCPrefixReserved("mesh_postflush");
	started_ns = realtime_ns();
	on_proc_exit(export_samples, (Datum) 0);
	if (enabled)
		RegisterXactCallback(observe_xact, NULL);
}
