use std::process::Command;

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
