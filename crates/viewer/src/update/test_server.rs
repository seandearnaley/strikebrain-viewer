//! A local HTTP/1.1 server for tests: it answers each path with a canned
//! reply, closes every connection after one, and records each request, so
//! tests never reach the network. [`plain_as_tls`] makes an agent that
//! reaches it at `https` addresses too.

use std::collections::HashMap;
use std::io::{self, BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use ureq::Agent;
use ureq::config::Config;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, NextTimeout, TcpConnector, Transport,
};

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
/// in lower case, and whether the whole reply was written. The client may
/// hang up first, as on a body longer than it takes; `whole` is settled
/// once the server has [stopped](Server::stop).
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) line: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) whole: bool,
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
type Requests = Arc<Mutex<Vec<Request>>>;

/// The server, on `127.0.0.1` at a free port, until stopped or dropped.
#[derive(Debug)]
pub(crate) struct Server {
    address: SocketAddr,
    routes: Routes,
    requests: Requests,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let routes = Routes::default();
        let requests = Requests::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let (routes, requests, stop) = (routes.clone(), requests.clone(), stop.clone());
            move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        // A client that hangs up mid-request is its test's
                        // to notice; the server goes on.
                        let _answered = answer(&stream, &routes, &requests);
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
        lock(&self.routes).insert(path.to_owned(), reply);
    }

    /// The `http` URL of `path` on this server.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// The `https` URL of `path` on this server, which only an agent from
    /// [`plain_as_tls`] reaches.
    pub(crate) fn https_url(&self, path: &str) -> String {
        format!("https://{}{path}", self.address)
    }

    /// The requests read so far, in order.
    pub(crate) fn requests(&self) -> Vec<Request> {
        lock(&self.requests).clone()
    }

    /// Stops the server once it has finished every reply, and returns the
    /// requests it read.
    pub(crate) fn stop(mut self) -> Vec<Request> {
        self.shut_down();
        self.requests()
    }

    fn shut_down(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop, which then sees the flag.
        let _woken = TcpStream::connect(self.address);
        // The thread does not panic; if it did, a test already failing
        // should not panic again here.
        let _joined = thread.join();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// The value behind `mutex`, even if a panicking test left it poisoned.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads one request and writes its route's reply. A connection that does
/// not start like an HTTP request, such as a TLS handshake, is closed
/// unanswered and not recorded.
fn answer(stream: &TcpStream, routes: &Routes, requests: &Requests) -> io::Result<()> {
    let mut first = [0_u8];
    if stream.peek(&mut first)? == 0 || !first[0].is_ascii_uppercase() {
        return Ok(());
    }
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let line = line.trim_end().to_owned();
    let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
    let index = {
        let mut requests = lock(requests);
        requests.push(Request {
            line,
            headers,
            whole: false,
        });
        requests.len() - 1
    };
    let reply = lock(routes)
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
    stream.write_all(head.as_bytes())?;
    stream.write_all(&reply.body)?;
    if let Some(request) = lock(requests).get_mut(index) {
        request.whole = true;
    }
    Ok(())
}

/// An agent with `config` that speaks plain HTTP over TCP to `https`
/// addresses, telling ureq the connection is TLS. ureq refuses `https` over
/// a connection that is not TLS (`Error::TlsRequired`); this lets a test
/// start at `https` on the local server and watch where its redirects go.
pub(crate) fn plain_as_tls(config: Config) -> Agent {
    Agent::with_parts(config, PlainAsTls::default(), DefaultResolver::default())
}

#[derive(Debug, Default)]
struct PlainAsTls(TcpConnector);

impl Connector for PlainAsTls {
    type Out = ClaimsTls;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<ClaimsTls>, ureq::Error> {
        let tcp = self.0.connect(details, chained)?;
        Ok(tcp.map(|tcp| ClaimsTls(Box::new(tcp))))
    }
}

/// A plain TCP transport that says it is TLS.
#[derive(Debug)]
struct ClaimsTls(Box<dyn Transport>);

impl Transport for ClaimsTls {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.0.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.0.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.0.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.0.is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}
