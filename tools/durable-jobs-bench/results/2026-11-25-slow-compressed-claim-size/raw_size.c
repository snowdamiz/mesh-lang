#include "postgres.h"
#include "fmgr.h"
#include "access/detoast.h"

PG_MODULE_MAGIC;

PG_FUNCTION_INFO_V1(mesh_jsonb_raw_size);
Datum
mesh_jsonb_raw_size(PG_FUNCTION_ARGS)
{
    PG_RETURN_INT64((int64) toast_raw_datum_size(PG_GETARG_DATUM(0)));
}
