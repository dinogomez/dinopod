//! Type-safe domain identifiers used by Dinopod orchestration.

use std::fmt;
use std::path::{Path, PathBuf};

macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Creates a new identifier from an already validated value.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns this identifier as a borrowed string.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

string_id!(
    TicketSlug,
    "Normalized ticket identifier safe for local hostnames."
);
string_id!(
    ProjectName,
    "Docker Compose project name for a Dinopod environment."
);
string_id!(HostName, "Local hostname routed to a Dinopod environment.");
string_id!(
    NetworkAlias,
    "Docker network alias used as the proxy upstream target."
);

/// Filesystem path for a ticket worktree.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorktreePath(PathBuf);

impl WorktreePath {
    /// Creates a new worktree path.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// Returns the worktree path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}
