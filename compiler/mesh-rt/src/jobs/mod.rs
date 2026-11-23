mod cron;
mod drain_store;
#[cfg(any(mesh_jobs_effects, test))]
mod effect_pool;
#[cfg(mesh_jobs_effects)]
mod effects;
mod events;
mod runner;
mod store;
pub mod testing;
mod trusted;
mod worker;

#[cfg(target_os = "linux")]
pub(crate) use worker::apply_worker_startup_policy;

use crate::db::jobs_connection::{self, PgJobDb};
use crate::db::pg::JobsAckError;
use crate::io::{alloc_result, err_result, ok_int, MeshResult};
use crate::string::{mesh_str, text_of, MeshString};

impl store::Database for PgJobDb<'_> {
    fn query(&mut self, sql: &str, params: &[&str]) -> Result<Vec<Vec<(String, String)>>, String> {
        PgJobDb::query(self, sql, params)
    }

    fn simple(&mut self, sql: &str) -> Result<(), String> {
        PgJobDb::simple(self, sql)
    }

    fn acknowledge(
        &mut self,
        sql: &str,
        params: &[&str],
    ) -> Result<Vec<Vec<(String, String)>>, JobsAckError> {
        PgJobDb::acknowledge(self, sql, params)
    }

    fn in_transaction(&self) -> bool {
        PgJobDb::in_transaction(self)
    }
}

fn with_pool<T>(
    pool: u64,
    run: impl FnOnce(&mut PgJobDb<'_>) -> Result<T, String>,
) -> Result<T, String> {
    unsafe { jobs_connection::with_pool(pool, run) }
}

fn text_result(result: Result<String, String>) -> *mut MeshResult {
    match result {
        Ok(value) => alloc_result(0, mesh_str(&value).cast()),
        Err(error) => err_result(&error),
    }
}

pub(crate) fn shutdown_all() {
    runner::shutdown_all();
}

#[no_mangle]
pub extern "C" fn mesh_jobs_migrate(pool: u64) -> *mut MeshResult {
    match with_pool(pool, |db| {
        store::migrate(db)?;
        db.transaction(|tx| cron::migrate(tx))
    }) {
        Ok(()) => alloc_result(0, std::ptr::null_mut()),
        Err(error) => err_result(&error),
    }
}

#[no_mangle]
pub extern "C" fn mesh_jobs_enqueue(
    pool: u64,
    namespace: *const MeshString,
    spec_or_batch: *const MeshString,
) -> *mut MeshResult {
    let (namespace, specs) = unsafe { (text_of(namespace), text_of(spec_or_batch)) };
    text_result(with_pool(pool, |db| {
        db.transaction(|tx| store::enqueue(tx, namespace, specs))
    }))
}

#[no_mangle]
pub extern "C" fn mesh_jobs_enqueue_in(
    connection: u64,
    namespace: *const MeshString,
    spec_or_batch: *const MeshString,
) -> *mut MeshResult {
    let result = unsafe {
        let (namespace, specs) = (text_of(namespace), text_of(spec_or_batch));
        jobs_connection::with_connection(connection, |db| {
            if !db.in_transaction() {
                return Err("Jobs.enqueue_in requires an active PostgreSQL transaction".into());
            }
            store::enqueue(db, namespace, specs)
        })
    };
    text_result(result)
}

#[no_mangle]
pub extern "C" fn mesh_jobs_control(
    pool: u64,
    namespace: *const MeshString,
    action: *const MeshString,
) -> *mut MeshResult {
    let (namespace, action) = unsafe { (text_of(namespace), text_of(action)) };
    text_result(with_pool(pool, |db| {
        #[derive(serde::Deserialize)]
        struct Operation {
            #[serde(alias = "op")]
            action: String,
        }

        if action.len() > 1024 * 1024 {
            return Err("Jobs control JSON exceeds 1 MiB".into());
        }
        let operation: Operation = serde_json::from_str(action).map_err(|err| err.to_string())?;
        match operation.action.as_str() {
            "schedule" | "unschedule" | "list_schedules" => cron::control(db, namespace, action),
            _ => store::control(db, namespace, action),
        }
    }))
}

#[no_mangle]
pub extern "C" fn mesh_jobs_start(
    pool: u64,
    config: *const MeshString,
    fn_ptr: *const u8,
    env_ptr: *const u8,
) -> *mut MeshResult {
    match runner::start(pool, unsafe { text_of(config) }, fn_ptr, env_ptr) {
        Ok(id) => ok_int(id as i64),
        Err(error) => err_result(&error),
    }
}

#[no_mangle]
pub extern "C" fn mesh_jobs_start_effects_v1(
    pool: u64,
    config: *const MeshString,
    fn_ptr: *const u8,
    env_ptr: *const u8,
    app_url: *const MeshString,
    operations: *const crate::json::MeshJson,
) -> *mut MeshResult {
    #[cfg(mesh_jobs_effects)]
    {
        let result = unsafe {
            crate::json::bounded_json(operations, 6, effects::MAX_REGISTRY_BYTES).and_then(
                |operations| {
                    runner::start_effects(
                        pool,
                        text_of(config),
                        fn_ptr,
                        env_ptr,
                        text_of(app_url),
                        &operations,
                    )
                },
            )
        };
        match result {
            Ok(id) => ok_int(id as i64),
            Err(error) => err_result(&error),
        }
    }
    #[cfg(not(mesh_jobs_effects))]
    {
        let _ = (pool, config, fn_ptr, env_ptr, app_url, operations);
        err_result("Jobs effects require a compiled GNU/Linux Mesh program on ARM64 or x86-64")
    }
}

#[no_mangle]
pub extern "C" fn mesh_jobs_effect_v1(
    name: *const MeshString,
    values: *const crate::json::MeshJson,
) -> *mut MeshResult {
    #[cfg(mesh_jobs_effects)]
    {
        match unsafe { effects::call(text_of(name), values) } {
            Ok(applied) => alloc_result(0, crate::io::box_scalar(applied)),
            Err(error) => err_result(error),
        }
    }
    #[cfg(not(mesh_jobs_effects))]
    {
        let _ = (name, values);
        err_result("Jobs effects require a compiled GNU/Linux Mesh program on ARM64 or x86-64")
    }
}

#[no_mangle]
pub extern "C-unwind" fn mesh_jobs_drain(
    pool: u64,
    config: *const MeshString,
    fn_ptr: *const u8,
    env_ptr: *const u8,
    options: *const MeshString,
) -> *mut MeshResult {
    let (config, options) = unsafe { (text_of(config), text_of(options)) };
    text_result(runner::drain(pool, config, fn_ptr, env_ptr, options))
}

#[no_mangle]
pub extern "C-unwind" fn mesh_jobs_stop(runner: i64, grace_ms: i64) -> *mut MeshResult {
    if runner <= 0 || grace_ms < 0 {
        return err_result("Jobs.stop requires a valid runner and nonnegative grace period");
    }
    text_result(runner::stop(runner as u64, grace_ms as u64))
}

#[no_mangle]
pub extern "C-unwind" fn mesh_jobs_join(runner: i64) -> *mut MeshResult {
    if runner <= 0 {
        return err_result("invalid Jobs runner");
    }
    text_result(runner::join(runner as u64))
}

#[no_mangle]
pub extern "C" fn mesh_jobs_status(runner: i64) -> *mut MeshResult {
    if runner <= 0 {
        return err_result("invalid Jobs runner");
    }
    text_result(runner::status(runner as u64))
}

#[no_mangle]
pub extern "C" fn mesh_jobs_read_events(runner: i64, limit: i64) -> *mut MeshResult {
    if runner <= 0 {
        return err_result("invalid Jobs runner");
    }
    text_result(runner::read_events(runner as u64, limit))
}

#[no_mangle]
pub extern "C" fn mesh_jobs_local_control(
    runner: i64,
    action: *const MeshString,
) -> *mut MeshResult {
    if runner <= 0 {
        return err_result("invalid Jobs runner");
    }
    text_result(runner::control(runner as u64, unsafe { text_of(action) }))
}
