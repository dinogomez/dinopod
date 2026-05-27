use std::any::TypeId;
use std::path::Path;

use dinopod::config::DinopodConfig;
use dinopod::domain::{HostName, NetworkAlias, ProjectName, WorktreePath};
use dinopod::names::{derive_names, normalize_slug};

#[test]
fn normalize_slug_should_lowercase_and_collapse_invalid_characters() {
    let slug = normalize_slug(" JIRA__123 / Fix Login! ").expect("slug should normalize");

    assert_eq!(slug.as_str(), "jira-123-fix-login");
}

#[test]
fn normalize_slug_should_reject_inputs_that_have_no_hostname_characters() {
    let error = normalize_slug("!!!").expect_err("punctuation-only ticket should fail");

    assert_eq!(
        error.to_string(),
        "ticket slug is empty after normalization"
    );
}

#[test]
fn derive_names_should_create_deterministic_environment_identifiers() {
    let config = DinopodConfig::default();
    let names = derive_names("MyApp", "JIRA-123", Path::new("/repo/myapp"), &config)
        .expect("names should derive");

    assert_eq!(names.ticket_slug.as_str(), "jira-123");
    assert_eq!(names.project.as_str(), "myapp-jira-123");
    assert_eq!(names.host.as_str(), "jira-123-myapp.localhost");
    assert_eq!(names.network_alias.as_str(), "myapp-jira-123-app");
    assert_eq!(
        names.worktree_path.as_path(),
        Path::new("/repo/.dinopod-worktrees/myapp-jira-123")
    );
}

#[test]
fn normalize_slug_should_reject_traefik_unsafe_characters() {
    let error = normalize_slug("foo`bar").expect_err("backticks should fail");

    assert_eq!(
        error.to_string(),
        "ticket contains characters that are unsafe for local hostnames"
    );
}

#[test]
fn derive_names_should_keep_host_label_within_dns_limits_for_long_repo_slugs() {
    let config = DinopodConfig::default();
    let long_repo = "a".repeat(60);
    let names = derive_names(&long_repo, "JIRA-123", Path::new("/repo/myapp"), &config)
        .expect("names should derive");

    let label = names
        .host
        .as_str()
        .strip_suffix(".localhost")
        .expect("host should use configured suffix");
    assert!(label.len() <= 63);
    assert!(label.starts_with("jira-123-"));
}

#[test]
fn derive_names_should_keep_distinct_hosts_for_truncated_repo_slugs() {
    let config = DinopodConfig::default();
    let repo_a = format!("{}-alpha", "a".repeat(55));
    let repo_b = format!("{}-beta", "a".repeat(55));
    let host_a = derive_names(&repo_a, "JIRA-123", Path::new("/repo/a"), &config)
        .expect("names should derive")
        .host;
    let host_b = derive_names(&repo_b, "JIRA-123", Path::new("/repo/b"), &config)
        .expect("names should derive")
        .host;

    assert_ne!(host_a, host_b);
}

#[test]
fn domain_identifier_types_should_not_collapse_to_one_raw_string_type() {
    assert_ne!(TypeId::of::<HostName>(), TypeId::of::<ProjectName>());
    assert_ne!(TypeId::of::<ProjectName>(), TypeId::of::<NetworkAlias>());
    assert_ne!(TypeId::of::<NetworkAlias>(), TypeId::of::<WorktreePath>());
}
