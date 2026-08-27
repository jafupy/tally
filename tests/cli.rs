use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn missing_path_fails_without_a_summary() {
    let output = Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("/definitely/not/a/tally/input")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn counts_stdin_with_dash_path() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tally"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"one\n\ntwo\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Stdin"));
    assert!(stdout.contains("Total"));
}
