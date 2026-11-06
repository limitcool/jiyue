//! JSON-RPC 2.0 framing for the MCP stdio transport: newline-delimited messages with a size
//! limit, error codes, response builders and protocol version negotiation.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// Newest protocol version this server speaks.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
/// Every protocol version the server accepts from a client.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// Longest incoming message accepted, in bytes (8 MiB).
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
/// Resource errors (unknown URI or note), as used by the MCP specification.
pub const RESOURCE_NOT_FOUND: i64 = -32002;

/// One unit read from the input stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A complete line, without its line terminator.
    Message(Vec<u8>),
    /// A line longer than the limit. Its bytes were discarded.
    TooLong,
    /// End of the input.
    Eof,
}

/// Reads the next line. Lines longer than `max_bytes` are skipped without being stored, so a
/// hostile client cannot make the server allocate without bound.
pub fn read_frame<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Frame> {
    let mut line: Vec<u8> = Vec::new();
    let mut too_long = false;
    let mut saw_bytes = false;
    loop {
        let (done, used) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if saw_bytes {
                    finish(line, too_long)
                } else {
                    Frame::Eof
                });
            }
            saw_bytes = true;
            let (chunk, done) = match available.iter().position(|&b| b == b'\n') {
                Some(i) => (&available[..i], true),
                None => (available, false),
            };
            if !too_long {
                if line.len() + chunk.len() > max_bytes {
                    too_long = true;
                    line = Vec::new();
                } else {
                    line.extend_from_slice(chunk);
                }
            }
            let used = if done { chunk.len() + 1 } else { chunk.len() };
            (done, used)
        };
        reader.consume(used);
        if done {
            return Ok(finish(line, too_long));
        }
    }
}

fn finish(mut line: Vec<u8>, too_long: bool) -> Frame {
    if too_long {
        return Frame::TooLong;
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Frame::Message(line)
}

/// Picks the protocol version for a client's request. Unknown or missing versions get the
/// newest version this server speaks.
pub fn negotiate_version(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|wanted| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|v| *v == wanted)
        })
        .unwrap_or(LATEST_PROTOCOL_VERSION)
}

/// A successful JSON-RPC response.
pub fn success_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// A JSON-RPC error response.
pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Writes one message as a single line and flushes.
pub fn write_message<W: Write>(out: &mut W, message: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
    bytes.push(b'\n');
    out.write_all(&bytes)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frames(input: &[u8], max: usize) -> Vec<Frame> {
        let mut reader = Cursor::new(input.to_vec());
        let mut out = Vec::new();
        loop {
            let frame = read_frame(&mut reader, max).expect("in-memory read cannot fail");
            let eof = frame == Frame::Eof;
            out.push(frame);
            if eof {
                return out;
            }
        }
    }

    #[test]
    fn splits_lines_and_strips_crlf() {
        let got = frames(b"one\ntwo\r\n", 100);
        assert_eq!(
            got,
            vec![
                Frame::Message(b"one".to_vec()),
                Frame::Message(b"two".to_vec()),
                Frame::Eof
            ]
        );
    }

    #[test]
    fn partial_last_line_is_still_returned() {
        let got = frames(b"abc", 100);
        assert_eq!(got, vec![Frame::Message(b"abc".to_vec()), Frame::Eof]);
    }

    #[test]
    fn empty_input_is_eof_immediately() {
        assert_eq!(frames(b"", 100), vec![Frame::Eof]);
    }

    #[test]
    fn blank_lines_are_empty_messages() {
        let got = frames(b"\n\n", 100);
        assert_eq!(
            got,
            vec![
                Frame::Message(Vec::new()),
                Frame::Message(Vec::new()),
                Frame::Eof
            ]
        );
    }

    #[test]
    fn overlong_line_is_discarded_and_the_next_one_survives() {
        let got = frames(b"abcdefghijklmnop\nok\n", 8);
        assert_eq!(
            got,
            vec![Frame::TooLong, Frame::Message(b"ok".to_vec()), Frame::Eof]
        );
    }

    #[test]
    fn overlong_partial_line_at_eof_is_reported_once() {
        let got = frames(b"0123456789", 4);
        assert_eq!(got, vec![Frame::TooLong, Frame::Eof]);
    }

    #[test]
    fn line_exactly_at_the_limit_is_accepted() {
        let got = frames(b"12345678\n", 8);
        assert_eq!(got, vec![Frame::Message(b"12345678".to_vec()), Frame::Eof]);
    }

    #[test]
    fn long_line_spanning_many_buffer_fills_is_read_whole() {
        let mut input = vec![b'x'; 20_000];
        input.push(b'\n');
        let mut reader = io::BufReader::with_capacity(16, Cursor::new(input.clone()));
        let frame = read_frame(&mut reader, MAX_MESSAGE_BYTES).expect("read");
        assert_eq!(frame, Frame::Message(input[..20_000].to_vec()));
    }

    #[test]
    fn negotiation_accepts_known_versions_and_defaults_to_latest() {
        assert_eq!(negotiate_version(Some("2025-03-26")), "2025-03-26");
        assert_eq!(negotiate_version(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate_version(Some("2025-06-18")), "2025-06-18");
        assert_eq!(
            negotiate_version(Some("1999-01-01")),
            LATEST_PROTOCOL_VERSION
        );
        assert_eq!(negotiate_version(None), LATEST_PROTOCOL_VERSION);
    }

    #[test]
    fn response_builders_have_the_jsonrpc_shape() {
        let ok = success_response(json!(7), json!({"x": 1}));
        assert_eq!(ok["jsonrpc"], "2.0");
        assert_eq!(ok["id"], 7);
        assert_eq!(ok["result"]["x"], 1);
        let err = error_response(Value::Null, PARSE_ERROR, "bad");
        assert_eq!(err["error"]["code"], -32700);
        assert_eq!(err["id"], Value::Null);
    }

    #[test]
    fn write_message_emits_one_terminated_line() {
        let mut out: Vec<u8> = Vec::new();
        write_message(&mut out, &json!({"a": "b"})).expect("write");
        assert_eq!(out, b"{\"a\":\"b\"}\n".to_vec());
    }
}
