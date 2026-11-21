//! The catalog's invariants: well-formed IDs and names, severities that fit
//! their family, lookups that round-trip, and examples for every code a
//! program's source can cause.

use std::collections::BTreeSet;
use std::path::Path;

use mesh_diagnostics::{Code, Family, Lookup, Severity};

const RETIRED: &[&str] = &["C0001", "E0013", "E0024", "E0025", "E0043"];

// Law file syntax, law size limits, logic size limits, and test file syntax.
const NOT_IN_A_PROGRAM: &[&str] = &[
    "P0201", "P0202", "P0302", "P0401",
    // Project loading always supplies the defining module. The type checker
    // tests the missing-origin case with a synthetic import context.
    "E0051",
];

fn is_id(id: &str) -> bool {
    let digits = match id.strip_prefix("CFG") {
        Some(digits) => digits,
        None => match id.as_bytes().first() {
            Some(letter) if letter.is_ascii_uppercase() => &id[1..],
            _ => return false,
        },
    };
    digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit())
}

fn is_kebab(name: &str) -> bool {
    name.split('-').all(|word| {
        word.starts_with(|c: char| c.is_ascii_lowercase())
            && word
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

fn is_source_code(code: Code) -> bool {
    matches!(
        code.family(),
        Family::Syntax | Family::Semantic | Family::Lint
    )
}

fn needs_example(code: Code) -> bool {
    is_source_code(code) && !NOT_IN_A_PROGRAM.contains(&code.id())
}

#[test]
fn ids_are_well_formed_and_sorted() {
    let ids: Vec<&str> = Code::all().map(Code::id).collect();
    for id in &ids {
        assert!(
            is_id(id),
            "`{id}` is not a letter and four digits, or CFG and four digits"
        );
    }
    for id in RETIRED {
        assert!(is_id(id), "retired `{id}`");
        assert!(!ids.contains(id), "retired `{id}` is live again");
    }
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "catalog rows are in ID order");
}

#[test]
fn severities_fit_their_family() {
    for code in Code::all() {
        let id = code.id();
        let expected = match id.as_bytes()[0] {
            b'W' | b'L' => Some(Severity::Warning),
            b'I' => None,
            _ => Some(Severity::Error),
        };
        if let Some(expected) = expected {
            assert_eq!(code.severity(), expected, "{id}");
        }
    }
    assert_eq!(Code::ClusterDeclarationInvalid.family(), Family::Project);
    assert_eq!(Code::UnexpectedToken.family(), Family::Syntax);
    assert_eq!(Code::RedundantArm.family(), Family::Semantic);
    assert_eq!(Code::InternalCompilerError.family(), Family::Internal);
}

#[test]
fn names_are_unique_kebab_case_and_never_ids() {
    let mut seen = BTreeSet::new();
    for code in Code::all() {
        let name = code.info().name;
        assert!(is_kebab(name), "{}: `{name}` is not kebab-case", code.id());
        assert!(seen.insert(name), "`{name}` names two codes");
        assert!(
            !is_id(&name.to_ascii_uppercase()),
            "{}: name `{name}` reads as an ID",
            code.id()
        );
    }
}

#[test]
fn every_row_has_prose_and_a_known_topic() {
    let references =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/skill/mesh/references");
    for code in Code::all() {
        let info = code.info();
        assert!(
            info.title.chars().next().is_some_and(|c| !c.is_uppercase()),
            "{}: title starts lowercase",
            info.id
        );
        for (field, text) in [("summary", info.summary), ("repair", info.repair)] {
            assert!(text.ends_with('.'), "{}: {field} is a sentence", info.id);
            assert!(
                !text.contains('\n'),
                "{}: {field} is one paragraph",
                info.id
            );
        }
        if is_source_code(code) {
            assert!(
                info.topic.is_some(),
                "{}: P/E/W/L codes name a guide topic",
                info.id
            );
        }
        if let Some(topic) = info.topic {
            let page = references.join(format!("{topic}.md"));
            assert!(
                page.is_file(),
                "{}: topic `{topic}` is not a skill reference",
                info.id
            );
        }
    }
}

#[test]
fn source_codes_have_examples() {
    for code in Code::all().filter(|&code| needs_example(code)) {
        let info = code.info();
        let example = info
            .example
            .unwrap_or_else(|| panic!("{} has no example", info.id));
        assert_ne!(example.bad, example.fixed, "{}", info.id);
    }
    for id in NOT_IN_A_PROGRAM {
        let Lookup::Found(code) = Code::lookup(id) else {
            panic!("NOT_IN_A_PROGRAM lists `{id}`, which is no code");
        };
        assert!(is_source_code(code), "{id}");
        assert!(code.info().example.is_none(), "{id} has an example");
    }
}

#[test]
fn lookup_round_trips_every_id_and_name() {
    for code in Code::all() {
        let info = code.info();
        for query in [
            info.id.to_string(),
            info.id.to_ascii_lowercase(),
            info.name.to_string(),
            info.name.to_ascii_uppercase(),
            format!("  {}\n", info.id),
        ] {
            assert_eq!(Code::lookup(&query), Lookup::Found(code), "{query:?}");
        }
    }
}

#[test]
fn retired_ids_stay_retired() {
    for id in RETIRED {
        assert_eq!(Code::lookup(id), Lookup::Retired(id));
        assert_eq!(Code::lookup(&id.to_ascii_lowercase()), Lookup::Retired(id));
    }
}

#[test]
fn unknown_queries_suggest_the_closest_codes() {
    let closest = |query: &str| match Code::lookup(query) {
        Lookup::Unknown { closest } => closest,
        other => panic!("{query}: {other:?}"),
    };
    assert_eq!(closest("type-mismach").first(), Some(&Code::TypeMismatch));
    assert_eq!(
        closest("unbound-varible").first(),
        Some(&Code::UnboundVariable)
    );
    let near: Vec<&str> = closest("E0110").into_iter().map(Code::id).collect();
    assert_eq!(
        near[..2],
        ["E0100", "E0010"],
        "one edit away, nearest first"
    );
    assert_eq!(closest("not-a-real-diagnostic-at-all"), []);
}

#[test]
fn links_derive_from_the_id() {
    let code = Code::UnboundVariable;
    assert_eq!(code.anchor(), "e0004");
    assert_eq!(code.docs_url(), "https://meshlang.dev/docs/errors/#e0004");
    assert_eq!(code.explain_command(), "meshc explain E0004");
    assert_eq!(code.skill_ref(), "references/diagnostics.md#e0004");
    assert_eq!(Code::ClusterDeclarationInvalid.anchor(), "cfg0001");
}
