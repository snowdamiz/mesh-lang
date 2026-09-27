//! Changeset validation pipeline for the Mesh runtime.
//!
//! Provides an opaque Changeset struct that accumulates validated changes
//! and errors. Each validator returns a new changeset (a changeset is never
//! changed in place), carrying the error it found if the field has none
//! yet. This enables pipe-chain composition where all validators run without
//! short-circuiting.
//!
//! ## Changeset object layout (4 slots, 32 bytes)
//!
//! | Slot | Offset | Name        | Type                          |
//! |------|--------|-------------|-------------------------------|
//! |  0   |   0    | data        | *mut u8 (Map<String,String>)  |
//! |  1   |   8    | changes     | *mut u8 (Map<String,String>)  |
//! |  2   |  16    | errors      | *mut u8 (Map<String,String>)  |
//! |  3   |  24    | valid       | i64: 1 = valid, 0 = invalid   |

use crate::collections::list::list_strings;
use crate::collections::list::mesh_list_new;
use crate::collections::map::{
    mesh_map_get, mesh_map_has_key, mesh_map_new_typed, mesh_map_put, mesh_map_size,
};
use crate::gc::mesh_gc_alloc_actor;
use crate::string::mesh_str;
use crate::string::text_of;

// ── Constants ────────────────────────────────────────────────────────

const CS_SLOTS: usize = 4;
const CS_SIZE: usize = CS_SLOTS * 8; // 32 bytes

const SLOT_DATA: usize = 0;
const SLOT_CHANGES: usize = 1;
const SLOT_ERRORS: usize = 2;
const SLOT_VALID: usize = 3;

// ── Slot access helpers ──────────────────────────────────────────────

unsafe fn cs_get(cs: *mut u8, slot: usize) -> *mut u8 {
    *(cs.add(slot * 8) as *const *mut u8)
}

unsafe fn cs_set(cs: *mut u8, slot: usize, val: *mut u8) {
    *(cs.add(slot * 8) as *mut *mut u8) = val;
}

unsafe fn cs_get_int(cs: *mut u8, slot: usize) -> i64 {
    *(cs.add(slot * 8) as *const i64)
}

unsafe fn cs_set_int(cs: *mut u8, slot: usize, val: i64) {
    *(cs.add(slot * 8) as *mut i64) = val;
}

/// The text `map` holds for `key`, if it holds one.
unsafe fn text_at(map: *mut u8, key: &str) -> Option<&'static str> {
    let key = mesh_str(key) as u64;
    (mesh_map_has_key(map, key) != 0).then(|| text_of(mesh_map_get(map, key) as *mut u8))
}

// ── Type coercion ────────────────────────────────────────────────────

fn coerce_value(val: &str, sql_type: &str) -> Result<String, ()> {
    match sql_type {
        "TEXT" => Ok(val.to_string()),
        "BIGINT" => val
            .trim()
            .parse::<i64>()
            .map(|v| v.to_string())
            .map_err(|_| ()),
        "DOUBLE PRECISION" => val
            .trim()
            .parse::<f64>()
            .map(|v| v.to_string())
            .map_err(|_| ()),
        "BOOLEAN" => match val.to_lowercase().as_str() {
            "true" | "t" | "1" | "yes" => Ok("true".to_string()),
            "false" | "f" | "0" | "no" => Ok("false".to_string()),
            _ => Err(()),
        },
        _ => Ok(val.to_string()), // unknown type -- pass through
    }
}

// ── Cast functions ───────────────────────────────────────────────────

/// Changeset.cast(data, params, allowed) -- 3-arg version, no type coercion.
///
/// Filters `params` to only include keys present in `allowed` list.
/// Creates a new changeset with the filtered params as `changes`.
#[no_mangle]
pub extern "C" fn mesh_changeset_cast(data: *mut u8, params: *mut u8, allowed: *mut u8) -> *mut u8 {
    mesh_changeset_cast_with_types(data, params, allowed, mesh_list_new())
}

/// Changeset.cast_with_types(data, params, allowed, field_types) -- 4-arg version with coercion.
///
/// Same as cast but additionally coerces string values based on SQL type metadata.
/// field_types is a List<String> of "field_name:SQL_TYPE" entries; a value
/// that does not coerce is the error "is invalid" instead of a change.
#[no_mangle]
pub extern "C" fn mesh_changeset_cast_with_types(
    data: *mut u8,
    params: *mut u8,
    allowed: *mut u8,
    field_types: *mut u8,
) -> *mut u8 {
    unsafe {
        // Build field_type lookup from "field:SQL_TYPE" entries
        let ft_entries = list_strings(field_types);
        let type_map: std::collections::HashMap<&str, &str> = ft_entries
            .iter()
            .filter_map(|entry| entry.split_once(':'))
            .collect();

        let mut changes = mesh_map_new_typed(1);
        let mut errors = mesh_map_new_typed(1);
        for field_name in list_strings(allowed) {
            let Some(value) = text_at(params, &field_name) else {
                continue;
            };
            let key = mesh_str(&field_name) as u64;
            match type_map
                .get(field_name.as_str())
                .map_or(Ok(value.to_string()), |sql_type| {
                    coerce_value(value, sql_type)
                }) {
                Ok(coerced) => changes = mesh_map_put(changes, key, mesh_str(&coerced) as u64),
                Err(()) => errors = mesh_map_put(errors, key, mesh_str("is invalid") as u64),
            }
        }

        let cs = mesh_gc_alloc_actor(CS_SIZE as u64, 8);
        cs_set(cs, SLOT_DATA, data);
        cs_set(cs, SLOT_CHANGES, changes);
        cs_set(cs, SLOT_ERRORS, errors);
        cs_set_int(cs, SLOT_VALID, (mesh_map_size(errors) == 0) as i64);
        cs
    }
}

// ── Validators ───────────────────────────────────────────────────────

/// `cs` with `message` as `field`'s error, unless it has one already: a new
/// changeset, invalid.
pub(crate) unsafe fn add_error(cs: *mut u8, field: &str, message: &str) -> *mut u8 {
    let errors = cs_get(cs, SLOT_ERRORS);
    let key = mesh_str(field) as u64;
    if mesh_map_has_key(errors, key) != 0 {
        return cs;
    }
    let new_cs = mesh_gc_alloc_actor(CS_SIZE as u64, 8);
    std::ptr::copy_nonoverlapping(cs, new_cs, CS_SIZE);
    cs_set(
        new_cs,
        SLOT_ERRORS,
        mesh_map_put(errors, key, mesh_str(message) as u64),
    );
    cs_set_int(new_cs, SLOT_VALID, 0);
    new_cs
}

/// `cs` with the error `check` finds in `field`'s change, if any; a field
/// without a change is not checked.
unsafe fn validate_change(
    cs: *mut u8,
    field: *mut u8,
    check: impl FnOnce(&str) -> Option<String>,
) -> *mut u8 {
    let field = text_of(field);
    match text_at(cs_get(cs, SLOT_CHANGES), field).and_then(check) {
        Some(message) => add_error(cs, field, &message),
        None => cs,
    }
}

/// Changeset.validate_required(changeset, fields_list)
///
/// Checks that each field in fields_list has a non-empty value in `changes`
/// or, when it has no change, in `data`. Adds "can't be blank" error for missing fields.
#[no_mangle]
pub extern "C" fn mesh_changeset_validate_required(cs: *mut u8, fields: *mut u8) -> *mut u8 {
    unsafe {
        let (changes, data) = (cs_get(cs, SLOT_CHANGES), cs_get(cs, SLOT_DATA));
        list_strings(fields).iter().fold(cs, |cs, field| {
            let value = text_at(changes, field).or_else(|| text_at(data, field));
            match value.is_some_and(|value| !value.is_empty()) {
                true => cs,
                false => add_error(cs, field, "can't be blank"),
            }
        })
    }
}

/// Changeset.validate_length(changeset, field, min, max)
///
/// Checks that the field value's length in characters is within [min, max].
/// Use -1 for "not set" (no bound). Only validates fields present in changes.
#[no_mangle]
pub extern "C" fn mesh_changeset_validate_length(
    cs: *mut u8,
    field: *mut u8,
    min: i64,
    max: i64,
) -> *mut u8 {
    unsafe {
        validate_change(cs, field, |value| {
            let len = value.chars().count() as i64;
            if min != -1 && len < min {
                Some(format!("should be at least {} character(s)", min))
            } else if max != -1 && len > max {
                Some(format!("should be at most {} character(s)", max))
            } else {
                None
            }
        })
    }
}

/// Changeset.validate_format(changeset, field, pattern)
///
/// Checks that the field value contains the pattern substring.
/// Adds "has invalid format" error if pattern is not found.
#[no_mangle]
pub extern "C" fn mesh_changeset_validate_format(
    cs: *mut u8,
    field: *mut u8,
    pattern: *mut u8,
) -> *mut u8 {
    unsafe {
        let pattern = text_of(pattern);
        validate_change(cs, field, |value| {
            (!value.contains(pattern)).then(|| "has invalid format".to_string())
        })
    }
}

/// Changeset.validate_inclusion(changeset, field, allowed_values_list)
///
/// Checks that the field value is one of the allowed values.
/// Adds "is invalid" error if not found in the list.
#[no_mangle]
pub extern "C" fn mesh_changeset_validate_inclusion(
    cs: *mut u8,
    field: *mut u8,
    allowed_values: *mut u8,
) -> *mut u8 {
    unsafe {
        let allowed = list_strings(allowed_values);
        validate_change(cs, field, |value| {
            (!allowed.iter().any(|a| a == value)).then(|| "is invalid".to_string())
        })
    }
}

/// Changeset.validate_number(changeset, field, gt, lt, gte, lte)
///
/// Checks that the field value (parsed as i64) is within the specified bounds.
/// Use -1 for "not set" (no bound). Adds appropriate error messages.
#[no_mangle]
pub extern "C" fn mesh_changeset_validate_number(
    cs: *mut u8,
    field: *mut u8,
    gt: i64,
    lt: i64,
    gte: i64,
    lte: i64,
) -> *mut u8 {
    unsafe {
        validate_change(cs, field, |value| match value.trim().parse::<i64>() {
            Err(_) => Some("is not a number".to_string()),
            Ok(num) if gt != -1 && num <= gt => Some(format!("must be greater than {}", gt)),
            Ok(num) if lt != -1 && num >= lt => Some(format!("must be less than {}", lt)),
            Ok(num) if gte != -1 && num < gte => {
                Some(format!("must be greater than or equal to {}", gte))
            }
            Ok(num) if lte != -1 && num > lte => {
                Some(format!("must be less than or equal to {}", lte))
            }
            Ok(_) => None,
        })
    }
}

// ── Field accessors ──────────────────────────────────────────────────

/// Changeset.valid(changeset) -> Bool
///
/// Returns 1 (true) if changeset has no errors, 0 (false) otherwise.
/// Return type is i64 cast to *mut u8, matching the Bool convention.
#[no_mangle]
pub extern "C" fn mesh_changeset_valid(cs: *mut u8) -> *mut u8 {
    unsafe { cs_get_int(cs, SLOT_VALID) as *mut u8 }
}

/// Changeset.errors(changeset) -> Map<String,String>
///
/// Returns the errors map.
#[no_mangle]
pub extern "C" fn mesh_changeset_errors(cs: *mut u8) -> *mut u8 {
    unsafe { cs_get(cs, SLOT_ERRORS) }
}

/// Changeset.changes(changeset) -> Map<String,String>
///
/// Returns the changes map.
#[no_mangle]
pub extern "C" fn mesh_changeset_changes(cs: *mut u8) -> *mut u8 {
    unsafe { cs_get(cs, SLOT_CHANGES) }
}

/// `field`'s entry in the map in `slot`, or "" if it has none.
unsafe fn entry_or_empty(cs: *mut u8, slot: usize, field: *mut u8) -> *mut u8 {
    let (map, key) = (cs_get(cs, slot), mesh_str(text_of(field)) as u64);
    match mesh_map_has_key(map, key) {
        0 => mesh_str("") as *mut u8,
        _ => mesh_map_get(map, key) as *mut u8,
    }
}

/// Changeset.get_change(changeset, field) -> String
///
/// Returns the value of a field from the changes map, or empty string if not found.
#[no_mangle]
pub extern "C" fn mesh_changeset_get_change(cs: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { entry_or_empty(cs, SLOT_CHANGES, field) }
}

/// Changeset.get_error(changeset, field) -> String
///
/// Returns the error message for a field, or empty string if no error.
#[no_mangle]
pub extern "C" fn mesh_changeset_get_error(cs: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { entry_or_empty(cs, SLOT_ERRORS, field) }
}

// ── Constraint-to-changeset error mapping ───────────────────────────

/// Map a PostgreSQL SQLSTATE code and constraint name to a (field, message) pair.
///
/// Handles:
/// - `23505` (unique_violation): "has already been taken"
/// - `23503` (foreign_key_violation): "does not exist"
/// - `23502` (not_null_violation): "can't be blank"
///
/// Returns `None` for unknown SQLSTATE codes.
pub(crate) fn map_constraint_error(
    sqlstate: &str,
    constraint: &str,
    table: &str,
    column: &str,
) -> Option<(String, String)> {
    match sqlstate {
        "23505" => {
            // unique_violation
            let field = extract_field_from_constraint(constraint, table)
                .unwrap_or_else(|| "_base".to_string());
            Some((field, "has already been taken".to_string()))
        }
        "23503" => {
            // foreign_key_violation
            let field = extract_field_from_constraint(constraint, table)
                .unwrap_or_else(|| "_base".to_string());
            Some((field, "does not exist".to_string()))
        }
        "23502" => {
            // not_null_violation
            if !column.is_empty() {
                Some((column.to_string(), "can't be blank".to_string()))
            } else {
                Some(("_base".to_string(), "can't be blank".to_string()))
            }
        }
        _ => None,
    }
}

/// Extract a field name from a PostgreSQL constraint name using naming conventions.
///
/// PostgreSQL constraint names follow these conventions:
/// - `{table}_{column}_key` for unique constraints (e.g., "users_email_key" -> "email")
/// - `{table}_{column}_fkey` for foreign keys (e.g., "posts_user_id_fkey" -> "user_id")
/// - `{table}_pkey` for primary key (e.g., "users_pkey" -> None)
/// - `{table}_{column}_check` for check constraints
///
/// Returns the extracted field name, or None if the constraint name doesn't match.
pub(crate) fn extract_field_from_constraint(
    constraint_name: &str,
    table_name: &str,
) -> Option<String> {
    // Strip the {table}_ prefix
    let prefix = format!("{}_", table_name);
    let remainder = constraint_name.strip_prefix(&prefix)?;

    // Try each known suffix
    for suffix in &["_key", "_fkey", "_pkey", "_check"] {
        if let Some(field) = remainder.strip_suffix(suffix) {
            if field.is_empty() {
                return None; // e.g., "users_pkey" -> empty field
            }
            return Some(field.to_string());
        }
    }

    // No known suffix matched -- return None
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::list::string_list;

    fn text(s: &str) -> *mut u8 {
        mesh_str(s) as *mut u8
    }

    /// A `Map<String, String>` of `entries`.
    fn string_map(entries: &[(&str, &str)]) -> *mut u8 {
        entries
            .iter()
            .fold(mesh_map_new_typed(1), |map, (key, value)| {
                mesh_map_put(map, mesh_str(key) as u64, mesh_str(value) as u64)
            })
    }

    /// A changeset whose changes are `entries`.
    fn changed(entries: &[(&str, &str)]) -> *mut u8 {
        let fields: Vec<&str> = entries.iter().map(|(key, _)| *key).collect();
        mesh_changeset_cast(string_map(&[]), string_map(entries), string_list(&fields))
    }

    fn error_of(cs: *mut u8, field: &str) -> String {
        unsafe { text_of(mesh_changeset_get_error(cs, text(field))) }.to_string()
    }

    /// A length is in characters, as its message says: "héllo" is five
    /// long, though six bytes.
    #[test]
    fn validate_length_counts_characters() {
        crate::gc::mesh_rt_init();
        let cs = changed(&[("name", "héllo"), ("city", "日本")]);
        let cs = mesh_changeset_validate_length(cs, text("name"), -1, 5);
        let cs = mesh_changeset_validate_length(cs, text("city"), 3, -1);
        assert_eq!(error_of(cs, "name"), "");
        assert_eq!(error_of(cs, "city"), "should be at least 3 character(s)");
    }
}
