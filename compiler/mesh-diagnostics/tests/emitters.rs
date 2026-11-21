use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use mesh_diagnostics::Code;
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::parse::{ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::{Meta, Token};

const LIBRARIES: &[&str] = &[
    "mesh-common",
    "mesh-lexer",
    "mesh-parser",
    "mesh-typeck",
    "mesh-codegen",
    "mesh-pkg",
    "mesh-lint",
    "mesh-fmt",
];

const UNWITNESSED: &[&str] = &[];

fn compiler() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path, visit: &mut impl FnMut(&Path, TokenStream)) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let entry = entry.unwrap();
        let path = entry.path();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            rust_files(&path, visit);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let source = fs::read_to_string(&path).unwrap();
            let tokens = source
                .parse()
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            visit(&path, tokens);
        }
    }
}

fn is_punct(token: &TokenTree, ch: char) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == ch)
}

fn is_cfg_test(token: &TokenTree) -> bool {
    let TokenTree::Group(attr) = token else {
        return false;
    };
    if attr.delimiter() != Delimiter::Bracket {
        return false;
    }
    matches!(syn::parse2::<Meta>(attr.stream()), Ok(Meta::List(cfg))
        if cfg.path.is_ident("cfg") && cfg.parse_args().is_ok_and(|meta| requires_test(&meta)))
}

fn requires_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) => {
            let Ok(args) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return false;
            };
            (list.path.is_ident("all") && args.iter().any(requires_test))
                || (list.path.is_ident("any") && !args.is_empty() && args.iter().all(requires_test))
        }
        Meta::NameValue(_) => false,
    }
}

fn walk(
    stream: TokenStream,
    mut test_only: bool,
    visit: &mut impl FnMut(&[TokenTree], bool),
) -> syn::Result<()> {
    let mut tokens: Vec<_> = stream.into_iter().collect();
    let mut idx = 0;
    while let Some(token) = tokens.get(idx) {
        let tail = &tokens[idx..];
        if !test_only && is_punct(token, '#') {
            if tail.get(1).is_some_and(is_cfg_test) {
                let rest: TokenStream = tail[2..].iter().cloned().collect();
                let split_item = |input: ParseStream<'_>| {
                    if input.fork().parse::<syn::Item>().is_ok() {
                        input.parse::<syn::Item>()?;
                    } else {
                        input.parse::<syn::Stmt>()?;
                    }
                    input.parse::<TokenStream>()
                };
                let rest: Vec<_> = split_item.parse2(rest)?.into_iter().collect();
                let end = tail.len() - rest.len();
                walk(tail[2..end].iter().cloned().collect(), true, visit)?;
                tokens = rest;
                idx = 0;
                continue;
            }
            if tail.get(1).is_some_and(|t| is_punct(t, '!')) && tail.get(2).is_some_and(is_cfg_test)
            {
                test_only = true;
            }
        }
        visit(tail, test_only);
        if let TokenTree::Group(group) = token {
            walk(group.stream(), test_only, visit)?;
        }
        idx += 1;
    }
    Ok(())
}

fn diagnostic_crates() -> impl Iterator<Item = PathBuf> {
    fs::read_dir(compiler())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name != "mesh-diagnostics")
                && path.join("src").is_dir()
        })
}

fn code_variant(tokens: &[TokenTree]) -> Option<String> {
    match tokens {
        [TokenTree::Ident(ty), colon, colon2, TokenTree::Ident(variant), ..]
            if ty == "Code" && is_punct(colon, ':') && is_punct(colon2, ':') =>
        {
            Some(variant.to_string())
        }
        _ => None,
    }
}

#[test]
fn libraries_do_not_print() {
    let mut sites = Vec::new();
    for name in LIBRARIES {
        rust_files(&compiler().join(name).join("src"), &mut |path, tokens| {
            walk(tokens, false, &mut |tokens, test_only| {
                if test_only {
                    return;
                }
                if let [TokenTree::Ident(name), bang, ..] = tokens {
                    if ["println", "eprintln", "print", "eprint"]
                        .iter()
                        .any(|mac| name == mac)
                        && is_punct(bang, '!')
                    {
                        sites.push(format!(
                            "{}:{}: {name}!",
                            path.display(),
                            name.span().start().line
                        ));
                    }
                }
            })
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        });
    }
    sites.sort();
    assert!(
        sites.is_empty(),
        "Library print macros:\n{}",
        sites.join("\n")
    );
}

#[test]
fn every_active_code_has_an_emitter() {
    let mut emitted = HashSet::new();
    for krate in diagnostic_crates() {
        rust_files(&krate.join("src"), &mut |path, tokens| {
            walk(tokens, false, &mut |tokens, test_only| {
                if test_only {
                    return;
                }
                if let Some(variant) = code_variant(tokens) {
                    emitted.insert(variant);
                }
                // This constructor reports I0001. Its definition lives in
                // mesh-diagnostics, so only its callers count as emitters.
                if matches!(tokens, [TokenTree::Ident(ty), colon, colon2, TokenTree::Ident(method), TokenTree::Group(args), ..]
                    if ty == "Diagnostic" && is_punct(colon, ':') && is_punct(colon2, ':')
                        && method == "internal" && args.delimiter() == Delimiter::Parenthesis)
                {
                    emitted.insert(format!("{:?}", Code::InternalCompilerError));
                }
            })
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        });
    }
    let missing: Vec<_> = Code::all()
        .filter(|code| !emitted.contains(&format!("{code:?}")))
        .map(|code| format!("{} ({code:?})", code.id()))
        .collect();
    assert!(
        missing.is_empty(),
        "Codes without a production emitter: {missing:?}"
    );
}

#[test]
fn every_active_code_has_a_witness() {
    let active_ids: HashSet<_> = Code::all().map(Code::id).collect();
    let mut witnessed_ids = HashSet::new();
    let mut witnessed_variants = HashSet::new();
    for krate in diagnostic_crates() {
        for (dir, test_only) in [("src", false), ("tests", true)] {
            let dir = krate.join(dir);
            if !dir.is_dir() {
                continue;
            }
            rust_files(&dir, &mut |path, tokens| {
                walk(tokens, test_only, &mut |tokens, test_only| {
                    if !test_only {
                        return;
                    }
                    if let Some(variant) = code_variant(tokens) {
                        witnessed_variants.insert(variant);
                    }
                    if let TokenTree::Literal(literal) = &tokens[0] {
                        for word in literal
                            .to_string()
                            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        {
                            if let Some(&id) = active_ids.get(word) {
                                witnessed_ids.insert(id);
                            }
                        }
                    }
                })
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            });
        }
    }
    let missing: Vec<_> = Code::all()
        .filter(|code| {
            code.info().example.is_none()
                && !witnessed_ids.contains(code.id())
                && !witnessed_variants.contains(&format!("{code:?}"))
        })
        .map(Code::id)
        .collect();
    assert_eq!(
        missing, UNWITNESSED,
        "UNWITNESSED must match the active codes without examples or test references"
    );
}
