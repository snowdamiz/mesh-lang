#![allow(dead_code)]

//! A minimal WebSocket client for tests: the upgrade, then single frames.

use std::io::{Read, Write};

/// A minimal WebSocket client over `stream`: the upgrade, then frames.
pub struct TestWsClient<S: Read + Write> {
    stream: S,
}

impl<S: Read + Write> TestWsClient<S> {
    pub fn open(mut stream: S, path: &str) -> Self {
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&head);
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        Self { stream }
    }

    /// A masked frame, as a client sends.
    pub fn send(&mut self, opcode: u8, payload: &[u8]) {
        let mask = [0x12, 0x34, 0x56, 0x78];
        let mut frame = vec![0x80 | opcode, 0x80 | payload.len() as u8];
        frame.extend(mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, byte)| byte ^ mask[i % 4]),
        );
        self.stream.write_all(&frame).unwrap();
    }

    pub fn receive(&mut self) -> (u8, Vec<u8>) {
        let mut head = [0; 2];
        self.stream.read_exact(&mut head).unwrap();
        let length = match head[1] & 0x7f {
            126 => {
                let mut length = [0; 2];
                self.stream.read_exact(&mut length).unwrap();
                u16::from_be_bytes(length) as usize
            }
            length => length as usize,
        };
        let mut payload = vec![0; length];
        self.stream.read_exact(&mut payload).unwrap();
        (head[0] & 0x0f, payload)
    }

    /// The next frame that is not a ping, answering each ping before it with
    /// a pong, as a client must to be kept.
    pub fn receive_answering_pings(&mut self) -> (u8, Vec<u8>) {
        loop {
            match self.receive() {
                (9, payload) => self.send(10, &payload),
                frame => return frame,
            }
        }
    }
}
