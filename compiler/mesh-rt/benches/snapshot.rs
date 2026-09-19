//! Run with `cargo bench -p mesh-rt --bench snapshot`.
//! SQLite setup and payload verification are outside the timed region.

use mesh_rt::dist::continuity_store::{
    ContinuityStore, ContinuityStoreLimits, SnapshotChunk, SqliteContinuityStore,
    StoredContinuityPhase, StoredContinuityRecord,
};
use sha2::{Digest, Sha256};
use std::{hint::black_box, path::Path, time::Instant};

fn records(count: usize) -> Vec<StoredContinuityRecord> {
    (0..count)
        .map(|index| StoredContinuityRecord {
            operation_key: format!("snapshot-{index:05}"),
            request_hash: format!("hash-{index:05}"),
            request_body: b"{\"kind\":\"snapshot\"}".to_vec(),
            runtime_record: Vec::new(),
            owner_node: "owner".to_string(),
            ownership_generation: 1,
            attempts: vec![format!("attempt-{index}")],
            phase: StoredContinuityPhase::Completed,
            replica_set: vec!["replica".to_string()],
            created_at_millis: 1,
            updated_at_millis: 2,
            terminal_at_millis: Some(2),
            expires_at_millis: Some(4_102_444_800_000),
            response_metadata: vec![("status".to_string(), "200".to_string())],
            response_body: vec![(index % 251) as u8; 256],
            control_term: 1,
            schema_version: 1,
            version: 1,
        })
        .collect()
}

fn verify(chunks: &[SnapshotChunk], records: &[StoredContinuityRecord], bound: usize) {
    let mut decoded = Vec::new();
    let mut hasher = Sha256::new();
    for (sequence, chunk) in chunks.iter().enumerate() {
        assert!(chunk.verify());
        assert!(chunk.payload.len() <= bound);
        assert_eq!(chunk.sequence as usize, sequence);
        assert_eq!(chunk.final_chunk, sequence + 1 == chunks.len());
        assert_eq!(chunk.high_water_mark as usize, records.len());
        assert_eq!(
            chunk.snapshot_id,
            format!("snapshot-{}-{}", records.len(), records.len())
        );
        hasher.update(chunk.checksum);
        decoded
            .extend(serde_json::from_slice::<Vec<StoredContinuityRecord>>(&chunk.payload).unwrap());
    }
    let checksum: [u8; 32] = hasher.finalize().into();
    assert!(chunks
        .iter()
        .all(|chunk| chunk.snapshot_checksum == checksum));
    assert_eq!(decoded, records);
}

fn main() {
    println!("case,records,chunk_bytes,chunks,payload_bytes,median_ns,min_ns,max_ns");
    for count in [100, 1000] {
        let source =
            SqliteContinuityStore::open(Path::new(":memory:"), ContinuityStoreLimits::default())
                .unwrap();
        let records = records(count);
        for record in &records {
            source.upsert(record).unwrap();
        }
        for bound in [4096, 1024 * 1024] {
            let expected = source.snapshot_chunks(bound).unwrap();
            verify(&expected, &records, bound);
            let mut samples = [0; 7];
            for sample in &mut samples {
                let start = Instant::now();
                let chunks = black_box(source.snapshot_chunks(black_box(bound)).unwrap());
                *sample = start.elapsed().as_nanos();
                assert_eq!(chunks, expected);
            }
            samples.sort_unstable();
            let bytes: usize = expected.iter().map(|chunk| chunk.payload.len()).sum();
            println!(
                "snapshot,{count},{bound},{},{bytes},{},{},{}",
                expected.len(),
                samples[3],
                samples[0],
                samples[6]
            );
        }
    }
}
