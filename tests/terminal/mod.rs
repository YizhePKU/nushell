//! This module contains tests that requires a terminal emulator.

use log::LevelFilter;
use nu_test_support::terminal::SimpleTerminal;
use simplelog::{Config, SimpleLogger};

#[test]
fn command_hints_are_pwd_aware() {
    let foo = tempfile::tempdir().unwrap();
    let bar = tempfile::tempdir().unwrap();
    let cd_to_foo = format!("cd {}\r", foo.path().display());
    let cd_to_bar = format!("cd {}\r", bar.path().display());

    let terminal = SimpleTerminal::nu(|term| {
        SimpleLogger::init(LevelFilter::Debug, Config::default()).unwrap();

        term.writer.write_all(b"echo AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\r").unwrap();
        // term.writer.write_all(cd_to_foo.as_bytes()).unwrap();
        // term.writer.write_all(b"print 'FOO'\r").unwrap();
        // term.writer.write_all(cd_to_bar.as_bytes()).unwrap();
        // term.writer.write_all(b"print 'BAR'\r").unwrap();
        // term.writer.write_all(cd_to_foo.as_bytes()).unwrap();
        // term.writer.write_all(b"print").unwrap();
    });

    print!("{}", &terminal);

    let last_line: String = terminal.buffer[terminal.cursor.0].into_iter().collect();
    dbg!(last_line);
}
