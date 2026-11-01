//! Reads the page server's `/api/events` stream: server-sent events, split from bytes as
//! they arrive, whatever their chunking.

/// One event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
    /// `event:`, or `message` when absent.
    pub(crate) name: String,
    /// The `data:` lines, joined with newlines.
    pub(crate) data: String,
    /// `id:`, when the event had one.
    pub(crate) id: Option<String>,
}

/// Turns chunks of the stream into whole events.
#[derive(Debug, Default)]
pub(crate) struct Parser {
    /// Bytes after the last complete line.
    pending: Vec<u8>,
    name: Option<String>,
    data: Vec<String>,
    id: Option<String>,
}

impl Parser {
    /// Takes the next chunk and returns the events it completed.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<Event> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches('\n').trim_end_matches('\r');
            if line.is_empty() {
                events.extend(self.dispatch());
            } else {
                self.field(line);
            }
        }
        events
    }

    fn field(&mut self, line: &str) {
        if line.starts_with(':') {
            return;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.name = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            "id" => self.id = Some(value.to_owned()),
            _ => {}
        }
    }

    fn dispatch(&mut self) -> Option<Event> {
        let name = self.name.take();
        let id = self.id.take();
        if name.is_none() && self.data.is_empty() {
            return None;
        }
        let data = core::mem::take(&mut self.data).join("\n");
        Some(Event {
            name: name.unwrap_or_else(|| "message".to_owned()),
            data,
            id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_survive_any_chunking() {
        let stream = "event: entry\nid: 9f2c:231\ndata: {\"seq\":231}\n\n: comment\nevent: tick\r\ndata: {\"at_ms\":1}\r\n\r\ndata: a\ndata: b\n\n";
        let expected = vec![
            Event {
                name: "entry".to_owned(),
                data: "{\"seq\":231}".to_owned(),
                id: Some("9f2c:231".to_owned()),
            },
            Event {
                name: "tick".to_owned(),
                data: "{\"at_ms\":1}".to_owned(),
                id: None,
            },
            Event {
                name: "message".to_owned(),
                data: "a\nb".to_owned(),
                id: None,
            },
        ];
        for size in [1, 2, 7, stream.len()] {
            let mut parser = Parser::default();
            let events: Vec<Event> = stream
                .as_bytes()
                .chunks(size)
                .flat_map(|chunk| parser.push(chunk))
                .collect();
            assert_eq!(events, expected, "chunks of {size}");
        }
    }
}
