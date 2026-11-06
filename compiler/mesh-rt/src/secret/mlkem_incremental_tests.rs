//! Runtime-only two-output ownership and quota tests for C1.
use super::*;
use crate::crypto::mlkem_incremental::{begin, finish, parts};
use crate::crypto::provider::FixedProvider;
fn first() -> crate::crypto::mlkem_incremental::FirstEncapsulation {
    let (header, _) = parts(&[0x69; 64]);
    begin(&FixedProvider::with_random(&[0x73; 32]), &header).unwrap()
}
#[test]
fn c1_pair_refuses_every_partial_quota_without_retaining_a_half_output() {
    let owner = ProcessId(191_131);
    for limits in [
        Limits::for_tests(1, 2, 132, 100, 132),
        Limits::for_tests(2, 1, 132, 100, 132),
        Limits::for_tests(2, 2, 131, 100, 132),
        Limits::for_tests(2, 2, 132, 99, 132),
        Limits::for_tests(2, 2, 132, 100, 131),
    ] {
        let mut table = ResourceTable::new(limits);
        let first = first();
        assert_eq!(
            table
                .insert_mlkem_encapsulation(owner, first.state, first.shared)
                .err()
                .unwrap(),
            ResourceError::ResourceLimitExceeded
        );
        assert_eq!(table.total_bytes, 0, "failed pair retained private bytes");
        assert!(
            !table.usage.contains_key(&owner),
            "failed pair retained actor quota"
        );
        assert!(
            table.slots.iter().all(|slot| slot.entry.is_none()),
            "failed pair retained hidden output"
        );
    }
}
#[test]
fn c1_pair_rejects_wrong_shared_width_before_any_publication() {
    let mut table = ResourceTable::new(Limits::for_tests(2, 2, 132, 100, 132));
    let owner = ProcessId(191_132);
    for length in [0, 31, 33] {
        let first = first();
        assert_eq!(
            table
                .insert_mlkem_encapsulation(
                    owner,
                    first.state,
                    ProtectedBytes::zeroed(length).unwrap()
                )
                .err()
                .unwrap(),
            ResourceError::WrongKind
        );
        assert_eq!(table.total_bytes, 0);
        assert!(!table.usage.contains_key(&owner));
    }
}
#[test]
fn c1_pair_exact_quota_success_preserves_owner_kind_retry_and_destroy() {
    let owner = ProcessId(191_133);
    let mut table = ResourceTable::new(Limits::for_tests(2, 2, 132, 100, 132));
    let first = first();
    let (_, vector) = parts(&[0x69; 64]);
    let expected = finish(&first.state, &vector).unwrap();
    let (state, shared) = table
        .insert_mlkem_encapsulation(owner, first.state, first.shared)
        .unwrap();
    assert_eq!(table.total_bytes, 132);
    assert_eq!(table.usage[&owner].secrets, 2);
    assert_eq!(table.usage[&owner].bytes, 132);
    assert_eq!(
        ResourceKind::from_raw(12),
        Some(ResourceKind::MlKemEncapsulationState)
    );
    let copy = table
        .with_resource(
            owner,
            state,
            ResourceKind::MlKemEncapsulationState,
            ProtectedBytes::copy_from,
        )
        .unwrap()
        .unwrap();
    assert!(matches!(
        table.validate(owner, state, ResourceKind::SecretBytes),
        Err(ResourceError::WrongKind)
    ));
    assert!(matches!(
        table.validate(
            ProcessId(191_134),
            state,
            ResourceKind::MlKemEncapsulationState
        ),
        Err(ResourceError::WrongOwner)
    ));
    assert_eq!(
        table
            .validate(owner, shared, ResourceKind::SecretBytes)
            .unwrap()
            .len(),
        32
    );
    assert_eq!(table.destroy_owned(owner), 2);
    assert_eq!(table.total_bytes, 0);
    assert!(table
        .validate(owner, state, ResourceKind::MlKemEncapsulationState)
        .is_err());
    // Explicitly prepared copies remain protected and independently owned.
    let prepared =
        crate::crypto::mlkem_incremental::EncapsulationState::from_protected(copy).unwrap();
    assert_eq!(finish(&prepared, &vector).unwrap(), expected);
}
#[test]
fn c1_actor_pair_handles_are_typed_affine_and_owner_bound() {
    use crate::actor::{Priority, Process};
    let owner = ProcessId(191_135);
    let mut process = Process::new(owner, Priority::Normal);
    let first = first();
    let (state, shared) =
        insert_owned_mlkem_encapsulation(&mut process, first.state, first.shared).unwrap();
    assert_eq!(owned_secret_count_for_test(owner), 2);
    assert_eq!(
        with_owned_resource(
            &process,
            state,
            ResourceKind::MlKemEncapsulationState,
            |b| b.len()
        )
        .unwrap(),
        100
    );
    assert_eq!(
        with_owned_resource(&process, shared, ResourceKind::SecretBytes, |b| b.len()).unwrap(),
        32
    );
    assert!(with_owned_resource(&process, state, ResourceKind::SecretBytes, |_| ()).is_err());
    let other = Process::new(ProcessId(191_136), Priority::Normal);
    assert!(
        with_owned_resource(&other, state, ResourceKind::MlKemEncapsulationState, |_| ()).is_err()
    );
    destroy_resource_for_process(&process, state, Some(ResourceKind::MlKemEncapsulationState));
    destroy_resource_for_process(&process, state, Some(ResourceKind::MlKemEncapsulationState));
    assert_eq!(owned_secret_count_for_test(owner), 1);
    assert!(with_owned_resource(
        &process,
        state,
        ResourceKind::MlKemEncapsulationState,
        |_| ()
    )
    .is_err());
    destroy_owned(owner);
    assert_eq!(owned_secret_count_for_test(owner), 0);
}
