//! Keyboard input of the wizard: the production stdin stream behind a testable seam.

use std::io;

/// Keyboard input of the wizard. Production reads the stdin byte stream; tests script the
/// answers, so the question flow can run without a terminal.
pub(super) trait Input {
    fn byte(&mut self) -> io::Result<u8>;
    fn readable_within(&mut self, ms: i32) -> io::Result<bool>;
}

/// The stdin keyboard.
pub(super) struct StdinInput;

impl Input for StdinInput {
    fn byte(&mut self) -> io::Result<u8> {
        read_byte()
    }

    fn readable_within(&mut self, ms: i32) -> io::Result<bool> {
        readable_within(ms)
    }
}

/// Read one key and normalize it: `q`, Ctrl-C (ISIG is cleared in raw mode, so 0x03 arrives
/// as a byte) and Esc all count as `q` (abort and exit), and EOF counts as `q` too — otherwise
/// the caller would receive `None`, fall into `_ => {}` and redraw forever. Arrow keys and other
/// escape sequences are consumed and ignored: a direction key is not an answer and must not quit
/// the wizard. Other bytes are returned as is; their meaning (`r`/`y`/`n`/digits) is up to the
/// caller.
pub(super) fn key(input: &mut dyn Input) -> io::Result<u8> {
    loop {
        let b = input.byte()?;
        if b != 0x1b {
            return Ok(normalize_key(b));
        }
        // Esc alone aborts. A sequence (arrow keys, Home, End, Delete, ...) follows within a few
        // milliseconds; wait for that follow-up byte instead of treating the Esc as final.
        if !input.readable_within(20)? {
            return Ok(b'q');
        }
        let next = input.byte()?;
        if next == b'[' || next == b'O' {
            // Consume the parameter, intermediate and final bytes of the CSI or SS3 sequence.
            loop {
                let c = input.byte()?;
                if is_escape_final(c) {
                    break;
                }
            }
            continue;
        }
        return Ok(b'q');
    }
}

/// Read one byte from the terminal. EOF maps to `q` so callers cannot loop on `None`.
fn read_byte() -> io::Result<u8> {
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(libc::STDIN_FILENO, b.as_mut_ptr().cast(), 1) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Ok(b'q');
        }
        return Ok(b[0]);
    }
}

/// Ctrl-C and a lone Esc abort; every other byte is an answer.
pub(super) fn normalize_key(byte: u8) -> u8 {
    match byte {
        0x03 | 0x1b => b'q',
        other => other,
    }
}

/// Final byte of a CSI/SS3 sequence (0x40..=0x7e). Parameter and intermediate bytes are below
/// that range, so this finds the end of arrow-key and similar sequences.
pub(super) fn is_escape_final(byte: u8) -> bool {
    (0x40..=0x7e).contains(&byte)
}

/// Whether stdin has a byte within `ms` milliseconds. `false` also covers the lone-Esc case when
/// the terminal sent nothing else.
fn readable_within(ms: i32) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        return Ok(n > 0);
    }
}
