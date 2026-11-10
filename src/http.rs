//! A small HTTP/1.1 client for the loopback model server.
//!
//! It only issues POST. Responses may be `Content-Length` or chunked.
//! Redirects are not followed. The caller has already required a loopback URL.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::error::{err, Result};

const MAX_BODY: usize = 1024 * 1024;

pub fn post_json(url: &str, body: &str, timeout: Duration) -> Result<String> {
    let parsed = parse_http_url(url)?;
    let mut stream = connect(&parsed, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\
         Content-Length: {len}\r\n\
         \r\n\
         {body}",
        path = parsed.path,
        host = parsed.host_header,
        len = body.len(),
        body = body,
    );
    stream.write_all(request.as_bytes())?;
    let mut raw = Vec::new();
    stream
        .take((MAX_BODY + 64 * 1024) as u64)
        .read_to_end(&mut raw)?;
    parse_http_response(&raw)
}

struct HttpUrl {
    host: String,
    port: u16,
    host_header: String,
    path: String,
}

fn parse_http_url(url: &str) -> Result<HttpUrl> {
    let Some(rest) = url.strip_prefix("http://") else {
        return Err(err("only http:// URLs are supported for the model server"));
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], rest[index..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() {
        return Err(err("missing host"));
    }
    let (host, port) = split_host_port(authority)?;
    Ok(HttpUrl {
        host_header: authority.to_string(),
        host,
        port,
        path,
    })
}

fn split_host_port(authority: &str) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, port_part)) = rest.split_once(']') else {
            return Err(err("bad IPv6 host"));
        };
        let port = if let Some(port) = port_part.strip_prefix(':') {
            port.parse::<u16>().map_err(|_| err("bad port"))?
        } else if port_part.is_empty() {
            80
        } else {
            return Err(err("bad IPv6 host"));
        };
        return Ok((host.to_string(), port));
    }
    if let Some((host, port)) = authority.rsplit_once(':') {
        if !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) {
            let port = port.parse::<u16>().map_err(|_| err("bad port"))?;
            return Ok((host.to_string(), port));
        }
    }
    Ok((authority.to_string(), 80))
}

fn connect(url: &HttpUrl, timeout: Duration) -> Result<TcpStream> {
    let target = format!("{}:{}", url.host, url.port);
    let mut last = None;
    for address in target
        .to_socket_addrs()
        .map_err(|e| err(format!("could not resolve {target}: {e}")))?
    {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = Some(e),
        }
    }
    Err(err(format!(
        "local model endpoint refused the connection on {target} ({})",
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "no address".into())
    )))
}

fn parse_http_response(raw: &[u8]) -> Result<String> {
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| err("the model server closed the connection before the HTTP headers"))?;
    let header_bytes = &raw[..header_end];
    let body = &raw[header_end + 4..];
    let header = String::from_utf8_lossy(header_bytes);
    let mut lines = header.split("\r\n");
    let status = lines.next().ok_or_else(|| err("empty HTTP status"))?;
    let code = status.split_whitespace().nth(1).unwrap_or("0");
    if code != "200" {
        return Err(err(format!(
            "local model HTTP {code}: {}",
            String::from_utf8_lossy(&body[..body.len().min(400)])
        )));
    }
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    let bytes = if chunked {
        decode_chunked(body)?
    } else if let Some(len) = content_length {
        body.get(..len).unwrap_or(body).to_vec()
    } else {
        body.to_vec()
    };
    if bytes.len() > MAX_BODY {
        return Err(err("the model response is larger than 1 MiB"));
    }
    String::from_utf8(bytes).map_err(|_| err("the model response was not UTF-8"))
}

fn decode_chunked(mut body: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| err("truncated chunk size"))?;
        let size_text =
            std::str::from_utf8(&body[..line_end]).map_err(|_| err("bad chunk size"))?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| err("bad chunk size"))?;
        body = &body[line_end + 2..];
        if size == 0 {
            break;
        }
        if body.len() < size + 2 {
            return Err(err("truncated chunk"));
        }
        out.extend_from_slice(&body[..size]);
        if out.len() > MAX_BODY {
            return Err(err("the model response is larger than 1 MiB"));
        }
        body = &body[size..];
        if !body.starts_with(b"\r\n") {
            return Err(err("chunk is missing its trailing CRLF"));
        }
        body = &body[2..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloEXTRA";
        assert_eq!(parse_http_response(raw).unwrap(), "hello");
    }

    #[test]
    fn parses_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        assert_eq!(parse_http_response(raw).unwrap(), "hello");
    }

    #[test]
    fn refuses_non_200() {
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\nno";
        assert!(parse_http_response(raw).is_err());
    }
}
