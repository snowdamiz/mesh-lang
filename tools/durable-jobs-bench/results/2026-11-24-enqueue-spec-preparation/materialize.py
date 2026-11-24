#!/usr/bin/env python3
"""Apply the bounded experiment to a fresh copy of a pinned workspace."""
import difflib
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent


def replace(source, anchor, update):
    if source.count(anchor) != 1:
        raise ValueError(f"expected one anchor: {anchor}")
    return source.replace(anchor, update)


def candidate(source):
    new = replace(source, "    fn prepare(&mut self) -> Result<Admission, String> {", """    fn prepare(&mut self) -> Result<Admission, String> {
        self.prepare_with_payload(false)
    }

    fn prepare_for_enqueue(mut self) -> Result<Admission, String> {
        self.prepare_with_payload(true)
    }

    fn prepare_with_payload(&mut self, take_payload: bool) -> Result<Admission, String> {""")
    old = "        let mut spec = serde_json::to_value(self).map_err(|error| error.to_string())?;"
    new = replace(new, old, """        // Preserve the serializer's existing field positions and map ordering.
        // The enqueue path can move these trees because it owns the input spec.
        let payload = take_payload.then(|| {
            (std::mem::take(&mut self.args), std::mem::take(&mut self.metadata))
        });
""" + old + """
        if let Some((args, metadata)) = payload {
            spec["args"] = args;
            spec["metadata"] = metadata;
        }""")
    new = replace(new, "        let spec = spec.to_string();", """        let spec = if take_payload {
            serde_json::to_string(&spec).map_err(|error| error.to_string())?
        } else {
            spec.to_string()
        };""")
    return replace(new, ".map(|mut spec| spec.prepare())", ".map(JobSpec::prepare_for_enqueue)")


def methods(source, anchor):
    start = source.index(anchor)
    end = source.index("\n}\n\nstruct Admission", start)
    return start, end, source[start:end]


def instrument(stock):
    staged = stock.replace("fn prepare(&mut self)", "fn prepare_staged(&mut self, stage: &mut [u128; 5])")
    for anchor, index in [
        ("        name(&self.worker,", None),
        ("        let mut spec = serde_json::to_value", 1),
        ("        let spec = spec.to_string();", 2),
        ("        let fingerprint =", 3),
        ("        Ok(Admission", 4),
    ]:
        prefix = ""
        if index is not None:
            prefix += f"        stage[{index}] += mark.elapsed().as_nanos();\n"
        if index != 4:
            prefix += "        let mark = std::time::Instant::now();\n"
        staged = replace(staged, anchor, prefix + anchor)
    return staged


def main(tree, mode, patch_path):
    package = tree / "compiler/mesh-rt"
    store = package / "src/jobs/store.rs"
    source = store.read_text()
    patched = candidate(source)
    rel = "compiler/mesh-rt/src/jobs/store.rs"
    if patch_path:
        patch_path.write_text("".join(difflib.unified_diff(
            source.splitlines(True), patched.splitlines(True), "a/" + rel, "b/" + rel)))
    if mode == "broker":
        store.write_text(patched)
        return
    if mode != "component":
        raise ValueError(mode)
    # The measured stock arm calls the original method body verbatim.
    _, end, stock = methods(source, "    fn prepare(&mut self)")
    _, _, candidate_methods = methods(patched, "    fn prepare_for_enqueue")
    enqueue_start = source.index("pub(super) fn enqueue(")
    enqueue_end = source.index("\nfn set_based_enqueue_available(" if "    independent: bool," in source
                               else "\n#[derive(Deserialize, Serialize)]", enqueue_start)
    enqueue_candidate = replace(source[enqueue_start:enqueue_end], "pub(super) fn enqueue(",
                                "fn enqueue_candidate(")
    enqueue_candidate = replace(enqueue_candidate, ".map(|mut spec| spec.prepare())",
                                ".map(JobSpec::prepare_for_enqueue)")
    store.write_text(source[:end] + "\n\n" + candidate_methods + "\n\n" + instrument(stock) + source[end:]
                     + "\n" + enqueue_candidate
                     + '\n#[path = "enqueue_spec_probe.rs"]\nmod enqueue_spec_probe;\n'
                     'pub(crate) use enqueue_spec_probe::run as run_enqueue_spec_probe;\n')
    (store.parent / "enqueue_spec_wire.rs").write_bytes((HERE / "wire.rs").read_bytes())
    probe = (HERE / "probe.rs").read_text()
    field_check = "                #[cfg(probe_bulk)]\n                assert_eq!(a.independent, b.independent);\n"
    probe = replace(probe, field_check, field_check.split("\n", 1)[1] if "    independent: bool," in source else "")
    (store.parent / "enqueue_spec_probe.rs").write_text(probe)
    with (store.parent / "mod.rs").open("a") as f:
        f.write("\npub(crate) use store::run_enqueue_spec_probe;\n")
    with (package / "src/lib.rs").open("a") as f:
        f.write("\n#[doc(hidden)]\npub fn run_enqueue_spec_probe() { jobs::run_enqueue_spec_probe(); }\n")
    (package / "examples").mkdir(exist_ok=True)
    (package / "examples/enqueue-spec-component.rs").write_text(
        "fn main() { mesh_rt::run_enqueue_spec_probe(); }\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3]) if len(sys.argv) == 4 else None)
