//! Test-only helpers: a scripted HTTP/1.1 server built on `std::net::TcpListener`.
//!
//! Each connection is answered with `Connection: close`. The handler decides the reply from the
//! recorded request. Replies can be delayed between writes (to test idle timeouts and cancel) or
//! cut off early (to simulate a dropped connection mid-stream).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Pause between body writes (the body is split on blank lines for SSE, or in 64-byte
    /// pieces otherwise).
    pub write_delay: Option<Duration>,
    /// Stop writing after this many body bytes and close the socket (Content-Length still
    /// declares the full body, so the client sees a truncated message).
    pub cut_after: Option<usize>,
    /// Stall for this long after the headers and before the body; used for idle timeouts.
    pub stall_before_body: Option<Duration>,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
            write_delay: None,
            cut_after: None,
            stall_before_body: None,
        }
    }

    pub fn sse(events: &[&str]) -> Self {
        let mut body = String::new();
        for ev in events {
            body.push_str("data: ");
            body.push_str(ev);
            body.push_str("\n\n");
        }
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "text/event-stream".into())],
            body: body.into_bytes(),
            write_delay: None,
            cut_after: None,
            stall_before_body: None,
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn with_write_delay(mut self, d: Duration) -> Self {
        self.write_delay = Some(d);
        self
    }

    pub fn with_cut_after(mut self, bytes: usize) -> Self {
        self.cut_after = Some(bytes);
        self
    }

    pub fn with_stall(mut self, d: Duration) -> Self {
        self.stall_before_body = Some(d);
        self
    }
}

pub type Handler = dyn Fn(&Recorded) -> Reply + Send + Sync;

pub struct MockServer {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockServer {
    pub fn start(handler: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let rec = Arc::clone(&requests);
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(stream) = conn else { continue };
                let handler = Arc::clone(&handler);
                let rec = Arc::clone(&rec);
                thread::spawn(move || handle(stream, &*handler, &rec));
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            requests,
        }
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

fn handle(mut stream: TcpStream, handler: &Handler, rec: &Mutex<Vec<Recorded>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let reply = handler(&request);
    if let Ok(mut all) = rec.lock() {
        all.push(request);
    }
    write_reply(&mut stream, &reply);
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let content_length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body: Vec<u8> = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);
    Some(Recorded {
        method,
        path,
        headers,
        body,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn write_reply(stream: &mut TcpStream, reply: &Reply) {
    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reason(reply.status));
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
    head.push_str("Connection: close\r\n\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();
    if let Some(stall) = reply.stall_before_body {
        thread::sleep(stall);
    }

    let limit = reply
        .cut_after
        .unwrap_or(reply.body.len())
        .min(reply.body.len());
    let body = &reply.body[..limit];
    let pieces: Vec<&[u8]> = match reply.write_delay {
        Some(_) => split_events(body),
        None => vec![body],
    };
    for piece in pieces {
        if stream.write_all(piece).is_err() {
            return;
        }
        let _ = stream.flush();
        if let Some(d) = reply.write_delay {
            thread::sleep(d);
        }
    }
    // Dropping the stream closes the socket.
}

/// Splits an SSE body into pieces ending with a blank line, so each event is written separately.
fn split_events(body: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + 1 < body.len() {
        if body[i] == b'\n' && body[i + 1] == b'\n' {
            out.push(&body[start..i + 2]);
            start = i + 2;
            i += 2;
        } else {
            i += 1;
        }
    }
    if start < body.len() {
        out.push(&body[start..]);
    }
    out
}
