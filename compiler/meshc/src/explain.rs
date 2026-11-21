use mesh_diagnostics::{docs, Code, Diagnostic, Lookup};

pub(crate) fn render(query: Option<&str>, json: bool) -> Result<String, Diagnostic> {
    let Some(query) = query else {
        return Ok(if json {
            docs::catalog_json()
        } else {
            docs::index()
        });
    };

    match Code::lookup(query) {
        Lookup::Found(code) => Ok(if json {
            docs::entry_json(code)
        } else {
            docs::explain(code)
        }),
        Lookup::Retired(id) => {
            let message = format!("{id} is retired. Mesh never reuses retired diagnostic IDs.");
            if json {
                let info = serde_json::json!({
                    "code": id,
                    "status": "retired",
                    "message": message,
                });
                Ok(format!("{info:#}\n"))
            } else {
                Ok(format!("{message}\n"))
            }
        }
        Lookup::Unknown { closest } => {
            let mut diagnostic = Diagnostic::new(
                Code::UnknownDiagnosticCode,
                format!("unknown diagnostic code or name `{query}`"),
            );
            if !closest.is_empty() {
                let commands = closest
                    .iter()
                    .map(|code| format!("`meshc explain {}`", code.id()))
                    .collect::<Vec<_>>()
                    .join(", ");
                diagnostic = diagnostic.with_repair(format!(
                    "Try {commands}. Run `meshc explain` to list diagnostic codes."
                ));
            }
            Err(diagnostic)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn explain_index() {
        let output = render(None, false).unwrap();
        assert!(output
            .lines()
            .any(|line| line == "E0004  unbound-variable  undefined variable"));
    }

    #[test]
    fn explain_code() {
        let output = render(Some("E0001"), false).unwrap();
        assert!(output.starts_with(
            "## E0001\n\n**type-mismatch** · error · semantic · guide: `meshc guide types`\n"
        ));
        assert!(output.contains("**Repair:**"));
        assert!(output.contains("**Example**\n```mesh\n"));
        assert!(output.contains("**Fixed**\n```mesh\n"));
        assert!(output.ends_with("Docs: https://meshlang.dev/docs/errors/#e0001\n"));
    }

    #[test]
    fn explain_name() {
        let output = render(Some("unbound-variable"), false).unwrap();
        assert!(output.starts_with("## E0004\n\n**unbound-variable**"));
    }

    #[test]
    fn explain_retired() {
        let output = render(Some("E0013"), false).unwrap();
        assert!(output.contains("E0013 is retired"));
        assert!(output.contains("never reuses retired diagnostic IDs"));
    }

    #[test]
    fn explain_catalog_json() {
        let output = render(None, true).unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(catalog["schema"], 1);
        assert_eq!(catalog["docs_base"], "https://meshlang.dev/docs/errors/");
        assert!(catalog["retired"]
            .as_array()
            .unwrap()
            .contains(&"E0013".into()));
        let codes = catalog["codes"].as_array().unwrap();
        let cfg = codes
            .iter()
            .find(|entry| entry["code"] == "CFG0001")
            .unwrap();
        assert_eq!(cfg["status"], "active");
        assert!(cfg.get("formerly").is_none());
    }

    #[test]
    fn explain_entry_json() {
        let output = render(Some("E0001"), true).unwrap();
        let entry: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(entry["code"], "E0001");
        assert_eq!(entry["name"], "type-mismatch");
        assert_eq!(entry["guide"]["explain"], "meshc explain E0001");
        assert_eq!(entry["guide"]["skill"], "references/diagnostics.md#e0001");

        let catalog: serde_json::Value =
            serde_json::from_str(&render(None, true).unwrap()).unwrap();
        assert!(catalog["codes"].as_array().unwrap().contains(&entry));
    }

    #[test]
    fn explain_retired_json() {
        let output = render(Some("E0013"), true).unwrap();
        let entry: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(entry["code"], "E0013");
        assert_eq!(entry["status"], "retired");
    }

    #[test]
    fn explain_unknown_suggests_codes() {
        let error = render(Some("unbound-varable"), false).unwrap_err();
        assert_eq!(error.code(), mesh_diagnostics::Code::UnknownDiagnosticCode);
        assert!(error.repair().contains("`meshc explain E0004`"));
    }
}
