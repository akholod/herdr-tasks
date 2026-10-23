//! Tiny HTTP/1.1 stub on 127.0.0.1 (std only): canned responses from a
//! handler, every request logged.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    pub fn header(&self, k: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn json(status: u16, body: impl Into<String>) -> Response {
        Response {
            status,
            headers: vec![],
            body: body.into(),
        }
    }
}

pub type Handler = Box<dyn FnMut(&Request) -> Response + Send>;

pub struct Stub {
    pub url: String,
    pub log: Arc<Mutex<Vec<Request>>>,
}

pub fn start(mut handler: Handler) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let path = parts.next().unwrap_or_default().to_string();
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                let mut l = String::new();
                if reader.read_line(&mut l).unwrap_or(0) == 0 {
                    break;
                }
                let l = l.trim_end();
                if l.is_empty() {
                    break;
                }
                if let Some((k, v)) = l.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let req = Request {
                method,
                path,
                headers,
                body: String::from_utf8_lossy(&body).into_owned(),
            };
            let resp = handler(&req);
            log2.lock().unwrap().push(req);
            let mut out = format!(
                "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                resp.status,
                resp.body.len()
            );
            for (k, v) in &resp.headers {
                out.push_str(&format!("{k}: {v}\r\n"));
            }
            out.push_str("\r\n");
            out.push_str(&resp.body);
            let _ = stream.write_all(out.as_bytes());
            let _ = stream.flush();
        }
    });
    Stub { url, log }
}
