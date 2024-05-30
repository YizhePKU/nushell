use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};
use std::{fmt::Display, io::Write, sync::mpsc, time::Duration};

const WIDTH: usize = 80;
const HEIGHT: usize = 24;

/// A simple terminal emulator for testing purposes. It implements
/// `vte::Perform`, so you can connect it to the parser directly. It doesn't
/// support colors or scrollback. Cursor movement operates on Unicode Scalar
/// Values. The window size is fixed to 80x24.
///
/// The following ANSI codes are supported:
/// * 0x0A               (Line Feed)
/// * 0x0D               (Carriage Return)
/// * ESC 7              (Save Cursor)
/// * ESC 8              (Restore Cursor)
/// * CSI $x    A/B/C/D  (Cursor Up/Down/Forward/Backward)
/// * CSI $x;$y H        (Cursor Position)
/// * CSI $x    J        (Erase in Display)
/// * CSI $x    K        (Erase in Line)
/// * CSI $x    n        (Device Status Report)
pub struct SimpleTerminal {
    pub cursor: (usize, usize),
    pub saved_cursor: (usize, usize),
    pub buffer: [[char; WIDTH]; HEIGHT],
    pub writer: Box<dyn Write + Send>,
}

impl SimpleTerminal {
    pub fn new(writer: Box<dyn Write + Send>) -> Self {
        Self {
            cursor: (0, 0),
            saved_cursor: (0, 0),
            buffer: [[' '; WIDTH]; HEIGHT],
            writer,
        }
    }

    /// Create a SimpleTerminal and connect it to an instance of Nushell. Within
    /// `func`, you can use `self.writer` to send keystrokes to Nushell, which
    /// will appear to Nushell as if they were typed in a terminal. Returns the
    /// final state of the terminal.
    ///
    /// The Nushell process will be killed after 500ms of inactivity. This is
    /// necessary because we have no way of knowing whether Nushell has finished
    /// writing data to the terminal.
    ///
    /// Hint: If you want to press the Enter key, you should send `\r` (NOT
    /// `\n`) regardless of the platform.
    pub fn nu(func: impl FnOnce(&mut Self)) -> SimpleTerminal {
        // Open a PTY pair.
        let PtyPair { slave, master } = native_pty_system()
            .openpty(PtySize {
                rows: HEIGHT as u16,
                cols: WIDTH as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        // Spawn Nushell to the slave end of the PTY.
        let mut cmd = CommandBuilder::new(crate::fs::executable_path());
        cmd.arg("--no-config-file");
        let mut child = slave.spawn_command(cmd).unwrap();

        let mut reader = master.try_clone_reader().unwrap();
        let writer = master.take_writer().unwrap();

        // Create a thread that reads from the master end of the PTY.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || loop {
            let mut buf = [0; 200];
            let n = reader.read(&mut buf).unwrap();
            let _ = tx.send(buf[..n].to_vec());
        });

        let mut parser = vte::Parser::new();
        let mut terminal = SimpleTerminal::new(writer);

        // Wait for Nushell to initialize.
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(buf) => {
                    for c in buf {
                        parser.advance(&mut terminal, c);
                    }
                }
                Err(_) => break,
            }
        }

        func(&mut terminal);

        // Wait for Nushell to respond.
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(buf) => {
                    for c in buf {
                        parser.advance(&mut terminal, c);
                    }
                }
                Err(_) => break,
            }
        }

        // Kill the Nushell process.
        child.kill().unwrap();

        terminal
    }
}

impl Display for SimpleTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for line in self.buffer {
            let text: String = line.iter().collect();
            writeln!(f, "{}", text)?;
        }
        Ok(())
    }
}

impl vte::Perform for SimpleTerminal {
    fn print(&mut self, c: char) {
        log::debug!("SimpleTerminal/print: {}, cursor = {:?}", c, self.cursor);

        self.buffer[self.cursor.0][self.cursor.1] = c;

        if self.cursor.1 + 1 < WIDTH {
            self.cursor.1 += 1;
        } else {
            self.cursor.1 = 0;
            if self.cursor.0 + 1 < HEIGHT {
                self.cursor.0 += 1;
            } else {
                // The screen is full. Shift everything up one line.
                self.buffer.rotate_left(1);
                self.buffer.last_mut().unwrap().fill(' ');
            }
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        log::debug!(
            "SimpleTerminal/csi_dispatch: params = {:X?}, intermediates = {:X?}, ignore = {}, action = {:X?}, cursor = {:?}",
            params, intermediates, ignore, action, self.cursor
        );

        // Handle Cursor Up.
        if action == 'A' {
            let n = params.into_iter().next().unwrap_or(&[1])[0] as usize;
            self.cursor.0 = self.cursor.0.saturating_sub(n);
        }
        // Handle Cursor Down.
        if action == 'B' {
            let n = params.into_iter().next().unwrap_or(&[1])[0] as usize;
            self.cursor.0 = self.cursor.0.saturating_add(n);
            if self.cursor.0 >= HEIGHT {
                self.cursor.0 = HEIGHT - 1;
            }
        }
        // Handle Cursor Forward.
        if action == 'C' {
            let n = params.into_iter().next().unwrap_or(&[1])[0] as usize;
            self.cursor.1 = self.cursor.1.saturating_add(n);
            if self.cursor.1 >= WIDTH {
                self.cursor.1 = WIDTH - 1;
            }
        }
        // Handle Cursor Backward.
        if action == 'D' {
            let n = params.into_iter().next().unwrap_or(&[1])[0] as usize;
            self.cursor.1 = self.cursor.1.saturating_sub(n);
        }
        // Handle Cursor Position.
        if action == 'H' {
            let mut iter = params.into_iter();
            let n = iter.next().unwrap_or(&[1])[0] as usize;
            let m = iter.next().unwrap_or(&[1])[0] as usize;
            // As a special case, if n == 0, reset the cursor position.
            if n == 0 {
                self.cursor = (0, 0);
            }
            if n > 0 && n <= HEIGHT && m > 0 && m <= WIDTH {
                self.cursor.0 = n - 1;
                self.cursor.1 = m - 1;
            }
        }
        // Handle Erase in Display.
        if action == 'J' {
            let n = params.into_iter().next().unwrap_or(&[0])[0];
            // Handle Erase Below (default).
            if n == 0 {
                self.buffer[self.cursor.0][self.cursor.1..].fill(' ');
                for i in self.cursor.0 + 1..HEIGHT {
                    self.buffer[i].fill(' ');
                }
            }
            // Handle Erase Above.
            if n == 1 {
                self.buffer[self.cursor.0][..=self.cursor.1].fill(' ');
                for i in 0..self.cursor.0 {
                    self.buffer[i].fill(' ');
                }
            }
            // Handle Erase All.
            if n == 2 {
                for i in 0..HEIGHT {
                    self.buffer[i].fill(' ');
                }
            }
        }
        // Handle Erase in Line.
        if action == 'K' {
            let n = params.into_iter().next().unwrap_or(&[0])[0];
            // Handle Erase to Right (default).
            if n == 0 {
                self.buffer[self.cursor.0][self.cursor.1..].fill(' ');
            }
            // Handle Erase to Left.
            if n == 1 {
                self.buffer[self.cursor.0][..=self.cursor.1].fill(' ');
            }
            // Handle Erase All.
            if n == 2 {
                self.buffer[self.cursor.0].fill(' ');
            }
        }
        // Handle Device Status Report.
        if action == 'n' {
            let n = params.into_iter().next().unwrap()[0];
            // Handle Status Report.
            if n == 5 {
                self.writer.write_all(b"\x1b[0n").unwrap();
            }
            // Handle Report Cursor Position.
            if n == 6 {
                let msg = format!("\x1b[{};{}R", self.cursor.0 + 1, self.cursor.1 + 1);
                self.writer.write_all(msg.as_bytes()).unwrap();
            }
        }
    }

    fn execute(&mut self, byte: u8) {
        log::debug!(
            "SimpleTerminal/execute: {:X?}, cursor = {:?}",
            byte,
            self.cursor
        );

        // Handle Line Feed.
        if byte == 0x0A && self.cursor.0 + 1 < HEIGHT {
            self.cursor.0 += 1;
        }
        // Handle Carriage Return.
        if byte == 0x0D {
            self.cursor.1 = 0;
        }
    }

    fn hook(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        log::debug!(
            "SimpleTerminal/hook: params = {:X?}, intermediates = {:X?}, ignore = {}, action = {:X?}, cursor = {:?}",
            params, intermediates, ignore, action, self.cursor
        );
    }

    fn put(&mut self, byte: u8) {
        log::debug!(
            "SimpleTerminal/put: {:X?}, cursor = {:?}",
            byte,
            self.cursor,
        );
    }

    fn unhook(&mut self) {
        log::debug!("SimpleTerminal/unhook, cursor = {:?}", self.cursor);
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        log::debug!(
            "SimpleTerminal/osc_dispatch: params = {:X?}, bell_terminated = {}, cursor = {:?}",
            params,
            bell_terminated,
            self.cursor,
        );
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        log::debug!(
            "SimpleTerminal/esc_dispatch: intermediates = {:X?}, ignore = {}, byte = {:X?}, cursor = {:?}",
            intermediates,
            ignore,
            byte,
            self.cursor,
        );

        // Handle Save Cursor.
        if byte == b'7' {
            self.saved_cursor = self.cursor;
        }
        // Handle Restore Cursor.
        if byte == b'8' {
            self.cursor = self.saved_cursor;
        }
    }
}

#[cfg(test)]
mod test {
    use super::SimpleTerminal;

    fn emulate(input: &str) -> SimpleTerminal {
        let mut terminal = SimpleTerminal::new(Box::new(vec![]));
        let mut parser = vte::Parser::new();
        for byte in input.as_bytes() {
            parser.advance(&mut terminal, *byte);
        }
        terminal
    }

    #[test]
    fn basic_cursor_movement() {
        let terminal = emulate("\x1b[10C\x1b[10B\x1b[5A\x1b[5D");
        assert_eq!(terminal.cursor, (5, 5));
    }

    #[test]
    fn overflowing_cursor_movement() {
        let terminal = emulate("\x1b[100C\x1b[100B");
        assert_eq!(terminal.cursor, (23, 79));

        let terminal = emulate("\x1b[10C\x1b[10B\x1b[100A\x1b[100D");
        assert_eq!(terminal.cursor, (0, 0));
    }

    #[test]
    fn print_at_cursor_position() {
        let terminal = emulate("\x1b[20;30Hfoo");
        let text: String = terminal.buffer[19][29..32].iter().collect();
        assert_eq!(text, "foo");
        assert_eq!(terminal.cursor, (19, 32));
    }

    #[test]
    fn print_with_line_feed_and_carriage_return() {
        let terminal = emulate("AAA\r\nAAA\r\nAAA\r\n");
        assert_eq!(terminal.cursor, (3, 0));
        assert_eq!(&terminal.buffer[0][..3], &['A', 'A', 'A']);
        assert_eq!(&terminal.buffer[1][..3], &['A', 'A', 'A']);
        assert_eq!(&terminal.buffer[2][..3], &['A', 'A', 'A']);
    }

    #[test]
    fn erase_in_display() {
        let terminal = emulate("AAA\r\nAAA\r\nAAA\x1b[2;2H\x1b[J");
        assert_eq!(&terminal.buffer[0][..3], &['A', 'A', 'A']);
        assert_eq!(&terminal.buffer[1][..3], &['A', ' ', ' ']);
        assert_eq!(&terminal.buffer[2][..3], &[' ', ' ', ' ']);

        let terminal = emulate("AAA\r\nAAA\r\nAAA\x1b[2;2H\x1b[1J");
        assert_eq!(&terminal.buffer[0][..3], &[' ', ' ', ' ']);
        assert_eq!(&terminal.buffer[1][..3], &[' ', ' ', 'A']);
        assert_eq!(&terminal.buffer[2][..3], &['A', 'A', 'A']);

        let terminal = emulate("AAA\r\nAAA\r\nAAA\x1b[2;2H\x1b[2J");
        assert_eq!(&terminal.buffer[0][..3], &[' ', ' ', ' ']);
        assert_eq!(&terminal.buffer[1][..3], &[' ', ' ', ' ']);
        assert_eq!(&terminal.buffer[2][..3], &[' ', ' ', ' ']);
    }

    #[test]
    fn erase_in_line() {
        let terminal = emulate("AAA\r\nAAA\r\nAAA\x1b[2;2H\x1b[K");
        assert_eq!(&terminal.buffer[0][..3], &['A', 'A', 'A']);
        assert_eq!(&terminal.buffer[1][..3], &['A', ' ', ' ']);
        assert_eq!(&terminal.buffer[2][..3], &['A', 'A', 'A']);

        let terminal = emulate("AAA\r\nAAA\r\nAAA\x1b[2;2H\x1b[1K");
        assert_eq!(&terminal.buffer[0][..3], &['A', 'A', 'A']);
        assert_eq!(&terminal.buffer[1][..3], &[' ', ' ', 'A']);
        assert_eq!(&terminal.buffer[2][..3], &['A', 'A', 'A']);

        let terminal = emulate("AAA\r\nAAA\r\nAAA\x1b[2;2H\x1b[2K");
        assert_eq!(&terminal.buffer[0][..3], &['A', 'A', 'A']);
        assert_eq!(&terminal.buffer[1][..3], &[' ', ' ', ' ']);
        assert_eq!(&terminal.buffer[2][..3], &['A', 'A', 'A']);
    }

    #[test]
    fn save_restore_cursor() {
        let terminal = emulate("\x1b[10;10H\x1b7\x1b[0H\x1b8");
        assert_eq!(terminal.cursor, (9, 9));
    }
}
