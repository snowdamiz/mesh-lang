//! Render the catalog for the CLI, embedded skill, and website.

use std::fmt::Write;

use serde::Serialize;

use crate::catalog::RETIRED;
use crate::{Code, Example, Family, Status, DOCS_BASE};

const PREAMBLE: &str = include_str!("docs/preamble.md");
const WEBSITE_FRONTMATTER: &str = "\
---
title: Mesh diagnostics
description: Look up Mesh diagnostic codes, repairs, and examples.
outline: [2, 2]
---

";

const FAMILIES: &[(Family, &str, &str)] = &[
    (Family::Syntax, "`P`", "Lexing and parsing"),
    (Family::Semantic, "`E`, `W`", "Semantic errors and warnings"),
    (Family::Lint, "`L`", "Lint warnings"),
    (
        Family::Project,
        "`M`, `CFG`",
        "Project and cluster configuration",
    ),
    (Family::Verify, "`V`", "Verification and proofs"),
    (
        Family::Build,
        "`B`",
        "Targets, linking, and build artifacts",
    ),
    (Family::Runtime, "`R`", "Runtime failures"),
    (Family::Test, "`T`", "Assertions and the test harness"),
    (Family::Cli, "`X`", "Command-line tools"),
    (Family::Internal, "`I`", "Internal compiler diagnostics"),
    (Family::Uncategorized, "`C`", "Transitional diagnostics"),
];

/// Render a single Markdown section as it appears in both generated pages.
pub fn explain(code: Code) -> String {
    let info = code.info();
    let mut section = format!(
        "## {}\n\n**{}** · {} · {}",
        info.id,
        info.name,
        info.severity.as_str(),
        code.family().as_str(),
    );

    if let Some(topic) = info.topic {
        write!(section, " · guide: `meshc guide {topic}`").unwrap();
    }

    write!(
        section,
        "\n\n{}\n\n**Repair:** {}\n\n",
        info.summary, info.repair
    )
    .unwrap();

    if let Some(example) = info.example {
        for (label, program) in [("Example", example.bad), ("Fixed", example.fixed)] {
            writeln!(section, "**{label}**\n```mesh").unwrap();
            section.push_str(program);
            if !program.ends_with('\n') {
                section.push('\n');
            }
            section.push_str("```\n\n");
        }
    }

    writeln!(section, "Docs: {}", code.docs_url()).unwrap();
    section
}

/// List the IDs, names, and titles in catalog order.
pub fn index() -> String {
    let mut index = String::new();
    for code in Code::all() {
        let info = code.info();
        writeln!(index, "{}  {}  {}", info.id, info.name, info.title).unwrap();
    }
    index
}

pub fn skill_page() -> String {
    let mut page = String::from(PREAMBLE);
    page.push_str("\n## Code families\n\n");
    page.push_str("| Prefix | Category | Covers |\n| --- | --- | --- |\n");
    for &(family, prefix, description) in FAMILIES {
        writeln!(page, "| {prefix} | `{}` | {description} |", family.as_str()).unwrap();
    }

    for code in Code::all() {
        page.push('\n');
        page.push_str(&explain(code));
    }

    page.push_str("\n## Retired codes\n\n");
    let retired = RETIRED
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>();
    writeln!(
        page,
        "Mesh never reuses retired IDs. Retired IDs are {}.",
        retired.join(", ")
    )
    .unwrap();
    page
}

pub fn website_page() -> String {
    let mut page = String::from(WEBSITE_FRONTMATTER);
    page.push_str(&skill_page());
    page
}

#[derive(Serialize)]
struct Guide {
    explain: String,
    skill: String,
    topic: Option<&'static str>,
}

#[derive(Serialize)]
struct Entry {
    code: &'static str,
    name: &'static str,
    severity: &'static str,
    category: &'static str,
    title: &'static str,
    summary: &'static str,
    repair: &'static str,
    topic: Option<&'static str>,
    docs_url: String,
    guide: Guide,
    example: Option<Example>,
    status: Status,
}

impl From<Code> for Entry {
    fn from(code: Code) -> Self {
        let info = code.info();
        Self {
            code: info.id,
            name: info.name,
            severity: info.severity.as_str(),
            category: code.family().as_str(),
            title: info.title,
            summary: info.summary,
            repair: info.repair,
            topic: info.topic,
            docs_url: code.docs_url(),
            guide: Guide {
                explain: code.explain_command(),
                skill: code.skill_ref(),
                topic: info.topic,
            },
            example: info.example,
            status: info.status,
        }
    }
}

#[derive(Serialize)]
struct Catalog {
    schema: u32,
    docs_base: &'static str,
    codes: Vec<Entry>,
    retired: &'static [&'static str],
}

pub fn catalog_json() -> String {
    pretty_json(&Catalog {
        schema: 1,
        docs_base: DOCS_BASE,
        codes: Code::all().map(Entry::from).collect(),
        retired: RETIRED,
    })
}

pub fn entry_json(code: Code) -> String {
    pretty_json(&Entry::from(code))
}

fn pretty_json(value: &impl Serialize) -> String {
    let mut json = serde_json::to_string_pretty(value).expect("serialize the diagnostic catalog");
    json.push('\n');
    json
}
