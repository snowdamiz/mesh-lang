//! Cross-module implementation identity includes every structural trait argument.

use mesh_typeck::error::TypeError;
use mesh_typeck::ty::Ty;
use mesh_typeck::{ExportedSymbols, ImportContext, ModuleExports, TypeckResult};

fn checked(source: &str, imports: &ImportContext) -> (mesh_parser::Parse, TypeckResult) {
    let parse = mesh_parser::parse(source);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let result = mesh_typeck::check_with_imports(&parse, imports);
    (parse, result)
}

fn exported(source: &str, imports: &ImportContext) -> ExportedSymbols {
    let (parse, result) = checked(source, imports);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    mesh_typeck::collect_exports(&parse, &result)
}

fn importing(modules: &[(&str, &ExportedSymbols)]) -> ImportContext {
    let mut imports = ImportContext::empty();
    for (name, exports) in modules {
        imports.all_trait_defs.extend(exports.trait_defs.clone());
        imports.all_trait_impls.extend(exports.trait_impls.clone());
        imports.module_exports.insert(
            (*name).to_string(),
            ModuleExports::new((*name).to_string(), exports),
        );
    }
    imports
}

fn from_arguments(exports: &ExportedSymbols) -> Vec<Vec<Ty>> {
    exports
        .trait_impls
        .iter()
        .filter(|implementation| implementation.trait_name == "From")
        .map(|implementation| implementation.trait_type_args.clone())
        .collect()
}

const FAILURE: &str = r#"
pub struct Failure do text :: String end
impl From<String> for Failure do
  fn from(text :: String) -> Failure do Failure { text: text } end
end
impl From<Int> for Failure do
  fn from(n :: Int) -> Failure do Failure { text: "number #{n}" } end
end
"#;

#[test]
fn every_error_conversion_crosses_the_module_boundary() {
    let exports = exported(FAILURE, &ImportContext::empty());
    let arguments = from_arguments(&exports);
    assert_eq!(arguments.len(), 2, "each conversion must be exported once");
    assert!(arguments.contains(&vec![Ty::string()]));
    assert!(arguments.contains(&vec![Ty::int()]));
    let imports = importing(&[("Failure", &exports)]);
    let (_, consumer) = checked(
        r#"
from Failure import Failure
fn text_error() -> Int!String do Err("text") end
fn int_error() -> Int!Int do Err(7) end
fn text_conversion() -> Int!Failure do Ok(text_error()?) end
fn int_conversion() -> Int!Failure do Ok(int_error()?) end
"#,
        &imports,
    );
    assert!(consumer.errors.is_empty(), "{:?}", consumer.errors);
    let (_, unsupported) = checked(
        "from Failure import Failure\nfn bad() -> Int!Bool do Err(true) end\nfn rejected() -> Int!Failure do Ok(bad()?) end\n",
        &imports,
    );
    assert!(
        !unsupported.errors.is_empty(),
        "an absent conversion stays absent"
    );
}

#[test]
fn a_new_local_argument_does_not_reexport_an_imported_sibling() {
    let first = exported(
        "pub struct Wrapper do n :: Int end\nimpl From<Int> for Wrapper do\nfn from(n :: Int) -> Wrapper do Wrapper { n: n * 2 } end\nend\n",
        &ImportContext::empty(),
    );
    let imports = importing(&[("First", &first)]);
    let second = exported(
        "from First import Wrapper\nimpl From<String> for Wrapper do\nfn from(text :: String) -> Wrapper do Wrapper { n: String.length(text) } end\nend\n",
        &imports,
    );
    assert_eq!(from_arguments(&second), vec![vec![Ty::string()]]);
    let pass_through = exported("from First import Wrapper\n", &imports);
    assert!(pass_through.trait_impls.is_empty());
    let all = importing(&[("First", &first), ("Second", &second)]);
    assert_eq!(
        all.all_trait_impls
            .iter()
            .filter(|i| i.trait_name == "From")
            .count(),
        2,
        "dependency traversal must not accumulate the imported implementation twice"
    );
    let (_, consumer) = checked(
        "from First import Wrapper\nimport Second\nfn check() -> Int do Wrapper.from(4).n + Wrapper.from(\"abc\").n end\n",
        &all,
    );
    assert!(consumer.errors.is_empty(), "{:?}", consumer.errors);
}

#[test]
fn aliases_and_whole_structural_arguments_survive_export() {
    let exports = exported(
        r#"
pub type Ints = List<Int>
pub struct Bag do n :: Int end
impl From<Ints> for Bag do
  fn from(xs :: Ints) -> Bag do Bag { n: List.length(xs) } end
end
impl From<List<String>> for Bag do
  fn from(xs :: List<String>) -> Bag do Bag { n: 100 + List.length(xs) } end
end
impl From<(Int, Int)> for Bag do
  fn from(pair :: (Int, Int)) -> Bag do
    let (a, b) = pair
    Bag { n: a + b }
  end
end
impl From<Fun(Int) -> Int> for Bag do
  fn from(f :: Fun(Int) -> Int) -> Bag do Bag { n: f(20) } end
end
"#,
        &ImportContext::empty(),
    );
    let arguments = from_arguments(&exports);
    assert_eq!(arguments.len(), 4);
    for argument in [
        Ty::list(Ty::int()),
        Ty::list(Ty::string()),
        Ty::Tuple(vec![Ty::int(), Ty::int()]),
        Ty::fun(vec![Ty::int()], Ty::int()),
    ] {
        assert!(
            arguments.contains(&vec![argument.clone()]),
            "missing {argument:?}"
        );
    }
    let (_, consumer) = checked(
        "from Bags import Bag, Ints\nfn check() -> Int do\nlet xs :: Ints = [1, 2]\nlet b :: Bag = xs.into()\nb.n + Bag.from([\"a\"]).n + Bag.from((3, 4)).n + Bag.from(fn n -> n + 1 end).n\nend\n",
        &importing(&[("Bags", &exports)]),
    );
    assert!(consumer.errors.is_empty(), "{:?}", consumer.errors);
}

#[test]
fn actual_duplicates_stay_errors_and_default_derives_export_once() {
    let (_, duplicate) = checked(
        "type Ints = List<Int>\nstruct Bag do n :: Int end\nimpl From<Ints> for Bag do\nfn from(xs :: Ints) -> Bag do Bag { n: List.length(xs) } end\nend\nimpl From<List<Int>> for Bag do\nfn from(xs :: List<Int>) -> Bag do Bag { n: 0 } end\nend\n",
        &ImportContext::empty(),
    );
    assert!(duplicate
        .errors
        .iter()
        .any(|e| matches!(e, TypeError::DuplicateImpl { .. })));
    let exports = exported(
        "pub struct Plain do n :: Int end\n",
        &ImportContext::empty(),
    );
    for name in ["Debug", "Eq", "Ord", "Hash"] {
        assert_eq!(
            exports
                .trait_impls
                .iter()
                .filter(|i| i.trait_name == name)
                .count(),
            1
        );
    }
    assert_eq!(exports.trait_impls.len(), 4);
}
