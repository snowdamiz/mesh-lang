//! The new role's preparation tests use the actual process/global table path.
use super::*;
use crate::actor::{ExitReason, Priority};
use std::time::Duration;
fn material() -> Vec<u8> {
    let mut bytes = vec![0, 3, 0, 2];
    for name in [b"root".as_slice(), b"mac".as_slice()] {
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&32u32.to_be_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&[0x5a; 32]);
    }
    bytes
}
fn source(p: &mut Process) -> *mut MeshSecretHandle {
    insert_owned_resource(
        p,
        ResourceKind::SecretMap,
        ProtectedBytes::copy_from(&material()).unwrap(),
    )
    .unwrap()
}
fn setup(id: u64) -> Process {
    crate::mesh_rt_init();
    Process::new(ProcessId(id), Priority::Normal)
}
#[test]
fn braid_map_prepare_allocator_runs_unlocked_and_preserves_borrow() {
    let mut p = setup(915_221);
    let h = source(&mut p);
    let v = prepare_braid_map_with_allocator(&mut p, h, |owner, n| {
        let guard = secret_table().try_lock_for(Duration::from_secs(5));
        assert!(guard.is_some());
        drop(guard);
        assert_eq!(owned_secret_count_for_test(owner.pid), 1);
        ProtectedBytes::zeroed(n).map_err(Into::into)
    })
    .unwrap();
    assert_eq!(&*v.bytes, material());
    validate_prepared_owned_resource(&p, &v, ResourceKind::SecretMap).unwrap();
    destroy_owned(p.pid);
}
#[test]
fn braid_map_prepare_allocation_failure_preserves_live_borrow() {
    let mut p = setup(915_222);
    let h = source(&mut p);
    assert!(matches!(
        prepare_braid_map_with_allocator(&mut p, h, |_, _| Err(
            ResourceError::ProtectionUnavailable
        )),
        Err(ResourceError::ProtectionUnavailable)
    ));
    assert_eq!(owned_secret_count_for_test(p.pid), 1);
    assert!(prepare_braid_responder_map(&mut p, h).is_ok());
    destroy_owned(p.pid);
}
#[test]
fn braid_map_prepare_rechecks_actor_generation_and_length_before_copy() {
    for mode in 0..3 {
        let mut p = setup(915_223);
        let h = source(&mut p);
        let e = prepare_braid_map_with_allocator(&mut p, h, |owner, n| {
            let allocated = ProtectedBytes::zeroed(n).map_err(ResourceError::from)?;
            if mode == 0 {
                assert!(owner.mark_exited(ExitReason::Killed));
            } else {
                let handle = validate_handle_pointer(owner, h).unwrap();
                let mut table = secret_table().lock();
                if mode == 1 {
                    drop(table.consume(owner.pid, handle, ResourceKind::SecretMap)?);
                } else {
                    table.replace_resource_bytes(
                        owner.pid,
                        handle,
                        ResourceKind::SecretMap,
                        ProtectedBytes::zeroed(n + 1).map_err(ResourceError::from)?,
                    )?;
                }
            }
            Ok(allocated)
        })
        .err()
        .unwrap();
        assert_eq!(
            e,
            if mode == 0 {
                ResourceError::OwnerExited
            } else {
                ResourceError::StaleHandle
            }
        );
        destroy_owned(p.pid);
    }
}

#[test]
fn braid_map_seal_revalidation_detects_same_handle_same_length_mutation() {
    let mut p = setup(915_224);
    let h = source(&mut p);
    let prepared = prepare_braid_responder_map(&mut p, h).unwrap();
    let handle = validate_handle_pointer(&p, h).unwrap();
    secret_table()
        .lock()
        .replace_resource_bytes(p.pid, handle, ResourceKind::SecretMap, {
            let mut changed = material();
            *changed.last_mut().unwrap() ^= 1;
            ProtectedBytes::copy_from(&changed).unwrap()
        })
        .unwrap();
    assert_eq!(
        validate_prepared_braid_map(&p, &prepared).err(),
        Some(ResourceError::StaleHandle)
    );
    destroy_owned(p.pid);
}

#[test]
fn braid_map_prepare_rejects_unbounded_legacy_map_before_allocating() {
    let mut p = setup(915_225);
    let h = insert_owned_resource(
        &mut p,
        ResourceKind::SecretMap,
        ProtectedBytes::zeroed(4096).unwrap(),
    )
    .unwrap();
    let mut allocated = false;
    assert_eq!(
        prepare_braid_map_with_allocator(&mut p, h, |_, n| {
            allocated = true;
            ProtectedBytes::zeroed(n).map_err(Into::into)
        })
        .err(),
        Some(ResourceError::WrongKind)
    );
    assert!(!allocated);
    assert_eq!(owned_secret_count_for_test(p.pid), 1);
    destroy_owned(p.pid);
}
