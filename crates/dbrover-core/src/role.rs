//! PostgreSQL's roles: the users and groups of a whole server.

use std::collections::BTreeSet;

/// What a role may do, as `CREATE ROLE` and `ALTER ROLE` set it: each attribute is on or off,
/// written as its keyword or the keyword with `NO` in front (`LOGIN`, `NOLOGIN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoleAttribute {
    /// May log in: a role with it is what PostgreSQL calls a user.
    Login,
    /// Bypasses every permission check.
    Superuser,
    CreateDb,
    CreateRole,
    /// Has the privileges of the roles it is a member of without `SET ROLE`.
    Inherit,
    /// May start streaming replication and take base backups.
    Replication,
    /// Ignores row-level security policies.
    BypassRls,
}

impl RoleAttribute {
    /// In the order PostgreSQL's documentation lists them, which is how dialogs show them.
    pub const ALL: [Self; 7] = [
        Self::Login,
        Self::Superuser,
        Self::CreateDb,
        Self::CreateRole,
        Self::Inherit,
        Self::Replication,
        Self::BypassRls,
    ];

    /// The keyword that turns the attribute on; `NO` in front of it turns it off.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Login => "LOGIN",
            Self::Superuser => "SUPERUSER",
            Self::CreateDb => "CREATEDB",
            Self::CreateRole => "CREATEROLE",
            Self::Inherit => "INHERIT",
            Self::Replication => "REPLICATION",
            Self::BypassRls => "BYPASSRLS",
        }
    }

    /// Whether a new role has it when `CREATE ROLE` doesn't say. Only `INHERIT` is on.
    pub fn is_default(self) -> bool {
        self == Self::Inherit
    }

    /// What it lets the role do, for a tooltip.
    pub fn description(self) -> &'static str {
        match self {
            Self::Login => "May log in: a role that may is a user",
            Self::Superuser => "Bypasses every permission check",
            Self::CreateDb => "May create databases",
            Self::CreateRole => "May create, change and drop other roles",
            Self::Inherit => "Has the privileges of the roles it belongs to without SET ROLE",
            Self::Replication => "May stream replication and take base backups",
            Self::BypassRls => "Ignores row-level security policies",
        }
    }
}

/// A role as the database describes it, for editing it and for what dropping it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleDescription {
    pub name: String,
    /// The attributes that are on.
    pub attributes: BTreeSet<RoleAttribute>,
    /// `None` for no limit.
    pub connection_limit: Option<u32>,
    /// When its password stops working, as the database prints the timestamp; `None` for
    /// never, whether it was never set or set to `infinity`.
    pub valid_until: Option<String>,
    /// The roles it is a member of.
    pub member_of: BTreeSet<String>,
    /// The roles that are members of it, which lose that membership when it is dropped.
    pub members: BTreeSet<String>,
    /// How many objects it owns in the connected database.
    pub owned_objects: u64,
    /// How many objects in the connected database it holds privileges on.
    pub privileges: u64,
    /// The databases it owns, in alphabetical order. Dropping it fails while there are any.
    pub owned_databases: Vec<String>,
}
