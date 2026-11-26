//! A local HTTP/1.1 server for tests: it answers each path with a canned
//! reply, closes every connection after one, and records each request, so
//! tests never reach the network.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// A reply: its status, where it redirects, and its body.
/// `Content-Length` and `Connection: close` are added.
#[derive(Clone, Debug)]
pub(crate) struct Reply {
    status: &'static str,
    location: Option<String>,
    body: Vec<u8>,
}

impl Reply {
    /// `200 OK` with `body`.
    pub(crate) fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: "200 OK",
            location: None,
            body: body.into(),
        }
    }

    /// `302 Found` to `location`.
    pub(crate) fn redirect(location: &str) -> Self {
        Self {
            status: "302 Found",
            location: Some(location.to_owned()),
            body: Vec::new(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: "404 Not Found",
            location: None,
            body: Vec::new(),
        }
    }
}

/// A request as the server read it: its first line and its headers, names
/// in lower case.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) line: String,
    pub(crate) headers: Vec<(String, String)>,
}

impl Request {
    /// The value of the header `name` (lower case), if sent.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

type Routes = Arc<Mutex<HashMap<String, Reply>>>;

/// The server, on `127.0.0.1` at a free port, until dropped.
#[derive(Debug)]
pub(crate) struct Server {
    address: SocketAddr,
    routes: Routes,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let routes = Routes::default();
        let requests = Arc::<Mutex<Vec<Request>>>::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let (routes, requests, stop) = (routes.clone(), requests.clone(), stop.clone());
            move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        answer(&stream, &routes, &requests);
                    }
                }
            }
        });
        Self {
            address,
            routes,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// Answers `path` with `reply` from now on.
    pub(crate) fn route(&self, path: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(path.to_owned(), reply);
    }

    /// The `http` URL of `path` on this server.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// The requests read so far, in order.
    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop, which then sees the flag.
        let _woken = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

/// Reads one request and writes its route's reply. A connection that does
/// not start like an HTTP request, such as a TLS handshake, is closed
/// unanswered and not recorded.
fn answer(stream: &TcpStream, routes: &Routes, requests: &Mutex<Vec<Request>>) {
    let mut first = [0_u8];
    if stream.peek(&mut first).unwrap_or(0) == 0 || !first[0].is_ascii_uppercase() {
        return;
    }
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let line = line.trim_end().to_owned();
    let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
    requests.lock().unwrap().push(Request { line, headers });
    let reply = routes
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap_or_else(Reply::not_found);
    let location = reply
        .location
        .map_or_else(String::new, |to| format!("Location: {to}\r\n"));
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n{location}\r\n",
        reply.status,
        reply.body.len()
    );
    let mut stream = stream;
    // The client may hang up first, as on a body longer than it takes.
    let _written = stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(&reply.body));
}
