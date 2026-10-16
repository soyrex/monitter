//! Stateful VT screen, sequenced backend output and xterm-compatible input.
use serde_json::Value;

#[derive(Default)]
pub struct TerminalCallbacks {
    responses: Vec<u8>,
}
impl vt100::Callbacks for TerminalCallbacks {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if i2.is_some() {
            return;
        }
        let p = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let response = match (i1, c, p) {
            (None, 'n', 5) => "\x1b[0n".to_owned(),
            (None, 'n', 6) => {
                let (r, c) = screen.cursor_position();
                format!("\x1b[{};{}R", r + 1, c + 1)
            }
            (None, 'c', 0) => "\x1b[?1;2c".to_owned(),
            (Some(b'>'), 'c', 0) => "\x1b[>0;0;0c".to_owned(),
            _ => return,
        };
        // Bound responses if a hostile output stream asks endless questions.
        if self.responses.len() + response.len() <= 65536 {
            self.responses.extend(response.bytes());
        }
    }
    // Deliberately do not implement OSC52: terminal output cannot read or write
    // the system clipboard. Native Copy/Paste remain explicit owner actions.
}

pub struct TerminalModel {
    pub parser: vt100::Parser<TerminalCallbacks>,
    pub seq: u64,
    pub status: String,
    pub exit_code: Option<i32>,
    pub truncated: bool,
}
impl TerminalModel {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(
                rows.max(1),
                cols.max(1),
                10000,
                TerminalCallbacks::default(),
            ),
            seq: 0,
            status: "running".into(),
            exit_code: None,
            truncated: false,
        }
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
    }
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.parser.callbacks_mut().responses)
    }
    /// Validate the entire response before changing either cursor or screen.
    /// Duplicate/retried chunks never run escape sequences twice.
    pub fn apply_read(&mut self, read: &Value) -> Result<bool, String> {
        let next = read["nextSeq"]
            .as_u64()
            .ok_or("Terminal response has no sequence.")?;
        if next < self.seq {
            return Ok(false);
        }
        let chunks = read["chunks"]
            .as_array()
            .ok_or("Terminal response has no output chunks.")?;
        let truncated = read["truncated"]
            .as_bool()
            .ok_or("Terminal response has no truncation status.")?;
        let status = read["status"]
            .as_str()
            .filter(|s| matches!(*s, "running" | "exited"))
            .ok_or("Terminal response has invalid process status.")?;
        let exit_code = match &read["exitCode"] {
            Value::Null => None,
            v => Some(
                i32::try_from(v.as_i64().ok_or("Invalid terminal exit code.")?)
                    .map_err(|_| "Invalid terminal exit code.")?,
            ),
        };
        let mut cursor = self.seq;
        let mut previous = None;
        let mut bytes = Vec::new();
        for chunk in chunks {
            let seq = chunk["seq"]
                .as_u64()
                .ok_or("Terminal chunk has invalid sequence.")?;
            if previous.is_some_and(|p| seq <= p) {
                return Err("Terminal chunks are not ordered.".into());
            }
            previous = Some(seq);
            let data: Vec<u8> = serde_json::from_value(chunk["data"].clone())
                .map_err(|_| "Terminal chunk contains invalid bytes.")?;
            if seq <= self.seq {
                continue;
            }
            if seq != cursor.saturating_add(1) && !(truncated && cursor == self.seq) {
                return Err("Terminal output has an unreported gap.".into());
            }
            cursor = seq;
            bytes.extend(data);
            if bytes.len() > 256 * 1024 {
                return Err("Terminal output exceeds its response limit.".into());
            }
        }
        if next != cursor {
            return Err("Terminal response cursor does not match its chunks.".into());
        }
        let changed = !bytes.is_empty()
            || self.status != status
            || self.exit_code != exit_code
            || (truncated && !self.truncated);
        if truncated && !bytes.is_empty() {
            let (rows, cols) = self.parser.screen().size();
            self.parser =
                vt100::Parser::new_with_callbacks(rows, cols, 10000, TerminalCallbacks::default());
            self.truncated = true;
        }
        self.parser.process(&bytes);
        self.seq = next;
        self.status = status.to_owned();
        self.exit_code = exit_code;
        Ok(changed)
    }
    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        // Match terminal paste semantics: newlines enter as CR; no escape
        // characters from clipboard can terminate bracketed-paste mode early.
        let text = text
            .replace("\r\n", "\n")
            .replace('\n', "\r")
            .replace('\x1b', "");
        if self.parser.screen().bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.into_bytes()
        }
    }
    pub fn key_bytes(&self, key: &str, shift: bool, alt: bool, ctrl: bool) -> Option<Vec<u8>> {
        let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
        let cursor = match key {
            "up" => Some('A'),
            "down" => Some('B'),
            "right" => Some('C'),
            "left" => Some('D'),
            "home" => Some('H'),
            "end" => Some('F'),
            _ => None,
        };
        if let Some(c) = cursor {
            return Some(
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{c}")
                } else if self.parser.screen().application_cursor() {
                    format!("\x1bO{c}")
                } else {
                    format!("\x1b[{c}")
                }
                .into_bytes(),
            );
        }
        let code = match key {
            "insert" => Some(2),
            "delete" => Some(3),
            "pageup" => Some(5),
            "pagedown" => Some(6),
            "f5" => Some(15),
            "f6" => Some(17),
            "f7" => Some(18),
            "f8" => Some(19),
            "f9" => Some(20),
            "f10" => Some(21),
            "f11" => Some(23),
            "f12" => Some(24),
            _ => None,
        };
        if let Some(n) = code {
            return Some(
                if modifier > 1 {
                    format!("\x1b[{n};{modifier}~")
                } else {
                    format!("\x1b[{n}~")
                }
                .into_bytes(),
            );
        }
        if let Some(c) = match key {
            "f1" => Some('P'),
            "f2" => Some('Q'),
            "f3" => Some('R'),
            "f4" => Some('S'),
            _ => None,
        } {
            return Some(
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{c}")
                } else {
                    format!("\x1bO{c}")
                }
                .into_bytes(),
            );
        }
        let mut bytes = match key {
            "enter" | "return" => vec![b'\r'],
            "escape" => vec![0x1b],
            "backspace" => vec![if ctrl { 8 } else { 127 }],
            "tab" if shift => return Some(b"\x1b[Z".to_vec()),
            "tab" => vec![b'\t'],
            "space" if ctrl => vec![0],
            "space" => vec![b' '],
            _ if ctrl => {
                let mut chars = key.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                let c = c.to_ascii_uppercase();
                if ('@'..='_').contains(&c) {
                    vec![c as u8 & 0x1f]
                } else if c == '?' {
                    vec![127]
                } else {
                    return None;
                }
            }
            _ => return None, // Printable text is committed through native IME.
        };
        if alt {
            bytes.insert(0, 0x1b);
        }
        Some(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn read(seq: u64, bytes: &[u8]) -> Value {
        json!({"chunks":[{"seq":seq,"data":bytes}],"nextSeq":seq,"status":"running","exitCode":null,"truncated":false})
    }
    #[::core::prelude::v1::test]
    fn preserves_utf8_and_escape_state_across_chunks() {
        let mut m = TerminalModel::new(3, 20);
        m.apply_read(&read(1, b"\x1b[31m\xe7\x95")).unwrap();
        m.apply_read(&read(2, b"\x8c\x1b[0m!")).unwrap();
        assert_eq!(m.parser.screen().contents(), "界!");
        assert_eq!(
            m.parser.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        assert!(m.parser.screen().cell(0, 0).unwrap().is_wide());
    }
    #[::core::prelude::v1::test]
    fn duplicate_and_stale_pages_do_not_replay() {
        let mut m = TerminalModel::new(3, 20);
        let page = read(1, b"one");
        assert!(m.apply_read(&page).unwrap());
        assert!(!m.apply_read(&page).unwrap());
        m.apply_read(&read(2, b"two")).unwrap();
        assert!(!m.apply_read(&page).unwrap());
        assert_eq!(m.parser.screen().contents(), "onetwo");
    }
    #[::core::prelude::v1::test]
    fn bad_batch_does_not_partially_mutate_screen() {
        let mut m = TerminalModel::new(3, 20);
        let page = json!({"chunks":[{"seq":1,"data":[65]},{"seq":3,"data":[66]}],"nextSeq":3,"status":"running","exitCode":null,"truncated":false});
        assert!(m.apply_read(&page).is_err());
        assert_eq!(m.seq, 0);
        assert_eq!(m.parser.screen().contents(), "");
    }
    #[::core::prelude::v1::test]
    fn reported_truncation_resynchronizes() {
        let mut m = TerminalModel::new(3, 20);
        m.apply_read(&read(1, b"old")).unwrap();
        let mut p = read(9, b"new");
        p["truncated"] = json!(true);
        m.apply_read(&p).unwrap();
        assert!(m.truncated);
        assert_eq!(m.seq, 9);
        assert_eq!(m.parser.screen().contents(), "new");
    }
    #[::core::prelude::v1::test]
    fn alternate_screen_and_cursor_modes_restore() {
        let mut m = TerminalModel::new(3, 20);
        m.parser.process(b"shell\x1b[?1049h\x1b[?1happ");
        assert_eq!(m.key_bytes("up", false, false, false).unwrap(), b"\x1bOA");
        m.parser.process(b"\x1b[?1049l\x1b[?1l");
        assert_eq!(m.parser.screen().contents(), "shell");
        assert_eq!(m.key_bytes("left", true, true, true).unwrap(), b"\x1b[1;8D");
    }
    #[::core::prelude::v1::test]
    fn paste_and_terminal_responses_are_bounded_and_explicit() {
        let mut m = TerminalModel::new(3, 20);
        m.parser.process(b"hi\x1b[6n\x1b[5n\x1b[?2004h");
        assert_eq!(m.take_responses(), b"\x1b[1;3R\x1b[0n");
        assert_eq!(
            m.paste_bytes("a\nb\x1b[201~"),
            b"\x1b[200~a\rb[201~\x1b[201~"
        );
        assert!(m.take_responses().is_empty());
        assert_eq!(m.key_bytes("c", false, false, true).unwrap(), vec![3]);
    }
}
