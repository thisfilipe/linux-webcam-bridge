use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jieli-usb-reset"))
}

#[test]
fn rejects_extra_arguments_before_touching_the_device() {
    let output = binary().arg("8/15").output().expect("run helper");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "This helper accepts no arguments\n"
    );
}

#[test]
fn rejects_unprivileged_execution_before_touching_the_device() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let output = binary().output().expect("run helper");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "This installed helper requires its narrowly scoped sudo rule\n"
    );
}
