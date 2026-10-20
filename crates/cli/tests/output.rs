//! Where output lands, end to end: forwarded child output on the stream the
//! child wrote it to, and spinners that draw on a terminal without clobbering
//! the lines printed around them.
//!
//! `dc up` with an `initializeCommand` runs commands on the host, under
//! spinners, before it needs docker; a failing one stops it there.

#![cfg(unix)]

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};
use tempfile::TempDir;

const ROWS: u16 = 50;
const COLS: u16 = 200;

/// A project whose `initializeCommand` is `initialize`, and a home to keep
/// everything we write out of the real one.
fn project(initialize: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".devcontainer")).unwrap();
    std::fs::write(
        repo.join(".devcontainer/devcontainer.json"),
        format!(r#"{{ "image": "alpine", "initializeCommand": {initialize} }}"#),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!("[projects.demo]\npath = {:?}\n", repo.display().to_string()),
    )
    .unwrap();
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    dir
}

fn dc_up(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_devconcurrent"));
    cmd.arg("up")
        .current_dir(dir.join("repo"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", dir)
        .env("DEVCONCURRENT_CONFIG", dir)
        .env("TERM", "xterm-256color");
    cmd
}

fn lines(bytes: &[u8]) -> Vec<String> {
    let mut lines: Vec<_> = String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

fn piped(initialize: &str) -> Output {
    let dir = project(initialize);
    let out = dc_up(dir.path()).output().unwrap();
    assert!(!out.status.success(), "the initializeCommand fails");
    out
}

/// Run `dc up` with stdout and stderr on a terminal, and return everything
/// written to it.
fn on_a_terminal(initialize: &str) -> Vec<u8> {
    let dir = project(initialize);

    let controller = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
    grantpt(&controller).unwrap();
    unlockpt(&controller).unwrap();
    let name = ptsname(&controller, Vec::new()).unwrap();
    let terminal = OpenOptions::new()
        .read(true)
        .write(true)
        .open(name.to_str().unwrap())
        .unwrap();
    tcsetwinsize(
        &terminal,
        Winsize {
            ws_row: ROWS,
            ws_col: COLS,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();

    let mut child = {
        let mut cmd = dc_up(dir.path());
        cmd.stdin(Stdio::null())
            .stdout(terminal.try_clone().unwrap())
            .stderr(terminal);
        cmd.spawn().unwrap()
        // Dropping `cmd` closes our copies of the terminal, so reads end once
        // the child's do.
    };

    let mut controller = File::from(controller);
    let mut output = Vec::new();
    let mut buf = [0; 4096];
    loop {
        match controller.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => output.extend_from_slice(&buf[..n]),
            // Linux reports the terminal's last close as EIO.
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => break,
            Err(e) => panic!("reading the terminal: {e}"),
        }
    }
    assert!(
        !child.wait().unwrap().success(),
        "the initializeCommand fails"
    );
    output
}

/// The terminal as a person would see it once `dc up` exits.
fn screen(output: &[u8]) -> Vec<String> {
    let mut parser = vt100::Parser::new(ROWS, COLS, 0);
    parser.process(output);
    parser
        .screen()
        .contents()
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect()
}

#[test]
fn child_output_keeps_its_stream() {
    let out = piped(r#""echo out; echo err >&2; exit 1""#);

    assert_eq!(lines(&out.stdout), ["out"]);
    let stderr = lines(&out.stderr);
    assert!(stderr.iter().any(|l| l == "err"), "{stderr:#?}");
    assert!(!stderr.iter().any(|l| l == "out"), "{stderr:#?}");
}

#[test]
fn parallel_child_output_names_its_command() {
    let out = piped(r#"{ "a": "echo a-out; echo a-err >&2", "b": "echo b-out; exit 1" }"#);

    assert_eq!(lines(&out.stdout), ["[a] a-out", "[b] b-out"]);
    let stderr = lines(&out.stderr);
    assert!(stderr.iter().any(|l| l == "[a] a-err"), "{stderr:#?}");
}

#[test]
fn piped_output_has_no_escape_codes() {
    let out = piped(r#"{ "a": "echo a-out", "b": "exit 1" }"#);

    assert!(!out.stdout.contains(&0x1b));
    assert!(!out.stderr.contains(&0x1b));
}

#[test]
fn spinners_draw_on_a_terminal_and_clear_when_done() {
    // Long enough for the spinners to tick while output arrives.
    let output = on_a_terminal(
        r#"{ "a": "sleep 0.3; echo a-out; echo a-err >&2; sleep 0.3",
             "b": "sleep 0.3; echo b-out; sleep 0.3; exit 1" }"#,
    );
    // indicatif's default spinner frames are braille.
    let spinner = |line: &str| line.chars().any(|c| ('\u{2800}'..='\u{28ff}').contains(&c));

    let raw = String::from_utf8_lossy(&output);
    assert!(spinner(&raw), "no spinner was drawn:\n{raw}");

    let screen = screen(&output);
    assert!(!screen.iter().any(|l| spinner(l)), "{screen:#?}");
    for expected in ["[a] a-out", "[a] a-err", "[b] b-out"] {
        assert!(
            screen.iter().any(|l| l == expected),
            "{expected:?}: {screen:#?}"
        );
    }
    for expected in ["[a] Took ", "[b] Failed after ", "[up] Failed after "] {
        assert!(
            screen.iter().any(|l| l.contains(expected)),
            "{expected:?}: {screen:#?}"
        );
    }
}
