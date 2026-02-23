//! Embedded UI server for netsim run artifacts.

use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const UI_INDEX: &str = include_str!("../../ui/dist/index.html");
/// Default bind address for the embedded UI server.
pub const DEFAULT_UI_BIND: &str = "127.0.0.1:7421";

/// Running embedded UI server handle.
pub struct UiServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl UiServer {
    /// Base HTTP URL.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Open the base URL in a browser.
    pub fn open_browser(&self) -> Result<()> {
        open_browser(&self.url())
    }
}

impl Drop for UiServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Start serving embedded UI + work-root files.
pub fn start_ui_server(work_root: PathBuf, bind: &str) -> Result<UiServer> {
    fs::create_dir_all(&work_root)
        .with_context(|| format!("create work root {}", work_root.display()))?;
    let listener = TcpListener::bind(bind).with_context(|| format!("bind UI server on {bind}"))?;
    listener
        .set_nonblocking(true)
        .context("set UI listener nonblocking")?;
    let addr = listener.local_addr().context("get UI listener address")?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let join = thread::spawn(move || {
        while !stop_thread.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _peer)) => {
                    let _ = handle_client(stream, &work_root);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
    });
    Ok(UiServer {
        addr,
        stop,
        join: Some(join),
    })
}

/// Open URL in default browser.
pub fn open_browser(url: &str) -> Result<()> {
    webbrowser::open(url).context("open browser")?;
    Ok(())
}

fn handle_client(mut stream: TcpStream, work_root: &Path) -> Result<()> {
    let mut buf = [0u8; 16 * 1024];
    let read = stream.read(&mut buf).context("read HTTP request")?;
    if read == 0 {
        return Ok(());
    }
    let req = String::from_utf8_lossy(&buf[..read]);
    let mut lines = req.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_path = parts.next().unwrap_or("/");
    let range_header = lines
        .clone()
        .find_map(|line| line.strip_prefix("Range:").map(str::trim));
    if method != "GET" && method != "HEAD" {
        write_response(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
            method == "HEAD",
        )?;
        return Ok(());
    }

    let (path, query) = match raw_path.split_once('?') {
        Some((path, query)) => (path, query),
        None => (raw_path, ""),
    };
    if path == "/" || path == "/index.html" {
        write_response(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            UI_INDEX.as_bytes(),
            method == "HEAD",
        )?;
        return Ok(());
    }
    if path == "/__netsim/runs" {
        let body = runs_json(work_root).context("build runs endpoint body")?;
        write_response(
            &mut stream,
            200,
            "application/json; charset=utf-8",
            body.as_bytes(),
            method == "HEAD",
        )?;
        return Ok(());
    }

    if path.contains("..") {
        write_response(
            &mut stream,
            403,
            "text/plain; charset=utf-8",
            b"forbidden",
            method == "HEAD",
        )?;
        return Ok(());
    }
    let rel = path.trim_start_matches('/');
    let full = work_root.join(rel);
    if !full.exists() || !full.is_file() {
        write_response(
            &mut stream,
            404,
            "text/plain; charset=utf-8",
            b"not found",
            method == "HEAD",
        )?;
        return Ok(());
    }
    if query_has_flag(query, "__meta") {
        let meta = read_log_meta(&full)?;
        let body = serde_json::json!({
            "path": rel,
            "size_bytes": meta.size_bytes,
            "line_count": meta.line_count,
        })
        .to_string();
        write_response(
            &mut stream,
            200,
            "application/json; charset=utf-8",
            body.as_bytes(),
            method == "HEAD",
        )?;
        return Ok(());
    }
    serve_file(
        &mut stream,
        &full,
        guess_mime(&full),
        method == "HEAD",
        range_header,
    )?;
    Ok(())
}

fn serve_file(
    stream: &mut TcpStream,
    full: &Path,
    content_type: &str,
    head_only: bool,
    range_header: Option<&str>,
) -> Result<()> {
    let mut file = fs::File::open(full).with_context(|| format!("open {}", full.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("stat {}", full.display()))?
        .len();
    if let Some(range) = range_header {
        let (start, end) = match parse_range(range, len) {
            Ok(v) => v,
            Err(_) => {
                write_response_with_headers(
                    stream,
                    416,
                    content_type,
                    b"",
                    head_only,
                    &[("Accept-Ranges", "bytes"), ("Content-Range", "bytes */0")],
                )?;
                return Ok(());
            }
        };
        let size = (end - start + 1) as usize;
        let mut bytes = vec![0u8; size];
        file.seek(SeekFrom::Start(start))
            .with_context(|| format!("seek {}", full.display()))?;
        file.read_exact(&mut bytes)
            .with_context(|| format!("read range {}", full.display()))?;
        let content_range = format!("bytes {}-{}/{}", start, end, len);
        write_response_with_headers(
            stream,
            206,
            content_type,
            &bytes,
            head_only,
            &[
                ("Accept-Ranges", "bytes"),
                ("Content-Range", &content_range),
            ],
        )?;
        return Ok(());
    }
    let bytes = fs::read(full).with_context(|| format!("read {}", full.display()))?;
    write_response_with_headers(
        stream,
        200,
        content_type,
        &bytes,
        head_only,
        &[("Accept-Ranges", "bytes")],
    )?;
    Ok(())
}

fn parse_range(header: &str, len: u64) -> Result<(u64, u64)> {
    if len == 0 {
        anyhow::bail!("empty file");
    }
    let raw = header
        .strip_prefix("bytes=")
        .context("unsupported range unit")?;
    let (start_s, end_s) = raw.split_once('-').context("invalid range")?;
    if start_s.is_empty() {
        let suffix: u64 = end_s.parse().context("invalid suffix range")?;
        let take = suffix.min(len);
        let start = len.saturating_sub(take);
        let end = len - 1;
        return Ok((start, end));
    }
    let start: u64 = start_s.parse().context("invalid range start")?;
    if start >= len {
        anyhow::bail!("range start out of bounds");
    }
    let end = if end_s.is_empty() {
        len - 1
    } else {
        let parsed: u64 = end_s.parse().context("invalid range end")?;
        parsed.min(len - 1)
    };
    if end < start {
        anyhow::bail!("range end before start");
    }
    Ok((start, end))
}

fn query_has_flag(query: &str, name: &str) -> bool {
    query.split('&').any(|part| {
        if let Some((k, v)) = part.split_once('=') {
            k == name && v == "1"
        } else {
            part == name
        }
    })
}

struct LogMeta {
    size_bytes: u64,
    line_count: u64,
}

fn read_log_meta(path: &Path) -> Result<LogMeta> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let size_bytes = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    let mut line_count = 0u64;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::with_capacity(16 * 1024);
    loop {
        let read = reader.read_until(b'\n', &mut buf)?;
        if read == 0 {
            break;
        }
        line_count += 1;
        buf.clear();
    }
    Ok(LogMeta {
        size_bytes,
        line_count,
    })
}

fn runs_json(work_root: &Path) -> Result<String> {
    let mut runs = Vec::new();
    for entry in fs::read_dir(work_root).with_context(|| format!("read {}", work_root.display()))? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !path.is_dir() || name.starts_with('.') || name == "latest" {
            continue;
        }
        runs.push(name.to_string());
    }
    runs.sort();
    runs.reverse();
    Ok(serde_json::json!({
        "workRoot": work_root.display().to_string(),
        "runs": runs,
    })
    .to_string())
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> Result<()> {
    write_response_with_headers(stream, status, content_type, body, head_only, &[])
}

fn write_response_with_headers(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    extra_headers: &[(&str, &str)],
) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        _ => "Error",
    };
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n",
        status, status_text, content_type, body.len()
    );
    for (name, value) in extra_headers {
        headers.push_str(name);
        headers.push_str(": ");
        headers.push_str(value);
        headers.push_str("\r\n");
    }
    headers.push_str("\r\n");
    stream
        .write_all(headers.as_bytes())
        .context("write response headers")?;
    if !head_only {
        stream.write_all(body).context("write response body")?;
    }
    Ok(())
}

fn guess_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "md" | "log" | "txt" => "text/plain; charset=utf-8",
        "qlog" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}
