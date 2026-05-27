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
fn domain_identifier_types_should_not_collapse_to_one_raw_string_type() {
    assert_ne!(TypeId::of::<HostName>(), TypeId::of::<ProjectName>());
    assert_ne!(TypeId::of::<ProjectName>(), TypeId::of::<NetworkAlias>());
    assert_ne!(TypeId::of::<NetworkAlias>(), TypeId::of::<WorktreePath>());
}
