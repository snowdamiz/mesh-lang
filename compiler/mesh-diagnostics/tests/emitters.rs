use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::parse::{ParseStream, Parser};

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

// The test runner conversion for runtime/test errors and the constructor test.
// Lower this count as producers adopt their own diagnostic codes.
const UNCATEGORIZED_SITES: usize = 2;

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
    let tokens: Vec<_> = attr.stream().into_iter().collect();
    matches!(tokens.as_slice(), [TokenTree::Ident(cfg), TokenTree::Group(args)]
        if cfg == "cfg" && args.delimiter() == Delimiter::Parenthesis
            && matches!(args.stream().into_iter().collect::<Vec<_>>().as_slice(),
                [TokenTree::Ident(test)] if test == "test"))
}

fn walk(
    stream: TokenStream,
    skip_test: bool,
    visit: &mut impl FnMut(&[TokenTree]),
) -> syn::Result<()> {
    let mut tokens: Vec<_> = stream.into_iter().collect();
    let mut idx = 0;
    while let Some(token) = tokens.get(idx) {
        let tail = &tokens[idx..];
        if skip_test && is_punct(token, '#') && tail.get(1).is_some_and(is_cfg_test) {
            let rest: TokenStream = tail[2..].iter().cloned().collect();
            let skip = |input: ParseStream<'_>| {
                if input.parse::<syn::Item>().is_err() {
                    return Err(input.error("expected a cfg(test) item"));
                }
                input.parse::<TokenStream>()
            };
            tokens = skip.parse2(rest)?.into_iter().collect();
            idx = 0;
            continue;
        }
        visit(tail);
        if let TokenTree::Group(group) = token {
            walk(group.stream(), skip_test, visit)?;
        }
        idx += 1;
    }
    Ok(())
}

#[test]
fn libraries_do_not_print() {
    let mut sites = Vec::new();
    for name in LIBRARIES {
        rust_files(&compiler().join(name).join("src"), &mut |path, tokens| {
            walk(tokens, true, &mut |tokens| {
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
fn uncategorized_sites_only_decrease() {
    let mut sites = Vec::new();
    rust_files(&compiler(), &mut |path, tokens| {
        walk(tokens, false, &mut |tokens| {
            if let [TokenTree::Ident(ty), colon, colon2, TokenTree::Ident(method), TokenTree::Group(args), ..] = tokens {
                if ty == "Diagnostic" && is_punct(colon, ':') && is_punct(colon2, ':')
                    && method == "uncategorized" && args.delimiter() == Delimiter::Parenthesis
                {
                    sites.push(format!("{}:{}", path.display(), method.span().start().line));
                }
            }
        }).unwrap();
    });
    sites.sort();
    assert_eq!(sites.len(), UNCATEGORIZED_SITES, "{}", sites.join("\n"));
}
