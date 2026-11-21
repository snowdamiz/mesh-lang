use std::fs;
use std::path::Path;

use mesh_diagnostics::docs;

const REGENERATE: &str = "MESH_BLESS=1 cargo test -p mesh-diagnostics --test artifacts";

#[test]
fn generated_artifacts_match_catalog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bless = std::env::var("MESH_BLESS").as_deref() == Ok("1");

    for (path, expected) in [
        (
            "tools/skill/mesh/references/diagnostics.md",
            docs::skill_page(),
        ),
        ("website/docs/docs/errors/index.md", docs::website_page()),
        ("website/docs/public/diagnostics.json", docs::catalog_json()),
    ] {
        assert!(
            !expected.contains(env!("CARGO_PKG_VERSION")),
            "{path} must stay independent of the crate version"
        );

        let artifact = root.join(path);
        if bless {
            fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            fs::write(&artifact, expected).unwrap_or_else(|error| panic!("write {path}: {error}"));
        } else {
            let actual = fs::read(&artifact)
                .unwrap_or_else(|error| panic!("read {path}: {error}. Run `{REGENERATE}`."));
            assert!(
                actual == expected.as_bytes(),
                "{path} differs from the catalog. Run `{REGENERATE}`."
            );
        }
    }
}
