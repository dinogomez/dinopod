use std::io;

use dinopod::ui::{BufferedUi, Ui};

fn report_environment_ready<U>(ui: &mut U) -> io::Result<()>
where
    U: Ui,
{
    ui.status("worktree: /repo/.dinopod-worktrees/myapp-jira-123")?;
    ui.success("url: http://jira-123-myapp.localhost")
}

#[test]
fn successful_messages_should_go_through_output_boundary() {
    let mut ui = BufferedUi::default();

    report_environment_ready(&mut ui).expect("buffered output should not fail");

    assert_eq!(
        ui.stdout(),
        [
            "worktree: /repo/.dinopod-worktrees/myapp-jira-123".to_owned(),
            "url: http://jira-123-myapp.localhost".to_owned(),
        ]
    );
    assert!(ui.stderr().is_empty());
}

#[test]
fn error_messages_should_go_to_stderr_boundary() {
    let mut ui = BufferedUi::default();

    ui.error("missing required dependency: docker")
        .expect("buffered output should not fail");

    assert!(ui.stdout().is_empty());
    assert_eq!(
        ui.stderr(),
        ["missing required dependency: docker".to_owned()]
    );
}
