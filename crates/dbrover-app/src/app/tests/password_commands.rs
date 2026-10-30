//! A data source whose password comes from a command (W9), set in the dialog, then trusted at
//! the prompt when it first connects. The engine runs `ScriptedPrograms`, whose `fail` exits
//! with status 3, so the connection stops at the password without reaching any server; the
//! engine's own tests connect to PostgreSQL with a real command.

use super::*;

#[test]
fn sets_a_password_command_then_asks_once_before_running_it() {
    let dir = tempfile::tempdir().unwrap();
    let (_layer, feed) = log_console::pipeline();
    let mut harness = start_app(dir.path(), feed);
    step_until_visible(&mut harness, "No data sources yet.");
    harness
        .get_all_by_label("New data source")
        .next()
        .unwrap()
        .click();
    harness.run();
    harness.get_by_label("PostgreSQL").click();
    harness.run();

    assert!(harness.query_by_label("Password command").is_none());
    harness.get_by_label("Password from a command").click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(Role::PasswordInput, "Password")
            .is_none(),
        "the command replaces the password"
    );
    type_into(
        &mut harness,
        Role::MultilineTextInput,
        "Password command",
        "fail",
    );
    harness.get_by_label("Save").click();

    step_until_visible(&mut harness, "Expand postgres@localhost");
    let saved = DataSourceFile::new(dir.path().join("data_sources.toml"))
        .load()
        .unwrap();
    let [saved] = saved.as_slice() else {
        panic!("expected one data source, got {saved:?}");
    };
    let command = saved
        .config
        .server()
        .and_then(|server| server.password_command.clone())
        .unwrap();
    assert_eq!(command.source(), ["fail"]);

    harness.get_by_label("Expand postgres@localhost").click();
    step_until_visible(&mut harness, "Run this command for a password?");
    // The modal sizes itself over its first frames; a click before then misses.
    harness.run();
    let shown: Vec<String> = harness
        .query_all_by_role(Role::MultilineTextInput)
        .filter_map(|input| input.value())
        .collect();
    assert!(shown.contains(&"fail".to_owned()), "{shown:?}");
    harness.get_by_label("Run and connect").click();

    let id = saved.id;
    step_until(&mut harness, "the command failing", |harness| {
        harness.state().state.data_source(id).is_some_and(|entry| {
            matches!(&entry.session, dbrover_engine::SessionState::Failed(error)
                    if error.ends_with("(exit status: 3): it went wrong"))
        })
    });
    assert!(
        harness
            .query_by_label("Run this command for a password?")
            .is_none()
    );
    let trusted = TrustedCommandsFile::new(dir.path().join("trusted_commands.toml"))
        .load()
        .unwrap();
    assert!(trusted.contains(&dbrover_store::TrustedCommand::password(id, &command)));
}
