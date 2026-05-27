use std::path::Path;

use dinopod::config::{ConfigOverrides, DinopodConfig};

#[test]
fn default_config_should_match_mvp_defaults() {
    let config = DinopodConfig::default();

    assert_eq!(config.app.service, "app");
    assert_eq!(config.app.internal_port, 3000);
    assert_eq!(config.app.compose_file, Path::new("docker-compose.yml"));
    assert_eq!(config.app.default_branch, "main");
    assert_eq!(config.worktree.root, Path::new("../.dinopod-worktrees"));
    assert_eq!(config.proxy.host_suffix, "localhost");
    assert_eq!(config.proxy.network, "dinopod-proxy");
    assert_eq!(config.proxy.container_name, "dinopod-traefik");
    assert_eq!(config.proxy.http_port, 80);
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
fn cli_overrides_should_win_over_file_values() {
    let config = DinopodConfig::from_toml_str(
        r#"
        [app]
        service = "web"
        internal_port = 8080
        "#,
    )
    .expect("config should load")
    .with_overrides(ConfigOverrides {
        app_service: Some("api".to_owned()),
        internal_port: Some(9000),
        ..ConfigOverrides::default()
    });

    assert_eq!(config.app.service, "api");
    assert_eq!(config.app.internal_port, 9000);
}
