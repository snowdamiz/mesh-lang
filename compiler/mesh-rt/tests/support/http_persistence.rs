//! An owned peer records actual request bytes and connection identity.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub enum Peer {
    Http10,
    Http10KeepAlive,
    Http11,
    Http11Close,
}

impl Peer {
    pub fn connections(self) -> usize {
        match self {
            Self::Http10 | Self::Http11Close => 2,
            Self::Http10KeepAlive | Self::Http11 => 1,
        }
    }
}

#[derive(Debug, Default)]
pub struct Observation {
    pub connections: usize,
    pub requests: Vec<(usize, String, Vec<u8>)>,
    pub unexpected_reuse: Vec<u8>,
    pub errors: Vec<String>,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
        assert!(head.len() <= 8192, "oversized fixture request");
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .expect("POST must have an explicit length");
    assert!(length <= 16);
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok((head, body))
}

fn accept(listener: &TcpListener) -> std::io::Result<TcpStream> {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit O_NONBLOCK on some hosts.
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(4)))?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "a new connection was required",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn serve(peer: Peer) -> (String, JoinHandle<Observation>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/effect", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let mut seen = Observation::default();
        let operation = |seen: &mut Observation| -> std::io::Result<()> {
            let mut stream = accept(&listener)?;
            seen.connections += 1;
            let (head, body) = read_request(&mut stream)?;
            seen.requests.push((seen.connections, head, body));
            let (version, connection) = match peer {
                Peer::Http10 => ("HTTP/1.0", ""),
                Peer::Http10KeepAlive => ("HTTP/1.0", "Connection: Keep-Alive\r\n"),
                Peer::Http11 => ("HTTP/1.1", ""),
                Peer::Http11Close => ("HTTP/1.1", "Connection: close\r\n"),
            };
            write!(
                stream,
                "{version} 200 OK\r\nContent-Length: 3\r\n{connection}\r\none"
            )?;
            stream.flush()?;
            if peer.connections() == 2 {
                // The response is complete while the old socket is still open.
                // Reuse during this window is incorrect, even if a later EOF
                // check or a replay on a new socket would hide it from callers.
                std::thread::sleep(Duration::from_millis(100));
                stream.set_nonblocking(true)?;
                let mut bytes = [0; 8192];
                match stream.read(&mut bytes) {
                    Ok(n) => seen.unexpected_reuse.extend_from_slice(&bytes[..n]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                    Err(error) => return Err(error),
                }
                drop(stream);
                stream = accept(&listener)?;
                seen.connections += 1;
            }
            let (head, body) = read_request(&mut stream)?;
            seen.requests.push((seen.connections, head, body));
            write!(
                stream,
                "{version} 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\ntwo"
            )?;
            stream.flush()?;
            drop(stream);
            // A replay must not turn a transport error into a third effect.
            let deadline = Instant::now() + Duration::from_millis(100);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((_, _)) => seen.connections += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        };
        if let Err(error) = operation(&mut seen) {
            seen.errors.push(error.to_string());
        }
        seen
    });
    (url, thread)
}

pub fn verify(peer: Peer, seen: &Observation) {
    assert!(seen.errors.is_empty(), "{seen:?}");
    assert!(seen.unexpected_reuse.is_empty(), "{seen:?}");
    assert_eq!(seen.connections, peer.connections(), "{seen:?}");
    assert_eq!(seen.requests.len(), 2, "{seen:?}");
    for ((_, head, body), expected) in seen.requests.iter().zip([b"first".as_slice(), b"second"]) {
        assert!(head.starts_with("POST /effect HTTP/1.1\r\n"), "{seen:?}");
        assert_eq!(body, expected, "{seen:?}");
    }
    assert_eq!(seen.requests[0].0, 1);
    assert_eq!(seen.requests[1].0, peer.connections());
}
