use std::path::Path;

use dinopod::config::{validate_setup_command, DinopodConfig};

#[test]
fn default_config_should_match_mvp_defaults() {
    let config = DinopodConfig::default();

    assert_eq!(config.app.service, "app");
    assert_eq!(config.app.internal_port, 3000);
    assert_eq!(config.runtime, None);
    assert_eq!(config.native.dev_script, None);
    assert_eq!(config.native.app_port, None);
    assert_eq!(config.app.compose_file, Path::new("docker-compose.yml"));
    assert_eq!(config.app.default_branch, "main");
    assert_eq!(config.worktree.root, Path::new("../.dinopod-worktrees"));
    assert_eq!(config.proxy.host_suffix, "localhost");
    assert_eq!(config.proxy.network, "dinopod-proxy");
    assert_eq!(config.proxy.container_name, "dinopod-traefik");
    assert_eq!(config.proxy.http_port, 80);
    assert!(config.settings.copy_env);
    assert!(config.setup.commands.is_empty());
}

#[test]
fn config_file_should_load_setup_commands_array() {
    let config = DinopodConfig::from_toml_str(
        r#"
        [setup]
        commands = ["pnpm db:migrate", "pnpm db:seed"]
        "#,
    )
    .expect("setup commands should load");

    assert_eq!(config.setup.commands.len(), 2);
    assert_eq!(config.setup.commands[0], "pnpm db:migrate");
}

#[test]
fn config_file_should_reject_docker_compose_in_setup() {
    let error = DinopodConfig::from_toml_str(
        r#"
        [setup]
        commands = ["docker compose up -d"]
        "#,
    )
    .expect_err("docker compose in setup should fail");

    assert!(error.to_string().contains("docker compose"));
}

#[test]
fn validate_setup_command_should_reject_compose_invocation() {
    let error = validate_setup_command("docker-compose up").expect_err("should reject");
    assert!(error.to_string().contains("docker compose"));
}

#[test]
fn config_file_should_override_defaults_without_requiring_every_field() {
    let config = DinopodConfig::from_toml_str(
        r#"
        [app]
        service = "web"
        internal_port = 8080

        [proxy]
        host_suffix = "test.localhost"
        "#,
    )
    .expect("partial config should load");

    assert_eq!(config.app.service, "web");
    assert_eq!(config.app.internal_port, 8080);
    assert_eq!(config.app.compose_file, Path::new("docker-compose.yml"));
    assert_eq!(config.proxy.host_suffix, "test.localhost");
    assert_eq!(config.proxy.network, "dinopod-proxy");
}

#[test]
fn starter_config_should_not_include_comments() {
    let rendered = dinopod::config::render_starter_config(&DinopodConfig::default());

    assert!(!rendered.contains('#'));
    assert!(rendered.starts_with("[compose]\n"));
    assert!(rendered.contains("[setup]\ncommands = []\n"));
}

#[test]
fn config_file_should_reject_unknown_keys() {
    let error = DinopodConfig::from_toml_str(
        r"
        [app]
        httpport = 18080
        ",
    )
    .expect_err("unknown keys should be rejected");

    assert!(
        error.to_string().contains("unknown field"),
        "expected unknown field error, got {error}"
    );
}

#[test]
fn config_file_should_load_runtime_and_native_overrides() {
    let config = DinopodConfig::from_toml_str(
        r#"
        runtime = "native"

        [native]
        dev_script = "start:dev"
        app_port = 4000
        "#,
    )
    .expect("native config should load");

    assert_eq!(config.runtime, Some(dinopod::config::RuntimeMode::Native));
    assert_eq!(config.native.dev_script.as_deref(), Some("start:dev"));
    assert_eq!(config.native.app_port, Some(4000));
}

#[test]
fn config_file_should_reject_invalid_runtime_values() {
    let error = DinopodConfig::from_toml_str("runtime = \"docker\"")
        .expect_err("invalid runtime should fail");

    assert!(
        error.to_string().contains("invalid runtime mode"),
        "expected invalid runtime error, got {error}"
    );
}
