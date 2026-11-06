//! Bounded SPQR map validation/capture directly in protected storage.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpqrMapRole {
    Chains,
    Skipped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpqrStorageSlot {
    pub(crate) epoch: u64,
    pub(crate) send_present: bool,
    pub(crate) send_count: u64,
    pub(crate) receive_present: bool,
    pub(crate) receive_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpqrStorageMetadata {
    pub(crate) direction: u8,
    pub(crate) local_epoch: u64,
    pub(crate) knowledge: u64,
    pub(crate) slots: [SpqrStorageSlot; 3],
    pub(crate) slot_count: u8,
    pub(crate) skipped_index: [(u64, u64); 64],
    pub(crate) skipped_count: u8,
}

pub(crate) fn validate_spqr_storage_metadata(m: &SpqrStorageMetadata) -> Result<(), ResourceError> {
    if m.direction > 1
        || !(1..=3).contains(&m.slot_count)
        || m.skipped_count > 64
        || m.knowledge > m.local_epoch
        || m.local_epoch - m.knowledge > 1
    {
        return Err(ResourceError::WrongKind);
    }
    let start = m.knowledge.saturating_sub(1);
    let count = m
        .local_epoch
        .checked_sub(start)
        .and_then(|n| n.checked_add(1))
        .ok_or(ResourceError::WrongKind)?;
    if count != u64::from(m.slot_count) {
        return Err(ResourceError::WrongKind);
    }
    let slots = &m.slots[..usize::from(m.slot_count)];
    let mut used_sends = 0;
    for (i, slot) in slots.iter().enumerate() {
        if Some(slot.epoch) != start.checked_add(i as u64)
            || !slot.receive_present
            || (!slot.send_present && (slot.epoch >= m.knowledge || slot.send_count != 0))
            || (slot.epoch > m.knowledge && (slot.send_count != 0 || slot.receive_count != 0))
        {
            return Err(ResourceError::WrongKind);
        }
        used_sends += usize::from(slot.send_present && slot.send_count > 0);
    }
    if used_sends > 1 {
        return Err(ResourceError::WrongKind);
    }
    let mut previous = None;
    for &(epoch, number) in &m.skipped_index[..usize::from(m.skipped_count)] {
        if previous.is_some_and(|p| p >= (epoch, number))
            || number == 0
            || !slots
                .iter()
                .any(|s| s.epoch == epoch && s.receive_present && number < s.receive_count)
        {
            return Err(ResourceError::WrongKind);
        }
        previous = Some((epoch, number));
    }
    Ok(())
}

fn expected_shape(
    role: SpqrMapRole,
    m: &SpqrStorageMetadata,
) -> Result<(usize, usize, u16), ResourceError> {
    validate_spqr_storage_metadata(m)?;
    Ok(match role {
        SpqrMapRole::Chains => {
            let directions: usize = m.slots[..usize::from(m.slot_count)]
                .iter()
                .map(|s| usize::from(s.send_present) + usize::from(s.receive_present))
                .sum();
            (1 + directions, 4 + 39 + directions * 48, 7)
        }
        SpqrMapRole::Skipped => {
            let count = usize::from(m.skipped_count);
            (count, 4 + count * 55, 64)
        }
    })
}

fn name_index(name: &[u8], role: SpqrMapRole, m: &SpqrStorageMetadata) -> Option<usize> {
    match role {
        SpqrMapRole::Chains => {
            if name == [0] {
                return Some(0);
            }
            if name.len() != 10 || name[0] != 1 {
                return None;
            }
            let mut index = 1;
            for slot in &m.slots[..usize::from(m.slot_count)] {
                if slot.send_present {
                    if name[1..9] == slot.epoch.to_be_bytes() && name[9] == m.direction {
                        return Some(index);
                    }
                    index += 1;
                }
                if slot.receive_present {
                    if name[1..9] == slot.epoch.to_be_bytes() && name[9] == (m.direction ^ 1) {
                        return Some(index);
                    }
                    index += 1;
                }
            }
            None
        }
        SpqrMapRole::Skipped => {
            if name.len() != 17 || name[0] != 2 {
                return None;
            }
            m.skipped_index[..usize::from(m.skipped_count)]
                .iter()
                .position(|&(e, n)| name[1..9] == e.to_be_bytes() && name[9..17] == n.to_be_bytes())
        }
    }
}

pub(crate) fn validate_spqr_storage_map(
    bytes: &[u8],
    role: SpqrMapRole,
    m: &SpqrStorageMetadata,
) -> Result<(), ResourceError> {
    let (count, length, capacity) = expected_shape(role, m)?;
    if bytes.len() != length
        || bytes[..2] != capacity.to_be_bytes()
        || bytes[2..4] != (count as u16).to_be_bytes()
    {
        return Err(ResourceError::WrongKind);
    }
    let mut offset = 4usize;
    let mut seen = 0u64;
    for _ in 0..count {
        let header_end = offset.checked_add(6).ok_or(ResourceError::WrongKind)?;
        let header = bytes
            .get(offset..header_end)
            .ok_or(ResourceError::WrongKind)?;
        let names = usize::from(u16::from_be_bytes([header[0], header[1]]));
        if u32::from_be_bytes(header[2..6].try_into().expect("bounded header")) != 32 {
            return Err(ResourceError::WrongKind);
        }
        let name_end = header_end
            .checked_add(names)
            .ok_or(ResourceError::WrongKind)?;
        let name = bytes
            .get(header_end..name_end)
            .ok_or(ResourceError::WrongKind)?;
        let bit = 1u64
            .checked_shl(name_index(name, role, m).ok_or(ResourceError::WrongKind)? as u32)
            .ok_or(ResourceError::WrongKind)?;
        if seen & bit != 0 {
            return Err(ResourceError::WrongKind);
        }
        seen |= bit;
        offset = name_end
            .checked_add(32)
            .filter(|&n| n <= bytes.len())
            .ok_or(ResourceError::WrongKind)?;
    }
    let expected = if count == 64 {
        u64::MAX
    } else {
        (1u64 << count) - 1
    };
    if offset != bytes.len() || seen != expected {
        return Err(ResourceError::WrongKind);
    }
    Ok(())
}

pub(crate) fn prepare_spqr_storage_map(
    process: &mut Process,
    pointer: *const MeshSecretHandle,
    role: SpqrMapRole,
    metadata: &SpqrStorageMetadata,
) -> Result<PreparedOwnedResource, ResourceError> {
    prepare_with_allocator(process, pointer, role, metadata, |_, n| {
        ProtectedBytes::zeroed(n).map_err(Into::into)
    })
}

fn prepare_with_allocator(
    process: &mut Process,
    pointer: *const MeshSecretHandle,
    role: SpqrMapRole,
    metadata: &SpqrStorageMetadata,
    mut allocate: impl FnMut(&mut Process, usize) -> Result<ProtectedBytes, ResourceError>,
) -> Result<PreparedOwnedResource, ResourceError> {
    let (_, expected, _) = expected_shape(role, metadata)?;
    let owner = live_owner(process)?;
    let handle = validate_handle_pointer(process, pointer).ok_or(ResourceError::StaleHandle)?;
    let length = secret_table()
        .lock()
        .validate(owner, handle, ResourceKind::SecretMap)?
        .len();
    if length != expected {
        return Err(ResourceError::WrongKind);
    }
    // Bounded protected allocation happens without the global table lock.
    let mut bytes = allocate(process, length)?;
    if bytes.len() != length {
        return Err(ResourceError::WrongKind);
    }
    {
        let table = secret_table().lock();
        if live_owner(process)? != owner {
            return Err(ResourceError::WrongOwner);
        }
        let source = table.validate(owner, handle, ResourceKind::SecretMap)?;
        if source.len() != length {
            return Err(ResourceError::StaleHandle);
        }
        bytes.copy_from_slice(source);
    }
    validate_spqr_storage_map(&bytes, role, metadata)?;
    Ok(PreparedOwnedResource { handle, bytes })
}

pub(crate) fn validate_prepared_spqr_storage_map(
    process: &Process,
    prepared: &PreparedOwnedResource,
) -> Result<(), ResourceError> {
    let owner = live_owner(process)?;
    let table = secret_table().lock();
    let source = table.validate(owner, prepared.handle, ResourceKind::SecretMap)?;
    if source.len() != prepared.bytes.len()
        || subtle::ConstantTimeEq::ct_eq(source, &prepared.bytes[..]).unwrap_u8() != 1
    {
        return Err(ResourceError::StaleHandle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::{ExitReason, Priority};
    use crate::storage_wrapping::spqr;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    fn metadata() -> SpqrStorageMetadata {
        let mut slots = [SpqrStorageSlot::default(); 3];
        slots[0].send_present = true;
        slots[0].receive_present = true;
        SpqrStorageMetadata {
            direction: 0,
            local_epoch: 0,
            knowledge: 0,
            slots,
            slot_count: 1,
            skipped_index: [(0, 0); 64],
            skipped_count: 0,
        }
    }
    fn map() -> Vec<u8> {
        let mut out = vec![0, 7, 0, 3];
        for (i, name) in [
            vec![0],
            vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        ]
        .iter()
        .enumerate()
        {
            out.extend_from_slice(&(name.len() as u16).to_be_bytes());
            out.extend_from_slice(&32u32.to_be_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&[i as u8 + 1; 32]);
        }
        out
    }
    fn context() -> Vec<u8> {
        let mut out = vec![1];
        for (byte, count) in [(0x11, 32), (0x22, 16), (0x33, 32), (0x44, 32)] {
            out.extend(vec![byte; count]);
        }
        out.extend(24u16.to_be_bytes());
        out.extend(9u64.to_be_bytes());
        out.extend([1, 0]);
        out.extend(0u64.to_be_bytes());
        out.extend(0u64.to_be_bytes());
        out.extend([0x55; 32]);
        out.extend([0x66; 32]);
        out.push(1);
        out.extend(0u64.to_be_bytes());
        out.push(1);
        out.extend(0u64.to_be_bytes());
        out.push(1);
        out.extend(0u64.to_be_bytes());
        out.push(0);
        assert_eq!(out.len(), 233);
        out
    }
    fn setup(id: u64) -> Process {
        crate::mesh_rt_init();
        Process::new(ProcessId(id), Priority::Normal)
    }
    fn source(p: &mut Process) -> *mut MeshSecretHandle {
        insert_owned_resource(
            p,
            ResourceKind::SecretMap,
            ProtectedBytes::copy_from(&map()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn spqr_protected_allocator_is_unlocked_and_borrow_survives() {
        let mut p = setup(915_241);
        let h = source(&mut p);
        let mut called = false;
        let prepared =
            prepare_with_allocator(&mut p, h, SpqrMapRole::Chains, &metadata(), |_, n| {
                called = true;
                assert_eq!(n, 139);
                let lock = secret_table().try_lock_for(Duration::from_secs(2));
                assert!(
                    lock.is_some(),
                    "protected allocator must not hold global table"
                );
                drop(lock);
                ProtectedBytes::zeroed(n).map_err(Into::into)
            })
            .expect("valid bounded role must prepare");
        assert!(called);
        assert_eq!(&*prepared.bytes, map());
        validate_prepared_spqr_storage_map(&p, &prepared).unwrap();
        assert_eq!(owned_secret_count_for_test(p.pid), 1);
        destroy_owned(p.pid);
    }

    #[test]
    fn spqr_allocation_failure_keeps_original_borrow_and_exact_error() {
        let mut p = setup(915_242);
        let h = source(&mut p);
        let error = prepare_with_allocator(&mut p, h, SpqrMapRole::Chains, &metadata(), |_, _| {
            Err(ResourceError::ProtectionUnavailable)
        })
        .err();
        assert_eq!(error, Some(ResourceError::ProtectionUnavailable));
        assert_eq!(owned_secret_count_for_test(p.pid), 1);
        assert!(prepare_spqr_storage_map(&mut p, h, SpqrMapRole::Chains, &metadata()).is_ok());
        destroy_owned(p.pid);
    }

    #[test]
    fn spqr_allocator_drift_rechecks_owner_generation_length_and_kind_before_copy() {
        for mode in 0..4 {
            let mut p = setup(915_243);
            let h = source(&mut p);
            let mut called = false;
            let error =
                prepare_with_allocator(&mut p, h, SpqrMapRole::Chains, &metadata(), |owner, n| {
                    called = true;
                    let allocation = ProtectedBytes::zeroed(n).map_err(ResourceError::from)?;
                    if mode == 0 {
                        owner.mark_exited(ExitReason::Killed);
                    } else {
                        let handle = validate_handle_pointer(owner, h).unwrap();
                        let mut table = secret_table().lock();
                        match mode {
                            1 => {
                                drop(table.consume(owner.pid, handle, ResourceKind::SecretMap)?);
                            }
                            2 => table.replace_resource_bytes(
                                owner.pid,
                                handle,
                                ResourceKind::SecretMap,
                                ProtectedBytes::zeroed(n + 1).unwrap(),
                            )?,
                            _ => {
                                table.slots[handle.slot as usize]
                                    .entry
                                    .as_mut()
                                    .unwrap()
                                    .kind = ResourceKind::SecretBytes
                            }
                        }
                    }
                    Ok(allocation)
                })
                .err();
            assert!(
                called,
                "valid source must reach unlocked allocation before drift"
            );
            assert_eq!(
                error,
                Some(match mode {
                    0 => ResourceError::OwnerExited,
                    3 => ResourceError::WrongKind,
                    _ => ResourceError::StaleHandle,
                })
            );
            destroy_owned(p.pid);
        }
    }

    #[test]
    fn spqr_wrong_lengths_and_kinds_never_invoke_protected_allocator() {
        for (kind, n) in [
            (ResourceKind::SecretMap, 140),
            (ResourceKind::SecretMap, 65536),
            (ResourceKind::SecretBytes, 139),
        ] {
            let mut p = setup(915_244);
            let h =
                insert_owned_resource(&mut p, kind, ProtectedBytes::zeroed(n).unwrap()).unwrap();
            let mut called = false;
            assert!(
                prepare_with_allocator(&mut p, h, SpqrMapRole::Chains, &metadata(), |_, _| {
                    called = true;
                    panic!("unbounded/wrong-kind allocation")
                })
                .is_err()
            );
            assert!(!called);
            destroy_owned(p.pid);
        }
    }

    #[test]
    fn spqr_same_handle_equal_length_content_drift_invalidates_capture() {
        for mode in 0..2 {
            let mut p = setup(915_245);
            let h = source(&mut p);
            let prepared = prepare_spqr_storage_map(&mut p, h, SpqrMapRole::Chains, &metadata())
                .expect("capture valid map");
            let handle = validate_handle_pointer(&p, h).unwrap();
            let mut changed = map();
            if mode == 0 {
                *changed.last_mut().unwrap() ^= 1;
            } else {
                changed[57] ^= 1;
            }
            secret_table()
                .lock()
                .replace_resource_bytes(
                    p.pid,
                    handle,
                    ResourceKind::SecretMap,
                    ProtectedBytes::copy_from(&changed).unwrap(),
                )
                .unwrap();
            assert_eq!(
                validate_prepared_spqr_storage_map(&p, &prepared).err(),
                Some(ResourceError::StaleHandle)
            );
            destroy_owned(p.pid);
        }
    }

    struct Counter {
        process: Arc<Mutex<Process>>,
        source: ResourceHandle,
        key: Option<ResourceHandle>,
        next: AtomicU64,
        calls: AtomicUsize,
        locked: AtomicUsize,
        mode: usize,
        source_pointer: usize,
        key_pointer: usize,
        context_source: usize,
    }
    unsafe extern "C" fn reserve(ctx: *mut c_void, out: *mut u64) -> i32 {
        let ctx = unsafe { &*(ctx.cast::<Counter>()) };
        ctx.calls.fetch_add(1, Ordering::SeqCst);
        let n = ctx.next.fetch_add(1, Ordering::SeqCst) + 1;
        unsafe { out.write(n) };
        let Some(mut p) = ctx.process.try_lock_for(Duration::from_secs(2)) else {
            ctx.locked.fetch_add(1, Ordering::SeqCst);
            return 1;
        };
        let Some(mut table) = secret_table().try_lock_for(Duration::from_secs(2)) else {
            ctx.locked.fetch_add(1, Ordering::SeqCst);
            return 1;
        };
        if ctx.mode == 6 {
            drop(table);
            p.mark_exited(ExitReason::Killed);
            return 0;
        }
        let result = match ctx.mode {
            0 => Ok(()),
            4 => table
                .consume(p.pid, ctx.source, ResourceKind::SecretMap)
                .and_then(|old| {
                    drop(old);
                    table.insert(
                        p.pid,
                        ResourceKind::SecretMap,
                        ProtectedBytes::copy_from(&map()).unwrap(),
                    )
                })
                .map(|replacement| unsafe {
                    (ctx.source_pointer as *mut MeshSecretHandle)
                        .write(MeshSecretHandle::from(replacement))
                }),
            5 => table
                .consume(p.pid, ctx.key.unwrap(), ResourceKind::StorageKey)
                .map(drop),
            7 => table
                .consume(p.pid, ctx.key.unwrap(), ResourceKind::StorageKey)
                .and_then(|old| {
                    drop(old);
                    table.insert_storage_key(
                        p.pid,
                        ProtectedBytes::copy_from(&[0x31; 36]).unwrap(),
                        StorageCounterSource::Production {
                            reserve,
                            context: ctx as *const Counter as usize,
                            last_counter: None,
                        },
                    )
                })
                .map(|replacement| unsafe {
                    (ctx.key_pointer as *mut MeshSecretHandle)
                        .write(MeshSecretHandle::from(replacement))
                }),
            8 => {
                if ctx.context_source == 0 {
                    Err(ResourceError::WrongKind)
                } else {
                    unsafe {
                        (ctx.context_source as *mut u8)
                            .add(std::mem::size_of::<crate::bytes::MeshBytes>() + 81)
                            .write(0xff)
                    };
                    Ok(())
                }
            }
            mode => {
                let mut changed = map();
                if mode == 1 {
                    *changed.last_mut().unwrap() ^= 1;
                } else if mode == 2 {
                    changed[57] ^= 1;
                } else {
                    let mut reordered = changed[..4].to_vec();
                    reordered.extend_from_slice(&changed[43..]);
                    reordered.extend_from_slice(&changed[4..43]);
                    changed = reordered;
                }
                table.replace_resource_bytes(
                    p.pid,
                    ctx.source,
                    ResourceKind::SecretMap,
                    ProtectedBytes::copy_from(&changed).unwrap(),
                )
            }
        };
        if result.is_ok() {
            0
        } else {
            2
        }
    }
    fn fixture(
        id: u64,
        mode: usize,
    ) -> (
        Arc<Mutex<Process>>,
        *mut MeshSecretHandle,
        *mut MeshSecretHandle,
        Box<Counter>,
    ) {
        let p = Arc::new(Mutex::new(setup(id)));
        let h = source(&mut p.lock());
        let handle = validate_handle_pointer(&p.lock(), h).unwrap();
        let mut counter = Box::new(Counter {
            process: p.clone(),
            source: handle,
            key: None,
            next: AtomicU64::new(40),
            calls: AtomicUsize::new(0),
            locked: AtomicUsize::new(0),
            mode,
            source_pointer: h as usize,
            key_pointer: 0,
            context_source: 0,
        });
        let key = insert_storage_key_resource(
            &mut p.lock(),
            ProtectedBytes::copy_from(&[0x31; 36]).unwrap(),
            reserve,
            (&mut *counter as *mut Counter).cast(),
        )
        .unwrap();
        counter.key = Some(validate_handle_pointer(&p.lock(), key).unwrap());
        counter.key_pointer = key as usize;
        (p, h, key, counter)
    }

    #[test]
    fn spqr_real_unlocked_reservation_drift_burns_counter_before_any_aead() {
        for mode in 1..=7 {
            let (p, h, key, counter) = fixture(915_246, mode);
            let calls = Arc::new(AtomicUsize::new(0));
            let observe = calls.clone();
            let result = spqr::with_test_hooks(
                move || {
                    observe.fetch_add(1, Ordering::SeqCst);
                },
                || {},
                || spqr::seal_for_owner(p.clone(), h, key, &context(), ResourceKind::SecretMap),
            );
            assert_eq!(
                counter.calls.load(Ordering::SeqCst),
                1,
                "valid role must reserve through actual host callback"
            );
            assert_eq!(counter.next.load(Ordering::SeqCst), 41);
            assert_eq!(counter.locked.load(Ordering::SeqCst), 0);
            assert!(
                result.is_err(),
                "changed owner/source/key must publish no ciphertext"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "post-reservation drift must fail before protected AEAD"
            );
            if mode == 4 {
                assert_ne!(validate_handle_pointer(&p.lock(), h), Some(counter.source));
                assert!(prepare_spqr_storage_map(&mut p.lock(),h,SpqrMapRole::Chains,&metadata()).is_ok(),"rebound caller pointer now names valid replacement, never the captured generation");
            }
            if mode == 7 {
                assert_ne!(validate_handle_pointer(&p.lock(), key), counter.key);
                assert!(
                    prepare_storage_key_resource(&p.lock(), key).is_ok(),
                    "rebound key pointer cannot authorize pending original key"
                );
            }
            destroy_owned(p.lock().pid);
        }
    }

    #[test]
    fn spqr_reservation_failure_retry_uses_strictly_newer_nonce() {
        let (p, h, key, mut counter) = fixture(915_247, 1);
        assert!(
            spqr::seal_for_owner(p.clone(), h, key, &context(), ResourceKind::SecretMap).is_err()
        );
        assert_eq!(counter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(counter.next.load(Ordering::SeqCst), 41);
        counter.mode = 0;
        let pid = p.lock().pid;
        secret_table()
            .lock()
            .replace_resource_bytes(
                pid,
                counter.source,
                ResourceKind::SecretMap,
                ProtectedBytes::copy_from(&map()).unwrap(),
            )
            .unwrap();
        let blob = spqr::seal_for_owner(p.clone(), h, key, &context(), ResourceKind::SecretMap)
            .expect("retry after restoring original fixture");
        assert_eq!(u64::from_be_bytes(blob[7..15].try_into().unwrap()), 42);
        assert_eq!(counter.calls.load(Ordering::SeqCst), 2);
        destroy_owned(pid);
    }

    #[test]
    fn spqr_after_aead_content_drift_refuses_publication() {
        let (p, h, key, counter) = fixture(915_248, 0);
        let entered = Arc::new(AtomicUsize::new(0));
        let before = entered.clone();
        let mutated = Arc::new(AtomicUsize::new(0));
        let after = mutated.clone();
        let owner = p.clone();
        let result = spqr::with_test_hooks(
            move || {
                before.fetch_add(1, Ordering::SeqCst);
            },
            move || {
                let pid = owner.lock().pid;
                let mut changed = map();
                *changed.last_mut().unwrap() ^= 1;
                secret_table()
                    .lock()
                    .replace_resource_bytes(
                        pid,
                        counter.source,
                        ResourceKind::SecretMap,
                        ProtectedBytes::copy_from(&changed).unwrap(),
                    )
                    .unwrap();
                after.fetch_add(1, Ordering::SeqCst);
            },
            || spqr::seal_for_owner(p.clone(), h, key, &context(), ResourceKind::SecretMap),
        );
        assert_eq!(entered.load(Ordering::SeqCst), 1);
        assert_eq!(mutated.load(Ordering::SeqCst), 1);
        if let Ok(blob) = &result {
            assert!(spqr::open(
                &crate::crypto::provider::SystemProvider,
                blob,
                &[0x31; 36],
                &context(),
                ResourceKind::SecretMap
            )
            .is_ok());
            println!("SPQR_FINAL_GUARD_MUTANT_EXPOSED_AUTHENTIC_BLOB");
        }
        assert!(
            result.is_err(),
            "final content guard must refuse otherwise authentic ciphertext"
        );
        destroy_owned(p.lock().pid);
    }
    #[test]
    fn spqr_original_public_context_mutation_cannot_redirect_captured_binding() {
        let (p, h, key, mut counter) = fixture(915_251, 8);
        let original = context();
        let mut native = vec![0u64; 1 + original.len().div_ceil(8)];
        native[0] = original.len() as u64;
        unsafe {
            std::slice::from_raw_parts_mut(native.as_mut_ptr().cast::<u8>().add(8), original.len())
                .copy_from_slice(&original)
        };
        counter.context_source = native.as_mut_ptr() as usize;
        let captured = unsafe { spqr::copy_resource_context(native.as_ptr().cast()) }.unwrap();
        let blob =
            spqr::seal_for_owner(p.clone(), h, key, &captured, ResourceKind::SecretMap).unwrap();
        assert_eq!(counter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(counter.locked.load(Ordering::SeqCst), 0);
        let changed = unsafe { spqr::copy_resource_context(native.as_ptr().cast()) }.unwrap();
        assert_ne!(changed, original);
        assert!(spqr::open(
            &crate::crypto::provider::SystemProvider,
            &blob,
            &[0x31; 36],
            &original,
            ResourceKind::SecretMap
        )
        .is_ok());
        assert_eq!(
            spqr::open(
                &crate::crypto::provider::SystemProvider,
                &blob,
                &[0x31; 36],
                &changed,
                ResourceKind::SecretMap
            )
            .err()
            .unwrap()
            .tag,
            CryptoErrorTag::AuthenticationFailed
        );
        destroy_owned(p.lock().pid);
    }

    #[test]
    fn spqr_after_aead_wrapping_key_or_owner_drift_refuses_publication() {
        for mode in 0..2 {
            let (p, h, key, counter) = fixture(915_252, 0);
            let owner = p.clone();
            let key_handle = counter.key.unwrap();
            let hit = Arc::new(AtomicUsize::new(0));
            let seen = hit.clone();
            let result = spqr::with_test_hooks(
                || {},
                move || {
                    let mut p = owner.lock();
                    if mode == 0 {
                        drop(
                            secret_table()
                                .lock()
                                .consume(p.pid, key_handle, ResourceKind::StorageKey)
                                .unwrap(),
                        );
                    } else {
                        p.mark_exited(ExitReason::Killed);
                    }
                    seen.fetch_add(1, Ordering::SeqCst);
                },
                || spqr::seal_for_owner(p.clone(), h, key, &context(), ResourceKind::SecretMap),
            );
            assert_eq!(counter.calls.load(Ordering::SeqCst), 1);
            assert_eq!(hit.load(Ordering::SeqCst), 1);
            assert!(
                result.is_err(),
                "post-AEAD key/owner identity must remain live"
            );
            destroy_owned(p.lock().pid);
        }
    }
}
