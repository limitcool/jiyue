//! Incremental Server-Sent Events decoder.
//!
//! Bytes arrive in arbitrary chunks. Lines are split on `\n` bytes (which never occur inside a
//! UTF-8 multi-byte sequence), so a character split across two chunks is decoded correctly.

#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
    has_data: bool,
}

impl SseDecoder {
    /// Feeds raw bytes and returns the data payloads of every event that completed.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = self.pending.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line_bytes[..line_bytes.len() - 1]).into_owned();
            self.handle_line(line.strip_suffix('\r').unwrap_or(&line), &mut events);
        }
        events
    }

    /// Call at end of stream. Flushes a final event that was not followed by a blank line and an
    /// unterminated last line.
    pub(crate) fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if !self.pending.is_empty() {
            let rest = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            self.handle_line(rest.strip_suffix('\r').unwrap_or(&rest), &mut events);
        }
        self.dispatch(&mut events);
        events
    }

    fn handle_line(&mut self, line: &str, events: &mut Vec<String>) {
        if line.is_empty() {
            self.dispatch(events);
            return;
        }
        if line.starts_with(':') {
            return; // comment / keep-alive
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        if field == "data" {
            self.data.push(value.to_string());
            self.has_data = true;
        }
        // "event", "id" and "retry" are not needed by the supported protocols.
    }

    fn dispatch(&mut self, events: &mut Vec<String>) {
        if self.has_data {
            events.push(self.data.join("\n"));
            self.data.clear();
            self.has_data = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_events_on_blank_lines() {
        let mut d = SseDecoder::default();
        let events = d.push(b"data: one\n\ndata: two\n\n");
        assert_eq!(events, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn handles_crlf_and_comments_and_event_lines() {
        let mut d = SseDecoder::default();
        let events = d.push(b": keep-alive\r\nevent: message\r\ndata: x\r\n\r\n");
        assert_eq!(events, vec!["x".to_string()]);
    }

    #[test]
    fn joins_multiline_data() {
        let mut d = SseDecoder::default();
        let events = d.push(b"data: a\ndata: b\n\n");
        assert_eq!(events, vec!["a\nb".to_string()]);
    }

    #[test]
    fn reassembles_events_split_across_chunks_and_utf8() {
        let mut d = SseDecoder::default();
        let full = "data: 你好\n\n".as_bytes();
        // Split inside the multi-byte characters.
        let mut out = Vec::new();
        for chunk in full.chunks(2) {
            out.extend(d.push(chunk));
        }
        assert_eq!(out, vec!["你好".to_string()]);
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut d = SseDecoder::default();
        assert!(d.push(b"data: tail").is_empty());
        assert_eq!(d.finish(), vec!["tail".to_string()]);
    }

    #[test]
    fn data_line_without_colon_is_empty_payload() {
        let mut d = SseDecoder::default();
        assert_eq!(d.push(b"data\n\n"), vec![String::new()]);
    }
}
