//! PostgreSQL's roles: the users and groups of a whole server.

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
