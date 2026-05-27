//! Deterministic name derivation for Dinopod environments.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use crate::config::DinopodConfig;
use crate::domain::{HostName, NetworkAlias, ProjectName, TicketSlug, WorktreePath};

/// All derived names for a Dinopod environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentNames {
    /// Normalized ticket slug.
    pub ticket_slug: TicketSlug,
    /// Docker Compose project name.
    pub project: ProjectName,
    /// Local hostname.
    pub host: HostName,
    /// Docker network alias for the app container.
    pub network_alias: NetworkAlias,
    /// Worktree path.
    pub worktree_path: WorktreePath,
}

/// Name derivation errors.
#[derive(Debug, thiserror::Error)]
pub enum NameError {
    /// Normalization produced no usable characters.
    #[error("ticket slug is empty after normalization")]
    EmptyTicketSlug,
    /// Repository name normalization produced no usable characters.
    #[error("repository slug is empty after normalization")]
    EmptyRepoSlug,
    /// Ticket input contains characters unsafe for Traefik host rules.
    #[error("ticket contains characters that are unsafe for local hostnames")]
    InvalidTicketCharacters,
}

/// Normalizes arbitrary input into a lowercase slug safe for local hostnames.
///
/// # Errors
///
/// Returns [`NameError::EmptyTicketSlug`] when no valid slug characters remain.
pub fn normalize_slug(input: &str) -> Result<TicketSlug, NameError> {
    validate_ticket_characters(input)?;
    normalize_to_string(input)
        .map(TicketSlug::new)
        .ok_or(NameError::EmptyTicketSlug)
}

/// Derives all environment names for a repo/ticket pair.
///
/// # Errors
///
/// Returns [`NameError`] when the repo or ticket cannot produce a usable slug.
pub fn derive_names(
    repo_name: &str,
    ticket: &str,
    repo_root: &Path,
    config: &DinopodConfig,
) -> Result<EnvironmentNames, NameError> {
    let repo_slug = normalize_to_string(repo_name).ok_or(NameError::EmptyRepoSlug)?;
    let ticket_slug = normalize_slug(ticket)?;
    let project = ProjectName::new(format!("{repo_slug}-{}", ticket_slug.as_str()));
    let host = HostName::new(format!(
        "{}.{}",
        build_host_label(ticket_slug.as_str(), &repo_slug),
        config.proxy.host_suffix
    ));
    let network_alias = NetworkAlias::new(format!("{}-{}", project.as_str(), config.app.service));
    let worktree_root = if config.worktree.root.is_absolute() {
        config.worktree.root.clone()
    } else {
        normalize_path(&repo_root.join(&config.worktree.root))
    };

    Ok(EnvironmentNames {
        ticket_slug,
        project: project.clone(),
        host,
        network_alias,
        worktree_path: WorktreePath::new(worktree_root.join(project.as_str())),
    })
}

const DNS_LABEL_MAX_LEN: usize = 63;

fn build_host_label(ticket: &str, repo: &str) -> String {
    let label = format!("{ticket}-{repo}");
    if label.len() <= DNS_LABEL_MAX_LEN {
        return label;
    }

    let hash_suffix = short_label_hash(&label);
    let ticket_with_hash = format!("{ticket}-{hash_suffix}");
    if ticket_with_hash.len() <= DNS_LABEL_MAX_LEN {
        return ticket_with_hash;
    }

    let max_ticket_len = DNS_LABEL_MAX_LEN.saturating_sub(hash_suffix.len() + 1);
    let truncated_ticket: String = ticket.chars().take(max_ticket_len).collect();
    format!("{truncated_ticket}-{hash_suffix}")
}

fn short_label_hash(input: &str) -> String {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:x}", hasher.finish()).chars().take(8).collect()
}

fn normalize_to_string(input: &str) -> Option<String> {
    let mut output = String::new();
    let mut last_was_dash = false;

    for character in input.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            output.push(character);
            last_was_dash = false;
        } else if !last_was_dash && !output.is_empty() {
            output.push('-');
            last_was_dash = true;
        }
    }

    while output.ends_with('-') {
        output.pop();
    }

    (!output.is_empty()).then_some(output)
}

fn validate_ticket_characters(input: &str) -> Result<(), NameError> {
    if input.contains("||")
        || input
            .chars()
            .any(|character| matches!(character, '`' | '(' | ')' | '"' | '\''))
    {
        return Err(NameError::InvalidTicketCharacters);
    }
    Ok(())
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }

    normalized
}
