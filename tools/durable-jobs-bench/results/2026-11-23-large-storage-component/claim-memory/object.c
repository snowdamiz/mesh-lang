#include "postgres.h"
#include "fmgr.h"
#include "varatt.h"
#include "utils/jsonb.h"

PG_MODULE_MAGIC;
PG_FUNCTION_INFO_V1(mesh_jsonb_is_object);

Datum
mesh_jsonb_is_object(PG_FUNCTION_ARGS)
{
    Datum input = PG_GETARG_DATUM(0);
    struct varlena *value = (struct varlena *) DatumGetPointer(input);
    uint32 header;

    if (VARATT_IS_EXTERNAL(value) || VARATT_IS_COMPRESSED(value))
        value = PG_DETOAST_DATUM_SLICE(input, 0, sizeof(header));
    memcpy(&header, VARDATA_ANY(value), sizeof(header));
    PG_FREE_IF_COPY(value, 0);
    PG_RETURN_BOOL((header & JB_FOBJECT) != 0);
}
