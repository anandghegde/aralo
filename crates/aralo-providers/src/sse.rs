//! Server-sent events, read incrementally from a body that arrives in chunks
//! of any size.
//!
//! It follows the WHATWG event-stream rules: lines end in CRLF, LF or a lone
//! CR; a line starting with `:` is a comment; `data` lines accumulate, joined
//! by newlines; a blank line dispatches the event. A chunk boundary may fall
//! anywhere, inside a line, between CR and LF, or inside a UTF-8 character,
//! and the events come out the same.

/// A line longer than this is refused rather than buffered without end. The
/// longest line a chat stream sends is one delta's JSON.
pub const MAX_LINE: usize = 1 << 20;

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event` field, when the stream named one.
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineTooLong;

#[derive(Debug, Default)]
pub struct SseParser {
    line: Vec<u8>,
    /// The last chunk ended in CR, so an LF at the start of the next one ends
    /// nothing.
    after_cr: bool,
    /// The first line has been seen, so a byte-order mark is no longer
    /// possible.
    started: bool,
    event: Option<String>,
    data: String,
    has_data: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a chunk and returns the events it completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, LineTooLong> {
        let mut events = Vec::new();
        for &byte in chunk {
            let after_cr = std::mem::take(&mut self.after_cr);
            match byte {
                b'\n' if after_cr => {}
                b'\n' | b'\r' => {
                    self.after_cr = byte == b'\r';
                    let line = std::mem::take(&mut self.line);
                    events.extend(self.line_ended(&line));
                }
                _ => {
                    if self.line.len() >= MAX_LINE {
                        return Err(LineTooLong);
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(events)
    }

    /// The body has ended. A last line with no newline is read, and an event
    /// with no blank line after it is dispatched: some servers close the
    /// connection straight after their final `data` line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        let line = std::mem::take(&mut self.line);
        if !line.is_empty() {
            if let Some(event) = self.line_ended(&line) {
                return Some(event);
            }
        }
        self.dispatch()
    }

    fn line_ended(&mut self, line: &[u8]) -> Option<SseEvent> {
        let mut line = line;
        if !self.started {
            self.started = true;
            line = line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(line);
        }
        if line.is_empty() {
            return self.dispatch();
        }
        if line[0] == b':' {
            return None;
        }
        let line = String::from_utf8_lossy(line);
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_ref(), ""),
        };
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_owned()),
            // `id` and `retry` are for reconnecting, which a chat stream
            // never does.
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if !std::mem::take(&mut self.has_data) {
            return None;
        }
        Some(SseEvent {
            event: event.filter(|name| !name.is_empty()),
            data: std::mem::take(&mut self.data),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(value: &str) -> SseEvent {
        SseEvent {
            event: None,
            data: value.into(),
        }
    }

    fn parse_whole(body: &[u8]) -> Vec<SseEvent> {
        let mut parser = SseParser::new();
        let mut events = parser.feed(body).unwrap();
        events.extend(parser.finish());
        events
    }

    #[test]
    fn a_split_anywhere_gives_the_same_events() {
        let body = "\u{feff}: keep-alive\r\ndata: {\"a\":\"héllo 👋\"}\r\n\r\nevent: x\ndata: one\ndata: two\n\ndata:three\r\rdata: [DONE]\n\n";
        let whole = parse_whole(body.as_bytes());
        assert_eq!(
            whole,
            vec![
                data("{\"a\":\"héllo 👋\"}"),
                SseEvent {
                    event: Some("x".into()),
                    data: "one\ntwo".into()
                },
                data("three"),
                data("[DONE]"),
            ]
        );
        let bytes = body.as_bytes();
        for split in 0..=bytes.len() {
            let mut parser = SseParser::new();
            let mut events = parser.feed(&bytes[..split]).unwrap();
            events.extend(parser.feed(&bytes[split..]).unwrap());
            events.extend(parser.finish());
            assert_eq!(events, whole, "split at byte {split}");
        }
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        for byte in bytes {
            events.extend(parser.feed(std::slice::from_ref(byte)).unwrap());
        }
        events.extend(parser.finish());
        assert_eq!(events, whole, "one byte at a time");
    }

    #[test]
    fn comments_and_empty_events_dispatch_nothing() {
        assert_eq!(
            parse_whole(
                b": OPENROUTER PROCESSING\n\n: ping\n\nevent: noise\n\nid: 7\nretry: 10\n\n"
            ),
            Vec::new()
        );
    }

    #[test]
    fn a_last_event_without_a_blank_line_is_kept() {
        assert_eq!(
            parse_whole(b"data: a\n\ndata: b"),
            vec![data("a"), data("b")]
        );
        assert_eq!(
            parse_whole(b"data: a\n\ndata: b\n"),
            vec![data("a"), data("b")]
        );
    }

    #[test]
    fn a_line_with_no_end_is_refused() {
        let mut parser = SseParser::new();
        let chunk = vec![b'x'; 64 * 1024];
        let mut outcome = Ok(Vec::new());
        for _ in 0..=(MAX_LINE / chunk.len()) {
            outcome = parser.feed(&chunk);
            if outcome.is_err() {
                break;
            }
        }
        assert_eq!(outcome, Err(LineTooLong));
    }
}
