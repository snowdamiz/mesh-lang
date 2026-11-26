"""Attach the archived preparation probe to each public production variant."""
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
STORE = Path("compiler/mesh-rt/src/jobs/store.rs")


def append(path, text):
    with path.open("a") as file:
        file.write(text)


def main(tree, lane):
    store = tree / STORE
    production = store.read_text()
    start = ("    let input_len = specs.len();" if lane == "candidate" else
             "    let (batch, specs) = match decode(specs)?")
    first = production.index(start, production.index("pub(super) fn enqueue_with("))
    last = production.index("\n\n    db.simple(\"SAVEPOINT", first)
    prepare = production[first:last]
    store.write_text(production[:first] + "    let (batch, admissions) = prepare_enqueue(specs)?;"
                     + production[last:] +
                     "\nfn prepare_enqueue(specs: &str) -> Result<(bool, Vec<Admission>), String> {\n"
                     + prepare + "\n    Ok((batch, admissions))\n}\n"
                     '\n#[path = "public_large_probe.rs"]\nmod public_large_probe;\n'
                     "pub(crate) use public_large_probe::run as run_public_large_probe;\n")
    original = (HERE / "original-probe.rs").read_text()
    corpus = original[original.index("fn corpus("):original.index("fn output_digest(")]
    corpus = corpus.replace("fn corpus()", "fn corpus(compare: &mut impl FnMut(&str))")
    corpus = corpus.replace("output(input, Arm::Stock)", "prepare_enqueue(input)")
    timing = original[original.index("#[cfg(all(target_os"):original.index("pub(crate) fn run()")]
    selected = ('assert_eq!(super::dominant_plain_arg(&spec.args, input.len()).is_some(), eligible);'
                if lane == "candidate" else "let _ = (spec, eligible);")
    probe = (HERE / "probe.rs").read_text().replace("// SELECTED", selected)
    (store.parent / "public_large_probe.rs").write_text(probe + "\n" + corpus + "\n" + timing)
    append(store.parent / "mod.rs", "\npub(crate) use store::run_public_large_probe;\n")
    package = tree / "compiler/mesh-rt"
    append(package / "src/lib.rs", "\n#[doc(hidden)]\npub fn run_public_large_probe() { jobs::run_public_large_probe(); }\n")
    (package / "examples").mkdir(exist_ok=True)
    (package / "examples/public-large-probe.rs").write_text("fn main() { mesh_rt::run_public_large_probe(); }\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]), sys.argv[2])
