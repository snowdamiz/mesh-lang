//! One worker must accept a request, run its yielding handler and drain it.
//! Each integration binary owns its process-wide scheduler/shutdown flag.
use mesh_rt::{
    actor::{mesh_actor_exit, mesh_actor_spawn, mesh_rt_init_actor, mesh_timer_sleep},
    http::{mesh_http_response_new, mesh_http_route_get, mesh_http_router, mesh_http_serve},
    process_signal::mesh_process_request_shutdown,
    string::{mesh_string_new, MeshString},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{Duration, Instant},
};

fn mesh_str(value: &str) -> *mut MeshString {
    mesh_string_new(value.as_ptr(), value.len() as u64)
}

extern "C-unwind" fn handler(_: *mut u8) -> *mut u8 {
    // The accept actor must also yield while draining this active connection.
    mesh_process_request_shutdown();
    mesh_timer_sleep(40);
    mesh_http_response_new(200, mesh_str("cooperative"))
}
struct Server {
    port: i64,
    done: mpsc::Sender<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.done.send(());
    }
}
extern "C-unwind" fn serve(argument: *const u8) {
    let server = unsafe { Box::from_raw(argument as *mut Server) };
    let router = mesh_http_route_get(
        mesh_http_router(),
        mesh_str("/"),
        handler as *mut u8,
        std::ptr::null_mut(),
    );
    mesh_http_serve(router, server.port);
}

#[test]
fn one_worker_serves_and_drains_a_yielding_http_handler() {
    // No elastic scheduler may conceal a blocked accept worker.
    std::env::remove_var("MESH_SCHEDULER_MIN_WORKERS");
    std::env::remove_var("MESH_SCHEDULER_MAX_WORKERS");
    let reservation = TcpListener::bind("[::1]:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    mesh_rt_init_actor(1);
    // Cancellation must unwind through the serving FFI and release the listener.
    let cancelled_port = TcpListener::bind("[::1]:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (cancelled, released) = mpsc::channel();
    let pid = mesh_actor_spawn(
        serve as *const u8,
        Box::into_raw(Box::new(Server {
            port: cancelled_port.into(),
            done: cancelled,
        }))
        .cast(),
        0,
        1,
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(connection) = TcpStream::connect(("::1", cancelled_port)) {
            drop(connection);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "cancel test listener did not bind"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    mesh_actor_exit(pid, 2);
    released
        .recv_timeout(Duration::from_secs(2))
        .expect("cancellation did not unwind the HTTP serve call");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(listener) = TcpListener::bind(("::", cancelled_port)) {
            drop(listener);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "cancelled HTTP listener remained bound"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let (done, completed) = mpsc::channel();
    mesh_actor_spawn(
        serve as *const u8,
        Box::into_raw(Box::new(Server {
            port: port.into(),
            done,
        }))
        .cast(),
        0,
        1,
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut connection = loop {
        if let Ok(stream) = TcpStream::connect(("::1", port)) {
            break stream;
        }
        assert!(Instant::now() < deadline, "HTTP listener did not bind");
        std::thread::sleep(Duration::from_millis(5));
    };
    connection
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    connection
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    connection
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut reply = String::new();
    connection
        .read_to_string(&mut reply)
        .expect("a single-worker HTTP server starved its own handler");
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.ends_with("cooperative"), "{reply}");
    completed
        .recv_timeout(Duration::from_secs(2))
        .expect("HTTP drain starved its yielding handler");
}
