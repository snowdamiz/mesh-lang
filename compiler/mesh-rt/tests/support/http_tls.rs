//! Ephemeral OpenSSL CA/server fixtures shared by owned HTTP regression tests.
//! No certificate keys are persisted outside the automatically removed tempdir.
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

pub struct TlsFixture {
    directory: tempfile::TempDir,
}

impl TlsFixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary TLS fixture directory");
        let fixture = Self { directory };
        fixture.openssl(&[
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
            "-subj",
            "/CN=Mesh owned test CA",
            "-days",
            "2",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-keyout",
            "ca.key",
            "-out",
            "ca.pem",
        ]);
        fixture.openssl(&[
            "req",
            "-new",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-keyout",
            "leaf.key",
            "-out",
            "leaf.csr",
        ]);
        std::fs::write(fixture.path("leaf.ext"),
            "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost\n")
            .expect("write test certificate extensions");
        fixture.sign_leaf("leaf.pem", "2");
        std::fs::write(fixture.path("index.txt"), "").unwrap();
        std::fs::write(fixture.path("serial"), "1000\n").unwrap();
        std::fs::write(fixture.path("ca.cnf"),
            "[ca]\ndefault_ca=issuer\n[issuer]\ndatabase=index.txt\nnew_certs_dir=.\ncertificate=ca.pem\nprivate_key=ca.key\nserial=serial\ndefault_md=sha256\npolicy=policy\n[policy]\ncommonName=supplied\n").unwrap();
        fixture.openssl(&[
            "ca",
            "-batch",
            "-notext",
            "-config",
            "ca.cnf",
            "-in",
            "leaf.csr",
            "-out",
            "expired.pem",
            "-extfile",
            "leaf.ext",
            "-startdate",
            "20200101000000Z",
            "-enddate",
            "20210101000000Z",
        ]);
        fixture
    }

    fn sign_leaf(&self, output: &str, days: &str) {
        self.openssl(&[
            "x509",
            "-req",
            "-in",
            "leaf.csr",
            "-CA",
            "ca.pem",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-days",
            days,
            "-extfile",
            "leaf.ext",
            "-out",
            output,
        ]);
    }

    fn openssl(&self, arguments: &[&str]) {
        let result = Command::new("openssl")
            .current_dir(self.directory.path())
            .args(arguments)
            .output()
            .expect("OpenSSL is required for owned HTTPS regression tests");
        assert!(
            result.status.success(),
            "OpenSSL fixture command failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    pub fn path(&self, file: &str) -> PathBuf {
        self.directory.path().join(file)
    }
    pub fn pem(&self, file: &str) -> Vec<u8> {
        std::fs::read(self.path(file)).expect("read test PEM")
    }

    pub fn serve(
        &self,
        listener: std::net::TcpListener,
        expired: bool,
        expected: usize,
        keep_alive: bool,
    ) -> std::thread::JoinHandle<(usize, usize)> {
        let connection = if keep_alive { "keep-alive" } else { "close" };
        self.serve_reply(
            listener,
            expired,
            expected,
            keep_alive,
            format!("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: {connection}\r\n\r\nok")
                .into_bytes(),
        )
    }

    pub fn serve_reply(
        &self,
        listener: std::net::TcpListener,
        expired: bool,
        expected: usize,
        keep_alive: bool,
        reply: Vec<u8>,
    ) -> std::thread::JoinHandle<(usize, usize)> {
        let config = self.server(expired);
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let received = Arc::new(AtomicUsize::new(0));
            let started = Instant::now();
            let mut workers = Vec::new();
            while workers.len() < expected && started.elapsed() < Duration::from_secs(5) {
                match listener.accept() {
                    Ok((tcp, _)) => {
                        tcp.set_nonblocking(false).unwrap();
                        tcp.set_read_timeout(Some(Duration::from_millis(500)))
                            .unwrap();
                        tcp.set_write_timeout(Some(Duration::from_millis(500)))
                            .unwrap();
                        let config = Arc::clone(&config);
                        let received = Arc::clone(&received);
                        let reply = reply.clone();
                        workers.push(std::thread::spawn(move || {
                            let mut tls = rustls::StreamOwned::new(
                                rustls::ServerConnection::new(config).unwrap(),
                                tcp,
                            );
                            loop {
                                let mut request = Vec::new();
                                while !request.ends_with(b"\r\n\r\n") {
                                    let mut byte = [0u8; 1];
                                    if tls.read_exact(&mut byte).is_err() {
                                        return;
                                    }
                                    if request.len() > 8192 {
                                        return;
                                    }
                                    request.push(byte[0]);
                                }
                                received.fetch_add(1, Ordering::SeqCst);
                                if tls.write_all(&reply).is_err() {
                                    return;
                                }
                                if tls.flush().is_err() {
                                    return;
                                }
                                if !keep_alive {
                                    tls.conn.send_close_notify();
                                    let _ = tls.flush();
                                    return;
                                }
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("owned TLS accept failed: {error}"),
                }
            }
            let accepted = workers.len();
            for worker in workers {
                worker.join().unwrap();
            }
            (accepted, received.load(Ordering::SeqCst))
        })
    }

    pub fn server(&self, expired: bool) -> std::sync::Arc<rustls::ServerConfig> {
        use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
        let _ = rustls::crypto::ring::default_provider().install_default();
        let file = if expired { "expired.pem" } else { "leaf.pem" };
        let certificates = CertificateDer::pem_file_iter(self.path(file))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let key = PrivateKeyDer::from_pem_file(self.path("leaf.key")).unwrap();
        std::sync::Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certificates, key)
                .unwrap(),
        )
    }
}
