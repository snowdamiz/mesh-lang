fn main() {
    if let Err(error) = mesh_rt::dist::driver_service::serve_docker_driver_from_env() {
        eprintln!("mesh capacity driver failed: {error}");
        std::process::exit(1);
    }
}
