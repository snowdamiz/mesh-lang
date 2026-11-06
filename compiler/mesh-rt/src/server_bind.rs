use std::net::{IpAddr, SocketAddr};

pub(crate) fn address(host: &str, port: i64) -> Result<SocketAddr, &'static str> {
    let port = u16::try_from(port).map_err(|_| "invalid server bind port")?;
    let ip = host
        .parse::<IpAddr>()
        .map_err(|_| "invalid server bind address")?;
    Ok(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_addresses_keep_the_requested_interface_and_port() {
        for host in ["127.0.0.1", "::1", "0.0.0.0", "::"] {
            for port in [0, 65535] {
                let addr = address(host, port).unwrap();
                assert_eq!(addr.ip(), host.parse::<IpAddr>().unwrap());
                assert_eq!(i64::from(addr.port()), port);
                let listener = std::net::TcpListener::bind(address(host, 0).unwrap()).unwrap();
                assert_eq!(listener.local_addr().unwrap().ip(), addr.ip());
            }
        }
    }
    #[test]
    fn invalid_inputs_never_become_a_different_listener_address() {
        for host in [
            "",
            "localhost",
            "127.0.0.1:80",
            "[::1]",
            " 127.0.0.1",
            "::1%lo0",
            "127.0.0.1\n",
        ] {
            assert_eq!(address(host, 1), Err("invalid server bind address"));
        }
        for port in [-1, 65536, i64::MAX, i64::MIN] {
            assert_eq!(address("127.0.0.1", port), Err("invalid server bind port"));
        }
    }
}
