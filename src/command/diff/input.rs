//! Read terminal events without losing Kitty's base-layout key before shortcuts are matched.
//! Crossterm still owns terminal output and raw mode; its published event parser drops that key.

#[cfg(not(unix))]
use std::{io, time::Duration};

#[cfg(not(unix))]
use crossterm::event::Event;

#[cfg(unix)]
mod unix {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::io::{self, Read};
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton,
        MouseEvent, MouseEventKind,
    };
    use termina::event as term;
    use termina::Parser;

    use super::super::keyboard::shifted_pc101_key;

    pub(crate) struct InputReader {
        tty: File,
        parser: Parser,
        prefix: Prefix,
        events: VecDeque<Event>,
        size: Option<(u16, u16)>,
        shortcuts: bool,
        paste: bool,
    }

    enum Prefix {
        None,
        Escape,
        Csi(Vec<u8>),
    }

    impl InputReader {
        pub(crate) fn new() -> io::Result<Self> {
            let tty = File::open("/dev/tty")?;
            Ok(Self {
                size: tty_size(&tty).ok(),
                tty,
                parser: Parser::default(),
                prefix: Prefix::None,
                events: VecDeque::new(),
                shortcuts: true,
                paste: false,
            })
        }

        pub(crate) fn set_shortcuts(&mut self, shortcuts: bool) {
            self.shortcuts = shortcuts;
        }

        pub(crate) fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.enqueue_resize();
            if !self.events.is_empty() {
                return Ok(true);
            }

            let timeout = if matches!(self.prefix, Prefix::Escape) {
                timeout.min(Duration::from_millis(25))
            } else {
                timeout
            };
            let mut fd = libc::pollfd {
                fd: self.tty.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let milliseconds = timeout.as_millis().min(i32::MAX as u128) as i32;
            // SAFETY: fd points to one initialized pollfd for the duration of this call.
            let ready = unsafe { libc::poll(&mut fd, 1, milliseconds) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            } else if ready > 0 {
                let mut bytes = [0_u8; 4096];
                let count = self.tty.read(&mut bytes)?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "terminal closed",
                    ));
                }
                for &byte in &bytes[..count] {
                    self.feed(byte);
                }
            } else if matches!(self.prefix, Prefix::Escape) {
                self.prefix = Prefix::None;
                self.parse(&[0x1b], false, false);
            }
            self.enqueue_resize();
            Ok(!self.events.is_empty())
        }

        pub(crate) fn read(&mut self) -> io::Result<Event> {
            self.events
                .pop_front()
                .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "no terminal event"))
        }

        fn enqueue_resize(&mut self) {
            if let Ok(size) = tty_size(&self.tty) {
                if self
                    .size
                    .replace(size)
                    .is_some_and(|previous| previous != size)
                {
                    self.events.push_back(Event::Resize(size.0, size.1));
                }
            }
        }

        fn feed(&mut self, byte: u8) {
            let prefix = std::mem::replace(&mut self.prefix, Prefix::None);
            match prefix {
                Prefix::None if byte == 0x1b => self.prefix = Prefix::Escape,
                Prefix::None => self.parse(&[byte], true, false),
                Prefix::Escape if byte == b'[' => {
                    self.prefix = Prefix::Csi(vec![0x1b, b'[']);
                }
                Prefix::Escape => self.parse(&[0x1b, byte], true, false),
                Prefix::Csi(mut sequence) => {
                    sequence.push(byte);
                    if (0x40..=0x7e).contains(&byte) {
                        if sequence == b"\x1b[200~" {
                            self.paste = true;
                        } else if sequence == b"\x1b[201~" {
                            self.paste = false;
                        }
                        let rewritten = if self.paste {
                            None
                        } else {
                            rewrite_kitty_key(&sequence, self.shortcuts)
                        };
                        if let Some(rewritten) = rewritten {
                            self.parse(rewritten.as_bytes(), true, true);
                        } else {
                            self.parse(&sequence, true, false);
                        }
                    } else if sequence.len() >= 256 {
                        self.parse(&sequence, true, false);
                    } else {
                        self.prefix = Prefix::Csi(sequence);
                    }
                }
            }
        }

        fn parse(&mut self, bytes: &[u8], maybe_more: bool, physical: bool) {
            self.parser.parse(bytes, maybe_more);
            while let Some(event) = self.parser.pop() {
                if let Some(event) = convert_event(event, physical) {
                    self.events.push_back(event);
                }
            }
        }
    }

    fn tty_size(tty: &File) -> io::Result<(u16, u16)> {
        let mut size = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: tty is an open file and size points to a writable winsize value.
        if unsafe { libc::ioctl(tty.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((size.ws_col, size.ws_row))
    }

    /// Replace only the key field of a Kitty CSI-u event. The published parser
    /// then handles the rest of the protocol, including modifiers and key state.
    fn rewrite_kitty_key(sequence: &[u8], shortcuts: bool) -> Option<String> {
        if !sequence.starts_with(b"\x1b[") || !sequence.ends_with(b"u") {
            return None;
        }
        let body = std::str::from_utf8(&sequence[2..sequence.len() - 1]).ok()?;
        let (keys, suffix) = body.split_once(';').unwrap_or((body, ""));
        let mut keys = keys.split(':');
        keys.next()?.parse::<u32>().ok()?;
        keys.next()?;
        let base = char::from_u32(keys.next()?.parse().ok()?)?;
        if !base.is_ascii() {
            return None;
        }
        let modifier = suffix
            .split(';')
            .next()?
            .split(':')
            .next()?
            .parse::<u16>()
            .unwrap_or(1)
            .saturating_sub(1);
        let shift = modifier & 1 != 0;
        let command_modifier = modifier & (2 | 4 | 8 | 16 | 32) != 0;
        if !shortcuts && !command_modifier {
            return None;
        }

        let mut rewritten = format!("\x1b[{}", base as u32);
        if shift {
            if let Some(shifted) = shifted_pc101_key(base) {
                rewritten.push(':');
                rewritten.push_str(&(shifted as u32).to_string());
            }
        }
        if !suffix.is_empty() {
            rewritten.push(';');
            rewritten.push_str(suffix);
        }
        rewritten.push('u');
        Some(rewritten)
    }

    fn convert_event(event: term::Event, physical: bool) -> Option<Event> {
        match event {
            term::Event::Key(key) => {
                let mut code = match key.code {
                    term::KeyCode::Char(c) => KeyCode::Char(c),
                    term::KeyCode::Enter => KeyCode::Enter,
                    term::KeyCode::Backspace => KeyCode::Backspace,
                    term::KeyCode::Tab => KeyCode::Tab,
                    term::KeyCode::Escape => KeyCode::Esc,
                    term::KeyCode::Left => KeyCode::Left,
                    term::KeyCode::Right => KeyCode::Right,
                    term::KeyCode::Up => KeyCode::Up,
                    term::KeyCode::Down => KeyCode::Down,
                    term::KeyCode::Home => KeyCode::Home,
                    term::KeyCode::End => KeyCode::End,
                    term::KeyCode::BackTab => KeyCode::BackTab,
                    term::KeyCode::PageUp => KeyCode::PageUp,
                    term::KeyCode::PageDown => KeyCode::PageDown,
                    term::KeyCode::Insert => KeyCode::Insert,
                    term::KeyCode::Delete => KeyCode::Delete,
                    term::KeyCode::KeypadBegin => KeyCode::KeypadBegin,
                    term::KeyCode::CapsLock => KeyCode::CapsLock,
                    term::KeyCode::ScrollLock => KeyCode::ScrollLock,
                    term::KeyCode::NumLock => KeyCode::NumLock,
                    term::KeyCode::PrintScreen => KeyCode::PrintScreen,
                    term::KeyCode::Pause => KeyCode::Pause,
                    term::KeyCode::Menu => KeyCode::Menu,
                    term::KeyCode::Null => KeyCode::Null,
                    term::KeyCode::Function(n) => KeyCode::F(n),
                    term::KeyCode::Modifier(_) | term::KeyCode::Media(_) => return None,
                };
                let modifiers = convert_modifiers(key.modifiers);
                let mut state = KeyEventState::NONE;
                if key.state.contains(term::KeyEventState::KEYPAD) {
                    state |= KeyEventState::KEYPAD;
                }
                if key.state.contains(term::KeyEventState::CAPS_LOCK) {
                    state |= KeyEventState::CAPS_LOCK;
                }
                if key.state.contains(term::KeyEventState::NUM_LOCK) {
                    state |= KeyEventState::NUM_LOCK;
                }
                if physical
                    && state.contains(KeyEventState::CAPS_LOCK)
                    && !modifiers.intersects(
                        KeyModifiers::CONTROL
                            | KeyModifiers::ALT
                            | KeyModifiers::SUPER
                            | KeyModifiers::HYPER
                            | KeyModifiers::META,
                    )
                {
                    if let KeyCode::Char(c) = code {
                        if c.is_ascii_lowercase() {
                            code = KeyCode::Char(c.to_ascii_uppercase());
                        } else if c.is_ascii_uppercase() {
                            code = KeyCode::Char(c.to_ascii_lowercase());
                        }
                    }
                }
                Some(Event::Key(KeyEvent::new_with_kind_and_state(
                    code,
                    modifiers,
                    match key.kind {
                        term::KeyEventKind::Press => KeyEventKind::Press,
                        term::KeyEventKind::Repeat => KeyEventKind::Repeat,
                        term::KeyEventKind::Release => KeyEventKind::Release,
                    },
                    state,
                )))
            }
            term::Event::Mouse(mouse) => {
                let button = |button| match button {
                    term::MouseButton::Left => MouseButton::Left,
                    term::MouseButton::Right => MouseButton::Right,
                    term::MouseButton::Middle => MouseButton::Middle,
                };
                let kind = match mouse.kind {
                    term::MouseEventKind::Down(b) => MouseEventKind::Down(button(b)),
                    term::MouseEventKind::Up(b) => MouseEventKind::Up(button(b)),
                    term::MouseEventKind::Drag(b) => MouseEventKind::Drag(button(b)),
                    term::MouseEventKind::Moved => MouseEventKind::Moved,
                    term::MouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
                    term::MouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
                    term::MouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
                    term::MouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
                };
                Some(Event::Mouse(MouseEvent {
                    kind,
                    column: mouse.column,
                    row: mouse.row,
                    modifiers: convert_modifiers(mouse.modifiers),
                }))
            }
            term::Event::FocusIn => Some(Event::FocusGained),
            term::Event::FocusOut => Some(Event::FocusLost),
            term::Event::Paste(text) => Some(Event::Paste(text)),
            _ => None,
        }
    }

    fn convert_modifiers(modifiers: term::Modifiers) -> KeyModifiers {
        let mut converted = KeyModifiers::NONE;
        for (source, target) in [
            (term::Modifiers::SHIFT, KeyModifiers::SHIFT),
            (term::Modifiers::ALT, KeyModifiers::ALT),
            (term::Modifiers::CONTROL, KeyModifiers::CONTROL),
            (term::Modifiers::SUPER, KeyModifiers::SUPER),
            (term::Modifiers::HYPER, KeyModifiers::HYPER),
            (term::Modifiers::META, KeyModifiers::META),
        ] {
            if modifiers.contains(source) {
                converted |= target;
            }
        }
        converted
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn keys(bytes: &[u8], shortcuts: bool) -> Vec<Event> {
            let mut input = InputReader {
                tty: File::open("/dev/null").unwrap(),
                parser: Parser::default(),
                prefix: Prefix::None,
                events: VecDeque::new(),
                size: None,
                shortcuts,
                paste: false,
            };
            for &byte in bytes {
                input.feed(byte);
            }
            input.events.into_iter().collect()
        }

        #[test]
        fn russian_hunks_and_stacked_commit_keys() {
            let next = keys(b"\x1b[1066::93;2u", true);
            assert!(matches!(&next[..], [Event::Key(k)] if k.code == KeyCode::Char('}')));
            let previous = keys(b"\x1b[1061::91;2u", true);
            assert!(matches!(&previous[..], [Event::Key(k)] if k.code == KeyCode::Char('{')));
            let commit = keys(b"\x1b[1076::108;5u", true);
            assert!(
                matches!(&commit[..], [Event::Key(k)] if k.code == KeyCode::Char('l') && k.modifiers.contains(KeyModifiers::CONTROL))
            );
        }

        #[test]
        fn caps_lock_and_shift_caps_lock_preserve_command_case() {
            let caps = keys(b"\x1b[1087::103;65u", true);
            assert!(matches!(&caps[..], [Event::Key(k)] if k.code == KeyCode::Char('G')));
            let shift_caps = keys(b"\x1b[1087:1055:103;66u", true);
            assert!(matches!(&shift_caps[..], [Event::Key(k)] if k.code == KeyCode::Char('g')));
        }

        #[test]
        fn text_mode_keeps_typed_characters_and_physical_control_keys() {
            let text = keys("д".as_bytes(), false);
            assert!(matches!(&text[..], [Event::Key(k)] if k.code == KeyCode::Char('д')));
            let control = keys(b"\x1b[1074::119;5u", false);
            assert!(
                matches!(&control[..], [Event::Key(k)] if k.code == KeyCode::Char('w') && k.modifiers.contains(KeyModifiers::CONTROL))
            );
        }

        #[test]
        fn legacy_navigation_and_mouse_events_still_reach_the_viewer() {
            let events = keys(b"j\x1b[A\x1b[<64;10;5M", true);
            assert!(
                matches!(&events[..], [Event::Key(j), Event::Key(up), Event::Mouse(scroll)]
                if j.code == KeyCode::Char('j')
                    && up.code == KeyCode::Up
                    && scroll.kind == MouseEventKind::ScrollUp)
            );
        }

        #[test]
        fn paste_keeps_key_looking_text_verbatim() {
            let events = keys(b"\x1b[200~\x1b[1066::93;2u\x1b[201~", true);
            assert!(matches!(&events[..], [Event::Paste(text)] if text == "\x1b[1066::93;2u"));
        }
    }
}

#[cfg(unix)]
pub(super) use unix::InputReader;

#[cfg(not(unix))]
pub(super) struct InputReader;

#[cfg(not(unix))]
impl InputReader {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self)
    }

    pub(super) fn set_shortcuts(&mut self, _shortcuts: bool) {}

    pub(super) fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        crossterm::event::poll(timeout)
    }

    pub(super) fn read(&mut self) -> io::Result<Event> {
        crossterm::event::read()
    }
}
