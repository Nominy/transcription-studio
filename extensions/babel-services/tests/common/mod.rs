//! Loopback HTTP mock: serves canned responses in order and records raw requests.

#![allow(dead_code)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

pub struct MockResponse {
    pub status: &'static str,
    pub body: Vec<u8>,
    pub headers: Vec<&'static str>,
}

impl MockResponse {
    pub fn with_headers(body: &'static str, headers: Vec<&'static str>) -> Self {
        Self {
            headers,
            ..Self::from(body)
        }
    }

    /// A response with a non-200 status line, e.g. `"401 Unauthorized"`.
    pub fn with_status(status: &'static str, body: &'static str) -> Self {
        Self {
            status,
            ..Self::from(body)
        }
    }
}

impl From<&'static str> for MockResponse {
    fn from(body: &'static str) -> Self {
        Self::from(body.as_bytes().to_vec())
    }
}

impl From<String> for MockResponse {
    fn from(body: String) -> Self {
        Self::from(body.into_bytes())
    }
}

impl From<Vec<u8>> for MockResponse {
    fn from(body: Vec<u8>) -> Self {
        Self {
            status: "200 OK",
            body,
            headers: Vec::new(),
        }
    }
}

pub struct MockServer {
    pub base_url: String,
    handle: thread::JoinHandle<Vec<String>>,
}

impl MockServer {
    pub fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                requests.push(read_request(&mut stream));
                write_response(&mut stream, &response);
            }
            requests
        });
        Self { base_url, handle }
    }

    pub fn json(bodies: Vec<&'static str>) -> Self {
        Self::start(bodies.into_iter().map(MockResponse::from).collect())
    }

    /// Raw requests, lowercased for case-insensitive header matching.
    pub fn join(self) -> Vec<String> {
        self.handle.join().expect("mock server join")
    }
}

pub fn contains(request: &str, expected: &str) -> bool {
    request.contains(&expected.to_ascii_lowercase())
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&buffer).to_ascii_lowercase();
        if let Some(header_end) = text.find("\r\n\r\n") {
            let content_length = text[..header_end]
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buffer.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8(buffer)
        .expect("utf8 request")
        .to_ascii_lowercase()
}

fn write_response(stream: &mut TcpStream, response: &MockResponse) {
    let extra_headers: String = response
        .headers
        .iter()
        .map(|header| format!("{header}\r\n"))
        .collect();
    write!(
        stream,
        "HTTP/1.1 {}\r\n{extra_headers}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        response.status,
        response.body.len(),
    )
    .expect("write response head");
    stream
        .write_all(&response.body)
        .expect("write response body");
}
