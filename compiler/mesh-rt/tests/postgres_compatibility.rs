use mesh_rt::db::pg::{native_pg_close, native_pg_connect, native_pg_query};

#[test]
#[ignore = "run scripts/verify-postgres-jobs.sh --compatibility VERSION"]
fn native_driver_connects_to_the_expected_release_over_tls() {
    let expected_version = std::env::var("MESH_TEST_POSTGRES_VERSION").unwrap();
    let url = std::env::var("MESH_TEST_DATABASE_URL").unwrap();
    let mut conn = native_pg_connect(&url).expect("connect with the configured password");
    let session = native_pg_query(
        &mut conn,
        "SELECT current_setting('server_version') AS version, ssl::text, version AS protocol,
                $1::text AS payload FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
        &["mesh compatibility"],
    )
    .expect("query the connected session");
    native_pg_close(conn);

    let [row] = session.as_slice() else {
        panic!("expected one session, got {session:?}");
    };
    let [(_, version), (_, ssl), (_, protocol), (_, payload)] = row.as_slice() else {
        panic!("expected version, TLS state, TLS protocol, and a payload, got {row:?}");
    };
    assert_eq!(
        version.split_whitespace().next(),
        Some(expected_version.as_str())
    );
    assert_eq!(ssl, "true");
    assert_eq!(payload, "mesh compatibility");

    let (scheme, endpoint) = url.split_once("://").expect("PostgreSQL URL scheme");
    let (credentials, address) = endpoint
        .split_once('@')
        .expect("connection URL with credentials");
    let (user, password) = credentials.split_once(':').expect("password in the URL");
    assert!(!password.is_empty(), "the test requires a password");
    let wrong_password_url = format!("{scheme}://{user}:{password}-invalid@{address}");
    let error = native_pg_connect(&wrong_password_url)
        .err()
        .expect("the server must reject a wrong password");
    assert!(error.contains("password authentication failed"), "{error}");
    println!("PostgreSQL {version} over {protocol}. The server rejected a wrong password.");
}
