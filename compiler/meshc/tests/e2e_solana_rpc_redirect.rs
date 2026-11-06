//! A configured RPC endpoint cannot replace itself via HTTP redirect.
#![cfg(unix)]
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
#[path = "support/test_artifacts.rs"]
mod artifacts;

fn read_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut data = Vec::new();
    let mut buffer = [0; 2048];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..count]);
        if let Some(end) = data.windows(4).position(|x| x == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&data[..end]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|x| x.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if data.len() >= end + 4 + length {
                break;
            }
        }
        assert!(data.len() < 8192);
    }
}

#[test]
fn compiled_rpc_send_rejects_post_to_get_redirect_to_another_endpoint() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("rpc-redirect");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("mesh.toml"),format!("[package]\nname=\"rpc-redirect\"\nversion=\"0.1.0\"\n[dependencies]\nmesh-solana={{path={:?}}}\n",artifacts::repo_root().join("packages/mesh-solana"))).unwrap();
    fs::write(project.join("main.mpl"),r#"from Solana.Read import rpc_request, rpc_send
fn proof() -> String!String do
  let client = Http.client()
  let response = rpc_send(client,Env.get("RPC_URL",""),rpc_request(73,"getGenesisHash","[]")?,1000,4096)
  Http.client_close(client)
  Ok(case response do
    Err(_) -> "redirect-rejected"
    Ok(_) -> "redirect-followed"
  end)
end
fn main() do
  case proof() do
    Ok(value) -> println(value)
    Err(error) -> println(error)
  end
end
"#).unwrap();
    for opt in ["0", "2"] {
        let executable = project.join(format!("proof-{opt}"));
        let build = Command::new(artifacts::meshc_bin())
            .args(["build", "--opt-level", opt])
            .arg(&project)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            artifacts::command_output_text(&build)
        );
        let unpinned = TcpListener::bind("127.0.0.1:0").unwrap();
        unpinned.set_nonblocking(true).unwrap();
        let destination = format!("http://{}/other", unpinned.local_addr().unwrap());
        let done = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let (stop, count) = (done.clone(), calls.clone());
        let target = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match unpinned.accept() {
                    Ok((mut stream, _)) => {
                        count.fetch_add(1, Ordering::Relaxed);
                        read_request(&mut stream);
                        let body = r#"{"jsonrpc":"2.0","id":73,"result":"replacement"}"#;
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        let pinned = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/rpc", pinned.local_addr().unwrap());
        let source = thread::spawn(move || {
            let (mut stream, _) = pinned.accept().unwrap();
            read_request(&mut stream);
            write!(stream,"HTTP/1.1 303 See Other\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let result = Command::new(&executable)
            .env("RPC_URL", url)
            .output()
            .unwrap();
        source.join().unwrap();
        done.store(true, Ordering::Release);
        target.join().unwrap();
        assert!(
            result.status.success(),
            "{}",
            artifacts::command_output_text(&result)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "redirect-rejected"
        );
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "unpinned endpoint received a redirected RPC"
        );
    }
}
