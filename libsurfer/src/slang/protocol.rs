//! JSON-RPC framing and message classification for the Language Server Protocol.
//!
//! The wire format is `Content-Length: N\r\n\r\n<N bytes of JSON>`. Nothing here knows
//! about processes; both the child-process transport and the replay transport used by
//! tests share these helpers.

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

/// Serializes one message with the LSP header.
pub fn write_frame(mut out: impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(&body)?;
    out.flush()
}

/// Reads one framed message. Returns `None` at a clean end of stream.
pub fn read_frame(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            // Tolerate stray blank lines before a header.
            continue;
        }
        if let Some(value) = header
            .strip_prefix("Content-Length:")
            .or_else(|| header.strip_prefix("content-length:"))
        {
            length = Some(value.trim().parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "bad Content-Length header")
            })?);
        }
    }
    let length = length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no length"))?;
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// A message received from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// Reply to one of our requests.
    Response {
        id: u64,
        result: Option<Value>,
        error: Option<Value>,
    },
    /// The server asks us something and expects a reply carrying `id`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

/// Classifies a decoded JSON-RPC message.
pub fn classify(message: Value) -> Option<Incoming> {
    let object = message.as_object()?;
    let method = object.get("method").and_then(Value::as_str);
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    match (method, object.get("id")) {
        (Some(method), Some(id)) => Some(Incoming::Request {
            id: id.clone(),
            method: method.to_owned(),
            params,
        }),
        (Some(method), None) => Some(Incoming::Notification {
            method: method.to_owned(),
            params,
        }),
        (None, Some(id)) => Some(Incoming::Response {
            id: id.as_u64()?,
            result: object.get("result").cloned(),
            error: object.get("error").cloned(),
        }),
        (None, None) => None,
    }
}

pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

pub fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Converts a filesystem path to a `file://` URI. Paths are expected to be absolute.
pub fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => {
                uri.push(byte as char);
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// Extracts the path of a `file://` URI, decoding percent escapes.
pub fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' && index + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[index + 1..index + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frames_round_trip() {
        let message = json!({"jsonrpc": "2.0", "id": 3, "method": "x", "params": {"a": [1, 2]}});
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &message).unwrap();
        assert!(buffer.starts_with(b"Content-Length: "));
        let mut reader = Cursor::new(buffer);
        assert_eq!(read_frame(&mut reader).unwrap(), Some(message));
        assert_eq!(read_frame(&mut reader).unwrap(), None);
    }

    #[test]
    fn frames_tolerate_extra_headers() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let mut buffer = format!(
            "Content-Type: application/vscode-jsonrpc\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        buffer.extend_from_slice(body);
        let value = read_frame(&mut Cursor::new(buffer)).unwrap().unwrap();
        assert_eq!(
            classify(value),
            Some(Incoming::Response {
                id: 1,
                result: Some(Value::Null),
                error: None
            })
        );
    }

    #[test]
    fn classification_distinguishes_requests_and_notifications() {
        let request = json!({"jsonrpc": "2.0", "id": "r1", "method": "client/registerCapability", "params": {}});
        assert!(
            matches!(classify(request), Some(Incoming::Request { method, .. }) if method == "client/registerCapability")
        );
        let note = json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": "file:///a"}});
        assert!(
            matches!(classify(note), Some(Incoming::Notification { method, .. }) if method == "textDocument/publishDiagnostics")
        );
        assert_eq!(classify(json!({"jsonrpc": "2.0"})), None);
    }

    #[test]
    fn file_uris_escape_and_decode() {
        assert_eq!(file_uri("/a b/c#.sv"), "file:///a%20b/c%23.sv");
        assert_eq!(
            uri_path("file:///a%20b/c%23.sv").as_deref(),
            Some("/a b/c#.sv")
        );
        assert_eq!(uri_path("http://x"), None);
    }
}
