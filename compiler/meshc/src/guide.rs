use sha2::{Digest, Sha256};

include!(concat!(env!("OUT_DIR"), "/guide_files.rs"));

pub(crate) fn files() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    FILES
        .iter()
        .map(|(name, content)| (*name, content.as_bytes()))
}

pub(crate) fn digest<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut hash = Sha256::new();
    for (name, bytes) in files {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

pub(crate) fn show(topic: Option<&str>) -> Result<(), String> {
    let path = topic
        .map(|topic| format!("references/{topic}.md"))
        .unwrap_or_else(|| "SKILL.md".to_owned());
    let body = FILES
        .iter()
        .find_map(|(name, body)| (*name == path).then_some(*body))
        .ok_or_else(|| {
            format!(
                "unknown guide topic '{}'. Run `meshc guide` for the topic list.",
                topic.unwrap_or_default()
            )
        })?;
    println!(
        "Mesh {} guide (SHA-256 {})\n",
        env!("CARGO_PKG_VERSION"),
        digest(files())
    );
    // The index's metadata is for agent hosts. The CLI prints the Markdown body.
    print!(
        "{}",
        body.strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map_or(body, |(_, body)| body.trim_start_matches('\n'))
    );
    Ok(())
}
