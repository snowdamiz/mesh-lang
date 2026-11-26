use std::env;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(mesh_jobs_effects)");

    let target = |key| env::var(key).expect("Cargo target configuration");
    if target("CARGO_CFG_TARGET_OS") == "linux"
        && target("CARGO_CFG_TARGET_ENV") == "gnu"
        && matches!(
            target("CARGO_CFG_TARGET_ARCH").as_str(),
            "aarch64" | "x86_64"
        )
        && target("CARGO_CFG_TARGET_POINTER_WIDTH") == "64"
    {
        println!("cargo::rustc-cfg=mesh_jobs_effects");
    }
}
