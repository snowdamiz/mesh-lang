//! Actual global-table preparation path for the new immutable epoch-key role.
use super::*;
use crate::actor::{ExitReason, Priority};
use std::time::Duration;
fn setup(id: u64) -> Process {
    crate::mesh_rt_init();
    Process::new(ProcessId(id), Priority::Normal)
}
fn source(p: &mut Process, n: usize) -> *mut MeshSecretHandle {
    insert_owned_resource(
        p,
        ResourceKind::MlKemPrivateKey,
        ProtectedBytes::zeroed(n).unwrap(),
    )
    .unwrap()
}
#[test]
fn c1_epoch_prepare_allocates_outside_actual_table_and_keeps_borrow() {
    let mut p = setup(915_231);
    let h = source(&mut p, 64);
    let prepared = prepare_braid_epoch_key_with_allocator(&mut p, h, |owner, n| {
        let table = secret_table().try_lock_for(Duration::from_secs(5));
        assert!(
            table.is_some(),
            "protected allocation ran under global table"
        );
        drop(table);
        assert_eq!(owned_secret_count_for_test(owner.pid), 1);
        assert_eq!(n, 64);
        ProtectedBytes::zeroed(n).map_err(Into::into)
    })
    .unwrap();
    validate_prepared_braid_epoch_key(&p, &prepared).unwrap();
    assert_eq!(owned_secret_count_for_test(p.pid), 1);
    destroy_owned(p.pid);
}
#[test]
fn c1_epoch_prepare_allocation_failure_preserves_valid_borrow() {
    let mut p = setup(915_232);
    let h = source(&mut p, 64);
    assert_eq!(
        prepare_braid_epoch_key_with_allocator(&mut p, h, |_, _| Err(
            ResourceError::ProtectionUnavailable
        ))
        .err(),
        Some(ResourceError::ProtectionUnavailable)
    );
    assert_eq!(owned_secret_count_for_test(p.pid), 1);
    assert!(prepare_braid_epoch_key(&mut p, h).is_ok());
    assert_eq!(
        prepare_braid_epoch_key_with_allocator(&mut p, h, |_, n| ProtectedBytes::zeroed(n - 1)
            .map_err(Into::into))
        .err(),
        Some(ResourceError::WrongKind)
    );
    assert!(prepare_braid_epoch_key(&mut p, h).is_ok());
    destroy_owned(p.pid);
}
#[test]
fn c1_epoch_prepare_rechecks_exit_generation_and_width_before_copy() {
    for mode in 0..3 {
        let mut p = setup(915_233);
        let h = source(&mut p, 64);
        let result = prepare_braid_epoch_key_with_allocator(&mut p, h, |owner, n| {
            let allocated = ProtectedBytes::zeroed(n).map_err(ResourceError::from)?;
            if mode == 0 {
                assert!(owner.mark_exited(ExitReason::Killed));
            } else {
                let handle = validate_handle_pointer(owner, h).unwrap();
                let mut table = secret_table().lock();
                if mode == 1 {
                    drop(table.consume(owner.pid, handle, ResourceKind::MlKemPrivateKey)?);
                } else {
                    table.replace_resource_bytes(
                        owner.pid,
                        handle,
                        ResourceKind::MlKemPrivateKey,
                        ProtectedBytes::zeroed(n + 1).map_err(ResourceError::from)?,
                    )?;
                }
            }
            Ok(allocated)
        });
        assert_eq!(
            result.err(),
            Some(if mode == 0 {
                ResourceError::OwnerExited
            } else {
                ResourceError::StaleHandle
            })
        );
        destroy_owned(p.pid);
    }
}
#[test]
fn c1_epoch_wrong_width_kind_and_same_handle_mutation_refuse() {
    let mut p = setup(915_234);
    for n in [0, 63, 65, 2400] {
        let h = source(&mut p, n);
        let mut allocated = false;
        assert_eq!(
            prepare_braid_epoch_key_with_allocator(&mut p, h, |_, n| {
                allocated = true;
                ProtectedBytes::zeroed(n).map_err(Into::into)
            })
            .err(),
            Some(ResourceError::WrongKind)
        );
        assert!(!allocated);
    }
    let wrong = insert_owned_resource(
        &mut p,
        ResourceKind::SecretBytes,
        ProtectedBytes::zeroed(64).unwrap(),
    )
    .unwrap();
    assert_eq!(
        prepare_braid_epoch_key(&mut p, wrong).err(),
        Some(ResourceError::WrongKind)
    );
    let h = source(&mut p, 64);
    let captured = prepare_braid_epoch_key(&mut p, h).unwrap();
    let handle = validate_handle_pointer(&p, h).unwrap();
    secret_table()
        .lock()
        .replace_resource_bytes(
            p.pid,
            handle,
            ResourceKind::MlKemPrivateKey,
            ProtectedBytes::copy_from(&[1; 64]).unwrap(),
        )
        .unwrap();
    assert_eq!(
        validate_prepared_braid_epoch_key(&p, &captured).err(),
        Some(ResourceError::StaleHandle)
    );
    destroy_owned(p.pid);
}
