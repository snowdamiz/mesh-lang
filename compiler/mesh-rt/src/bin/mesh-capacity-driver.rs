fn main() {
    // The driver accepts connections for as long as it runs: it returns
    // only when it cannot serve.
    let error = mesh_rt::dist::driver_service::serve_docker_driver_from_env()
        .expect_err("the capacity driver serves until it fails");
    eprintln!("mesh capacity driver failed: {error}");
    std::process::exit(1);
}
