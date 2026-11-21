use mesh_diagnostics::{closest, Code, Diagnostic};
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

pub(crate) fn show(topic: Option<&str>) -> Result<(), Diagnostic> {
    let path = topic
        .map(|topic| format!("references/{topic}.md"))
        .unwrap_or_else(|| "SKILL.md".to_owned());
    let body = FILES
        .iter()
        .find_map(|(name, body)| (*name == path).then_some(*body))
        .ok_or_else(|| unknown_topic(topic.unwrap_or_default()))?;
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

fn unknown_topic(topic: &str) -> Diagnostic {
    let diagnostic = Diagnostic::new(
        Code::UnknownGuideTopic,
        format!("unknown guide topic '{topic}'"),
    );
    let topic = topic.to_ascii_lowercase();
    let digits = topic.strip_prefix("cfg").or_else(|| {
        topic
            .as_bytes()
            .first()
            .filter(|byte| byte.is_ascii_alphabetic())
            .map(|_| &topic[1..])
    });
    if digits
        .is_some_and(|digits| digits.len() == 4 && digits.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return diagnostic.with_repair(format!(
            "Run `meshc explain {}`.",
            topic.to_ascii_uppercase()
        ));
    }
    let topics = FILES
        .iter()
        .filter_map(|(name, _)| name.strip_prefix("references/")?.strip_suffix(".md"));
    if let Some(suggestion) = closest(&topic, topics, 3) {
        return diagnostic.with_repair(format!(
            "Run `meshc guide {suggestion}`, or run `meshc guide` to list topics."
        ));
    }
    diagnostic
}

#[cfg(test)]
mod tests {
    use mesh_diagnostics::{docs, Code};

    use super::files;

    #[test]
    fn embedded_skill_matches_catalog() {
        let embedded = files()
            .find(|(path, _)| *path == "references/diagnostics.md")
            .expect("embed the diagnostic reference")
            .1;
        assert_eq!(std::str::from_utf8(embedded).unwrap(), docs::skill_page());
    }

    #[test]
    fn every_catalog_topic_is_embedded() {
        for code in Code::all() {
            if let Some(topic) = code.info().topic {
                let reference = format!("references/{topic}.md");
                assert!(
                    files().any(|(path, _)| path == reference),
                    "{} points to a missing guide topic: {topic}",
                    code.id(),
                );
            }
        }
    }
}
