//! Split behavior uses the production process wrapper/global
//! table path. No ResourceTable method consumes+allocates+copies a split.
use super::*;
use crate::actor::{ExitReason, Priority};
use std::time::Duration;
fn process(id: u64) -> Process {
    crate::mesh_rt_init();
    Process::new(ProcessId(id), Priority::Normal)
}
fn input(p: &mut Process, n: usize) -> *mut MeshSecretHandle {
    let known = (0..n).map(|i| i as u8).collect::<Vec<_>>();
    insert_owned_resource(
        p,
        ResourceKind::SecretBytes,
        Zeroizing::new(known.into_boxed_slice()),
    )
    .unwrap()
}
fn bytes(p: &Process, h: *const MeshSecretHandle) -> Vec<u8> {
    with_owned_resource(p, h, ResourceKind::SecretBytes, |v| v.to_vec()).unwrap()
}
fn stale(p: &Process, h: *const MeshSecretHandle) {
    assert!(matches!(
        with_owned_resource(p, h, ResourceKind::SecretBytes, |_| ()),
        Err(ResourceError::StaleHandle)
    ));
}
fn clean(id: ProcessId) {
    assert_eq!(owned_secret_count_for_test(id), 0);
    let t = secret_table().lock();
    assert!(t
        .usage
        .get(&id)
        .is_none_or(|u| u.secrets == 0 && u.bytes == 0));
}
fn assert_table_unlocked() {
    // A bounded acquisition detects this same thread incorrectly retaining the
    // lock, while allowing brief unrelated global-table activity in broad runs.
    let guard = secret_table().try_lock_for(Duration::from_secs(5));
    assert!(
        guard.is_some(),
        "protected allocator callback ran while split retained the global table lock"
    );
    drop(guard);
}
fn protected(_p: &mut Process, n: usize) -> Result<ProtectedBytes, ResourceError> {
    ProtectedBytes::zeroed(n).map_err(ResourceError::from)
}
#[test]
fn split_production_wrapper_exact_order_minimum_maximum_and_stale_input() {
    for (n, at) in [(2, 1), (64, 32), (65536, 1), (65536, 65535)] {
        let mut p = process(915_001);
        let source = input(&mut p, n);
        let expected = bytes(&p, source);
        // This is the ordinary wrapper, also used by mesh_secret_split_at.
        let (left, right) = split_for_process(&mut p, source, at).unwrap();
        assert_eq!(bytes(&p, left), expected[..at as usize]);
        assert_eq!(bytes(&p, right), expected[at as usize..]);
        stale(&p, source);
        assert_eq!(owned_secret_count_for_test(p.pid), 2);
        destroy_owned(p.pid);
        clean(p.pid);
    }
}
#[test]
fn split_production_core_invalid_index_consumes_without_calling_allocator() {
    for at in [i64::MIN, -1, 0, 32, 33, 65536, i64::MAX] {
        let mut p = process(915_002);
        let source = input(&mut p, 32);
        let e = split_for_process_with_allocator(&mut p, source, at, |_, _| {
            panic!("invalid index must precede allocation")
        })
        .unwrap_err();
        if at <= 0 || at > 65535 {
            assert_eq!(e.tag, CryptoErrorTag::InvalidLength);
            assert_eq!((e.expected, e.actual), (65535, at));
        } else {
            assert_eq!(e.tag, CryptoErrorTag::InvalidKey);
            assert_eq!((e.expected, e.actual), (0, 0));
        }
        stale(&p, source);
        clean(p.pid);
    }
}
#[test]
fn split_production_core_second_allocation_failure_has_no_half_pair() {
    let mut p = process(915_003);
    let source = input(&mut p, 32);
    let mut calls = 0;
    let e = split_for_process_with_allocator(&mut p, source, 16, |owner, n| {
        calls += 1;
        assert_table_unlocked();
        stale(owner, source);
        assert_eq!(owned_secret_count_for_test(owner.pid), 0);
        if calls == 2 {
            Err(ResourceError::ProtectionUnavailable)
        } else {
            protected(owner, n)
        }
    })
    .unwrap_err();
    assert_eq!(calls, 2);
    assert_eq!(e.tag, CryptoErrorTag::InternalFailure);
    stale(&p, source);
    clean(p.pid);
    // Same actual owner/table admits new work after failure; no half-pair or quota leak.
    let retry = input(&mut p, 32);
    let (left, right) = split_for_process(&mut p, retry, 16).unwrap();
    assert_eq!(bytes(&p, left).len() + bytes(&p, right).len(), 32);
    destroy_owned(p.pid);
    clean(p.pid);
}
#[test]
fn split_production_core_actor_exit_during_either_allocation_blocks_final_publication() {
    for exit_at in [1, 2] {
        let mut p = process(915_004);
        let source = input(&mut p, 64);
        let mut calls = 0;
        let e = split_for_process_with_allocator(&mut p, source, 32, |owner, n| {
            calls += 1;
            assert_table_unlocked();
            let allocated = protected(owner, n)?;
            if calls == exit_at {
                assert!(owner.mark_exited(ExitReason::Killed));
            }
            Ok(allocated)
        })
        .unwrap_err();
        // Both allocations succeeded; final same-lock owner check must win over quota.
        assert_eq!(calls, 2);
        assert_eq!(e.tag, CryptoErrorTag::SecretDestroyed);
        assert!(matches!(p.state, ProcessState::Exited(ExitReason::Killed)));
        clean(p.pid);
        assert!(matches!(
            with_owned_resource(&p, source, ResourceKind::SecretBytes, |_| ()),
            Err(ResourceError::OwnerExited)
        ));
    }
}
#[test]
fn split_production_core_rechecks_actual_owner_quota_after_unlocked_allocation() {
    let mut p = process(915_005);
    let source = input(&mut p, 64);
    let mut calls = 0;
    let e = split_for_process_with_allocator(&mut p, source, 32, |owner, n| {
        calls += 1;
        assert_table_unlocked();
        stale(owner, source);
        if calls == 2 {
            // Actual global production table, not a parallel local split model. Work
            // appearing between consume and commit must participate in final preflight.
            for _ in 0..MAX_SECRETS_PER_ACTOR - 1 {
                let one = ProtectedBytes::zeroed(1).map_err(ResourceError::from)?;
                let _ = insert_owned_resource(owner, ResourceKind::SecretBytes, one)?;
            }
        }
        protected(owner, n)
    })
    .unwrap_err();
    assert_eq!(calls, 2);
    assert_eq!(e.tag, CryptoErrorTag::ResourceLimitExceeded);
    stale(&p, source);
    assert_eq!(
        owned_secret_count_for_test(p.pid),
        MAX_SECRETS_PER_ACTOR - 1
    );
    destroy_owned(p.pid);
    clean(p.pid);
    let source = input(&mut p, 32);
    let _ = split_for_process(&mut p, source, 16).unwrap();
    assert_eq!(owned_secret_count_for_test(p.pid), 2);
    destroy_owned(p.pid);
    clean(p.pid);
}
#[test]
fn split_production_wrapper_foreign_or_wrong_kind_never_consumes_other_resources() {
    let mut p = process(915_006);
    let mut other = process(915_007);
    let source = input(&mut p, 32);
    assert_eq!(
        split_for_process(&mut other, source, 0).unwrap_err().tag,
        CryptoErrorTag::SecretDestroyed
    );
    assert_eq!(bytes(&p, source).len(), 32);
    let key = insert_owned_resource(
        &mut p,
        ResourceKind::AeadKey,
        ProtectedBytes::zeroed(32).unwrap(),
    )
    .unwrap();
    assert_eq!(
        split_for_process(&mut p, key, 0).unwrap_err().tag,
        CryptoErrorTag::InvalidKey
    );
    assert!(with_owned_resource(&p, key, ResourceKind::AeadKey, |_| ()).is_ok());
    assert_eq!(owned_secret_count_for_test(p.pid), 2);
    destroy_owned(p.pid);
    clean(p.pid);
    clean(other.pid);
}
// The following tests exercise only real private publication primitives, not a
// second consume/allocate/copy split algorithm. Allocations occur before calls.
fn table(slots: usize, count: usize) -> ResourceTable {
    ResourceTable::new(Limits::for_tests(slots, count, 65536, 65536, 65536))
}
fn table_input(t: &mut ResourceTable, id: ProcessId, n: usize) -> ResourceHandle {
    t.insert_secret(id, vec![0; n].into_boxed_slice()).unwrap()
}
#[test]
fn split_pair_publication_primitive_preflights_slots_and_actor_count_without_half_entry() {
    for mut t in [table(1, 2), table(2, 1)] {
        let id = ProcessId(915_008);
        let left = ProtectedBytes::zeroed(16).unwrap();
        let right = ProtectedBytes::zeroed(16).unwrap();
        assert_eq!(
            t.insert_secret_pair(id, left, right).unwrap_err(),
            ResourceError::ResourceLimitExceeded
        );
        assert_eq!(t.total_bytes, 0);
        assert!(t
            .usage
            .get(&id)
            .is_none_or(|u| u.secrets == 0 && u.bytes == 0));
    }
}
#[test]
fn borrowed_pair_publication_revalidates_each_generation_under_its_insert_lock() {
    for changed in [0, 1] {
        let id = ProcessId(915_009);
        let mut t = table(4, 4);
        let a = table_input(&mut t, id, 16);
        let b = table_input(&mut t, id, 16);
        let handles = [a, b];
        drop(
            t.consume(id, handles[changed], ResourceKind::SecretBytes)
                .unwrap(),
        );
        let fresh = table_input(&mut t, id, 16);
        let before = t.total_bytes;
        let output = ProtectedBytes::zeroed(32).unwrap();
        let e = t
            .insert_secret_after_borrows(id, &handles, output)
            .unwrap_err();
        assert_eq!(e, ResourceError::StaleHandle);
        assert_eq!(t.total_bytes, before);
        assert!(t
            .validate(id, handles[1 - changed], ResourceKind::SecretBytes)
            .is_ok());
        assert!(t.validate(id, fresh, ResourceKind::SecretBytes).is_ok());
        t.destroy_owned(id);
    }
    let id = ProcessId(915_010);
    let mut t = table(4, 4);
    let a = table_input(&mut t, id, 16);
    let output = ProtectedBytes::zeroed(32).unwrap();
    let out = t.insert_secret_after_borrows(id, &[a, a], output).unwrap();
    assert_eq!(
        t.validate(id, out, ResourceKind::SecretBytes)
            .unwrap()
            .len(),
        32
    );
    t.destroy_owned(id);
}

#[test]
fn secret_crypto_preparation_allocator_runs_outside_actual_table() {
    let mut p = process(916_001);
    let key = input(&mut p, 32);
    let mut calls = 0;
    let prepared = prepare_secret_bytes_for_crypto_with_allocator(&mut p, key, |owner, n| {
        calls += 1;
        assert_table_unlocked();
        assert_eq!(owned_secret_count_for_test(owner.pid), 1);
        protected(owner, n)
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        &*prepared.bytes,
        &(0..32).map(|n| n as u8).collect::<Vec<_>>()
    );
    assert_eq!(bytes(&p, key), prepared.bytes.as_ref());
    drop(prepared);
    destroy_owned(p.pid);
    clean(p.pid);
}
#[test]
fn secret_crypto_preparation_allocation_failure_preserves_borrowed_input() {
    let mut p = process(916_002);
    let key = input(&mut p, 32);
    let e = prepare_secret_bytes_for_crypto_with_allocator(&mut p, key, |_, _| {
        assert_table_unlocked();
        Err(ResourceError::ProtectionUnavailable)
    })
    .err()
    .unwrap();
    assert_eq!(e, ResourceError::ProtectionUnavailable);
    assert_eq!(bytes(&p, key), (0..32).map(|n| n as u8).collect::<Vec<_>>());
    assert_eq!(owned_secret_count_for_test(p.pid), 1);
    destroy_owned(p.pid);
    clean(p.pid);
}
#[test]
fn secret_crypto_preparation_refuses_generation_exit_or_length_change_before_copy() {
    for change in [0, 1, 2] {
        let mut p = process(916_003);
        let key = input(&mut p, 32);
        let e = prepare_secret_bytes_for_crypto_with_allocator(&mut p, key, |owner, n| {
            assert_table_unlocked();
            let output = protected(owner, n)?;
            match change {
                0 => {
                    destroy_resource_for_process(owner, key, Some(ResourceKind::SecretBytes));
                    let _ = input(owner, 32);
                }
                1 => {
                    assert!(owner.mark_exited(ExitReason::Killed));
                }
                _ => {
                    let h = validate_handle_pointer(owner, key).unwrap();
                    let changed = protected(owner, n + 1)?;
                    secret_table().lock().replace_resource_bytes(
                        owner.pid,
                        h,
                        ResourceKind::SecretBytes,
                        changed,
                    )?;
                }
            }
            Ok(output)
        })
        .err()
        .unwrap();
        assert_eq!(
            e,
            if change == 1 {
                ResourceError::OwnerExited
            } else {
                ResourceError::StaleHandle
            }
        );
        if change == 2 {
            assert_eq!(bytes(&p, key).len(), 33);
        }
        destroy_owned(p.pid);
        clean(p.pid);
    }
}
