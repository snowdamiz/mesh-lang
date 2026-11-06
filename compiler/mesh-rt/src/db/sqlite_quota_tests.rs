use super::*;
use crate::bytes::MeshBytes;
use crate::collections::list::{mesh_list_append, mesh_list_get, mesh_list_length, mesh_list_new};
use crate::collections::map::mesh_map_get;
use crate::gc::mesh_rt_init;
use crate::io::MeshResult;

struct Connection(u64);
impl Connection {
    fn new() -> Self {
        mesh_rt_init();
        Self(unsafe { *(success(mesh_sqlite_open(mesh_str(":memory:"))) as *const u64) })
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        mesh_sqlite_close(self.0);
    }
}

fn success(value: *mut u8) -> *mut u8 {
    let result = unsafe { &*(value as *const MeshResult) };
    assert_eq!(
        result.tag,
        0,
        "SQLite call refused: {}",
        if result.tag == 1 {
            unsafe { text_of(result.value as *const MeshString) }
        } else {
            ""
        }
    );
    result.value
}

fn refused(value: *mut u8) -> String {
    let result = unsafe { &*(value as *const MeshResult) };
    assert_eq!(result.tag, 1, "SQLite call unexpectedly succeeded");
    unsafe { text_of(result.value as *const MeshString).to_owned() }
}

fn parameters(bytes: &[u8], copies: usize) -> *mut u8 {
    let data = mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64);
    let value = unsafe { alloc_db_value(DB_VALUE_BINARY, data.cast()) } as u64;
    (0..copies).fold(mesh_list_new(), |list, _| mesh_list_append(list, value))
}

fn binary_result(value: *mut u8) -> &'static [u8] {
    let rows = success(value);
    assert_eq!(mesh_list_length(rows), 1);
    let row = mesh_list_get(rows, 0) as *mut u8;
    let value = mesh_map_get(row, mesh_str("payload") as u64) as *const MeshDbValue;
    unsafe {
        assert_eq!((*value).tag, DB_VALUE_BINARY);
        (&*((*value).payload as *const MeshBytes)).as_slice()
    }
}

#[test]
fn explicit_sqlite_quota_round_trips_full_encrypted_archive_extent() {
    let db = Connection::new();
    success(mesh_sqlite_execute(
        db.0,
        mesh_str("CREATE TABLE archive(payload BLOB NOT NULL) STRICT"),
        mesh_list_new(),
    ));
    let bytes: Vec<u8> = (0..16_797_311).map(|index| (index % 251) as u8).collect();
    let args = parameters(&bytes, 1);
    assert!(refused(mesh_sqlite_execute_values(
        db.0,
        mesh_str("INSERT INTO archive VALUES (?)"),
        args
    ))
    .contains("16777216 byte limit"));
    success(mesh_sqlite_execute_values_v2(
        db.0,
        mesh_str("INSERT INTO archive VALUES (?)"),
        args,
        bytes.len() as i64,
    ));
    assert_eq!(
        binary_result(mesh_sqlite_query_values_v2(
            db.0,
            mesh_str("SELECT payload FROM archive"),
            mesh_list_new(),
            bytes.len() as i64
        )),
        bytes
    );
    assert!(refused(mesh_sqlite_query_values_v2(
        db.0,
        mesh_str("SELECT payload FROM archive"),
        mesh_list_new(),
        bytes.len() as i64 - 1
    ))
    .contains("byte limit"));
    assert!(refused(mesh_sqlite_query_values(
        db.0,
        mesh_str("SELECT payload FROM archive"),
        mesh_list_new()
    ))
    .contains("16777216 byte limit"));
}

#[test]
fn explicit_sqlite_quota_reads_the_inclusive_public_maximum() {
    let db = Connection::new();
    let rows = mesh_sqlite_query_values_v2(
        db.0,
        mesh_str("SELECT zeroblob(33554432) AS payload"),
        mesh_list_new(),
        33_554_432,
    );
    let bytes = binary_result(rows);
    assert_eq!(bytes.len(), 33_554_432);
    assert!(bytes.iter().all(|byte| *byte == 0));
}

#[test]
fn invalid_sqlite_quotas_refuse_before_statement_mutation() {
    let db = Connection::new();
    success(mesh_sqlite_execute(
        db.0,
        mesh_str("CREATE TABLE changes(value INTEGER)"),
        mesh_list_new(),
    ));
    for limit in [-1, 0, 33_554_433, i64::MAX] {
        assert!(refused(mesh_sqlite_execute_values_v2(
            db.0,
            mesh_str("INSERT INTO changes VALUES (1)"),
            mesh_list_new(),
            limit
        ))
        .contains("quota"));
        assert!(refused(mesh_sqlite_query_values_v2(
            db.0,
            mesh_str("INSERT INTO changes VALUES (2) RETURNING value"),
            mesh_list_new(),
            limit
        ))
        .contains("quota"));
    }
    let rows = success(mesh_sqlite_query_values(
        db.0,
        mesh_str("SELECT value FROM changes"),
        mesh_list_new(),
    ));
    assert_eq!(mesh_list_length(rows), 0);
}

#[test]
fn sqlite_v2_parameter_budget_refuses_before_binding_or_writes() {
    let db = Connection::new();
    success(mesh_sqlite_execute(
        db.0,
        mesh_str("CREATE TABLE changes(a BLOB,b BLOB,c BLOB)"),
        mesh_list_new(),
    ));
    let args = parameters(&vec![7; 22_369_622], 3);
    assert!(refused(mesh_sqlite_execute_values_v2(
        db.0,
        mesh_str("INSERT INTO changes VALUES (?,?,?)"),
        args,
        33_554_432
    ))
    .contains("aggregate"));
    let rows = success(mesh_sqlite_query_values(
        db.0,
        mesh_str("SELECT a FROM changes"),
        mesh_list_new(),
    ));
    assert_eq!(mesh_list_length(rows), 0);
}
