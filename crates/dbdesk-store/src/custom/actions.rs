//! `actions.toml`: SQL the user attaches to explorer nodes and grid cells.

use std::fmt;

use dbdesk_core::{DriverKind, NodeKind, Placeholder, SqlTemplate};
use serde::Deserialize;

use super::{Checked, drivers, entries, entry_label, serde_problem};

/// An action's stable name: what `keymap.toml` binds as `custom.<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionId(String);

impl ActionId {
    /// The prefix that sets custom actions apart from dbdesk's own in `keymap.toml`.
    pub const KEYMAP_PREFIX: &str = "custom.";

    /// Lowercase letters, digits, `-` and `_`, starting with a letter or digit, so an id reads
    /// the same in TOML keys, menu ids and the keymap.
    pub fn new(id: &str) -> Option<Self> {
        let mut chars = id.chars();
        let valid = chars
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        valid.then(|| Self(id.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `custom.<id>`.
    pub fn keymap_id(&self) -> String {
        format!("{}{}", Self::KEYMAP_PREFIX, self.0)
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What an action can be chosen on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionTarget {
    /// A data source's own row in the explorer.
    DataSource,
    /// An explorer node of this kind; never a folder.
    Node(NodeKind),
    /// The grid's selected cells, acted on once.
    GridSelection,
    /// The grid's selected rows, acted on once each.
    GridRow,
}

/// The node kinds an action can name, as `applies_to` writes them.
const NODE_KINDS: [(&str, NodeKind); 16] = [
    ("database", NodeKind::Database),
    ("schema", NodeKind::Schema),
    ("table", NodeKind::Table),
    ("view", NodeKind::View),
    ("materialized_view", NodeKind::MaterializedView),
    ("function", NodeKind::Function),
    ("procedure", NodeKind::Procedure),
    ("sequence", NodeKind::Sequence),
    ("event", NodeKind::Event),
    ("synonym", NodeKind::Synonym),
    ("dictionary", NodeKind::Dictionary),
    ("column", NodeKind::Column),
    ("index", NodeKind::Index),
    ("constraint", NodeKind::Constraint),
    ("trigger", NodeKind::Trigger),
    ("role", NodeKind::Role),
];

impl ActionTarget {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "data_source" => Some(Self::DataSource),
            "grid.selection" => Some(Self::GridSelection),
            "grid.row" => Some(Self::GridRow),
            _ => NODE_KINDS
                .iter()
                .find(|(written, _)| *written == name)
                .map(|&(_, kind)| Self::Node(kind)),
        }
    }

    /// As `applies_to` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::DataSource => "data_source",
            Self::GridSelection => "grid.selection",
            Self::GridRow => "grid.row",
            Self::Node(kind) => NODE_KINDS
                .iter()
                .find(|(_, known)| *known == kind)
                .map_or("folder", |(written, _)| written),
        }
    }

    pub fn is_grid(self) -> bool {
        matches!(self, Self::GridSelection | Self::GridRow)
    }

    /// Whether something acted on here can have a value for `placeholder` at all. Whether it
    /// does is only known when the action runs: a table in a database without schemas has no
    /// `{schema}`.
    fn can_fill(self, placeholder: &Placeholder) -> bool {
        use NodeKind as K;
        let node = match self {
            Self::GridSelection | Self::GridRow => return true,
            Self::DataSource => None,
            Self::Node(kind) => Some(kind),
        };
        match placeholder {
            Placeholder::Value | Placeholder::Values | Placeholder::Row(_) => false,
            Placeholder::Database | Placeholder::DataSource => true,
            Placeholder::Column { default } => default.is_some() || node == Some(K::Column),
            Placeholder::Table => matches!(
                node,
                Some(
                    K::Table
                        | K::View
                        | K::MaterializedView
                        | K::Column
                        | K::Index
                        | K::Constraint
                        | K::Trigger
                )
            ),
            Placeholder::Schema => !matches!(node, None | Some(K::Database | K::Role)),
            Placeholder::QualifiedName => {
                !matches!(node, None | Some(K::Database | K::Schema | K::Folder(_)))
            }
        }
    }
}

/// One action from `actions.toml`, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomAction {
    pub id: ActionId,
    /// What the menus show.
    pub name: String,
    pub applies_to: Vec<ActionTarget>,
    /// `None` for every driver.
    pub drivers: Option<Vec<DriverKind>>,
    pub sql: SqlTemplate,
    /// Run at once; otherwise open the SQL in a console to edit first.
    pub run: bool,
}

impl CustomAction {
    pub fn applies_to(&self, target: ActionTarget, driver: DriverKind) -> bool {
        self.applies_to.contains(&target) && self.runs_on(driver)
    }

    pub fn runs_on(&self, driver: DriverKind) -> bool {
        self.drivers
            .as_ref()
            .is_none_or(|drivers| drivers.contains(&driver))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    id: String,
    name: String,
    applies_to: Vec<String>,
    #[serde(default)]
    drivers: Option<Vec<String>>,
    sql: String,
    #[serde(default = "run_by_default")]
    run: bool,
}

fn run_by_default() -> bool {
    true
}

/// Checks the text of `actions.toml`.
pub fn parse_actions(text: &str) -> Checked<Vec<CustomAction>> {
    let mut checked = Checked::<Vec<CustomAction>>::default();
    for (index, entry) in entries(text, "action", &mut checked.problems)
        .into_iter()
        .enumerate()
    {
        let label = entry_label("action", &entry, "id", index);
        let action = entry
            .try_into::<RawAction>()
            .map_err(|error| serde_problem(&error))
            .and_then(check);
        match action {
            Ok(action) if checked.value.iter().any(|kept| kept.id == action.id) => {
                checked.problems.push(format!(
                    "{label}: another action already has this id; the first one is kept"
                ));
            }
            Ok(action) => checked.value.push(action),
            Err(problem) => checked.problems.push(format!("{label}: {problem}")),
        }
    }
    checked
}

fn check(raw: RawAction) -> Result<CustomAction, String> {
    let id = ActionId::new(&raw.id).ok_or_else(|| {
        "an id may only have lowercase letters, digits, \"-\" and \"_\", and starts with a \
         letter or digit"
            .to_owned()
    })?;
    if raw.name.trim().is_empty() {
        return Err("the name is empty".to_owned());
    }
    if raw.applies_to.is_empty() {
        return Err("applies_to is empty, so the action appears nowhere".to_owned());
    }
    let applies_to = raw
        .applies_to
        .iter()
        .map(|name| {
            ActionTarget::parse(name).ok_or_else(|| {
                format!(
                    "{name:?} is not something actions apply to; use a node kind such as \
                     table or column, data_source, grid.selection or grid.row"
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let drivers = drivers(raw.drivers)?;
    if raw.sql.trim().is_empty() {
        return Err("the sql is empty".to_owned());
    }
    let sql = SqlTemplate::parse(&raw.sql).map_err(|error| error.to_string())?;
    for placeholder in sql.placeholders() {
        if let Some(target) = applies_to
            .iter()
            .find(|target| !target.can_fill(placeholder))
        {
            return Err(unfillable(placeholder, *target));
        }
    }
    Ok(CustomAction {
        id,
        name: raw.name,
        applies_to,
        drivers,
        sql,
        run: raw.run,
    })
}

fn unfillable(placeholder: &Placeholder, target: ActionTarget) -> String {
    let target = target.name();
    match placeholder {
        Placeholder::Column { .. } => format!(
            "{placeholder} can't be filled for {target:?}; give it a default, as \
             {{column:created_at}}"
        ),
        _ if placeholder.needs_cells() => {
            format!("{placeholder} can't be filled for {target:?}: only the grid has values")
        }
        _ => format!("{placeholder} can't be filled for {target:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
        [[action]]
        id = "row-count"
        name = "Row count"
        applies_to = ["table", "view"]
        drivers = ["postgres", "mysql"]
        sql = "SELECT count(*) FROM {qualified_name}"

        [[action]]
        id = "latest"
        name = "Last 100 rows"
        applies_to = ["table"]
        sql = "SELECT * FROM {qualified_name} ORDER BY {column:created_at} DESC LIMIT 100"
        run = false

        [[action]]
        id = "find-order"
        name = "Find the order"
        applies_to = ["grid.selection", "grid.row"]
        sql = "SELECT * FROM orders WHERE id IN ({values})"
    "#;

    #[test]
    fn reads_the_plans_example() {
        let checked = parse_actions(EXAMPLE);

        assert!(checked.problems.is_empty(), "{:?}", checked.problems);
        let [count, latest, find] = checked.value.as_slice() else {
            panic!("{:?}", checked.value);
        };
        assert_eq!(count.id.keymap_id(), "custom.row-count");
        assert_eq!(
            count.applies_to,
            [
                ActionTarget::Node(NodeKind::Table),
                ActionTarget::Node(NodeKind::View)
            ]
        );
        assert!(count.applies_to(ActionTarget::Node(NodeKind::View), DriverKind::Mysql));
        assert!(!count.applies_to(ActionTarget::Node(NodeKind::View), DriverKind::Sqlite));
        assert!(count.run);
        assert!(!latest.run);
        assert!(latest.runs_on(DriverKind::Sqlite));
        assert!(find.applies_to(ActionTarget::GridRow, DriverKind::DuckDb));
    }

    #[test]
    fn skips_and_names_each_broken_action() {
        let entry = |id: &str, fields: &str| {
            format!(
                "[[action]]\nid = {id:?}\nname = \"N\"\napplies_to = [\"table\"]\nsql = \"SELECT 1\"\n{fields}\n"
            )
        };
        let cases = [
            (
                "[[action]]\nname = \"N\"\n".to_owned(),
                "action 1: missing field `id`",
            ),
            (
                entry("row-count", "nmae = \"x\""),
                "action \"row-count\": unknown field `nmae`, expected one of `id`, `name`, \
                 `applies_to`, `drivers`, `sql`, `run`",
            ),
            (
                entry("Row Count", ""),
                "action \"Row Count\": an id may only have lowercase letters, digits, \"-\" and \
                 \"_\", and starts with a letter or digit",
            ),
            (
                entry("x", "").replace("name = \"N\"", "name = \" \""),
                "action \"x\": the name is empty",
            ),
            (
                entry("x", "").replace("[\"table\"]", "[]"),
                "action \"x\": applies_to is empty, so the action appears nowhere",
            ),
            (
                entry("x", "").replace("[\"table\"]", "[\"tabel\"]"),
                "action \"x\": \"tabel\" is not something actions apply to; use a node kind \
                 such as table or column, data_source, grid.selection or grid.row",
            ),
            (
                entry("x", "drivers = [\"oracle\"]"),
                "action \"x\": \"oracle\" is not a driver; the drivers are postgres, sqlite, \
                 mysql, sqlserver, duckdb, clickhouse",
            ),
            (
                entry("x", "").replace("SELECT 1", " "),
                "action \"x\": the sql is empty",
            ),
            (
                entry("x", "").replace("SELECT 1", "SELECT {tabel}"),
                "action \"x\": {tabel} is not a placeholder",
            ),
            (
                entry("x", "").replace("SELECT 1", "SELECT {value}"),
                "action \"x\": {value} can't be filled for \"table\": only the grid has values",
            ),
            (
                entry("x", "").replace("SELECT 1", "SELECT {column}"),
                "action \"x\": {column} can't be filled for \"table\"; give it a default, as \
                 {column:created_at}",
            ),
            (
                entry("x", "")
                    .replace("[\"table\"]", "[\"schema\"]")
                    .replace("SELECT 1", "SELECT {table}"),
                "action \"x\": {table} can't be filled for \"schema\"",
            ),
            (
                format!("{}{}", entry("x", ""), entry("x", "")),
                "action \"x\": another action already has this id; the first one is kept",
            ),
        ];
        for (text, problem) in cases {
            let checked = parse_actions(&text);
            assert_eq!(checked.problems, [problem], "{text}");
            assert!(checked.value.len() <= 1, "{text}");
        }
    }

    #[test]
    fn a_broken_action_costs_only_itself() {
        let text = format!("{EXAMPLE}\n[[action]]\nid = \"broken\"\n");
        let checked = parse_actions(&text);
        assert_eq!(checked.value.len(), 3);
        assert_eq!(checked.problems.len(), 1);
    }

    #[test]
    fn which_targets_can_fill_which_placeholders() {
        let table = ActionTarget::Node(NodeKind::Table);
        let column = ActionTarget::Node(NodeKind::Column);
        let schema = ActionTarget::Node(NodeKind::Schema);
        let no_default = Placeholder::Column { default: None };
        let cases = [
            (table, Placeholder::QualifiedName, true),
            (table, Placeholder::Table, true),
            (table, no_default.clone(), false),
            (column, no_default.clone(), true),
            (column, Placeholder::Table, true),
            (schema, Placeholder::Schema, true),
            (schema, Placeholder::QualifiedName, false),
            (ActionTarget::DataSource, Placeholder::Database, true),
            (ActionTarget::DataSource, Placeholder::Schema, false),
            (
                ActionTarget::GridRow,
                Placeholder::Row("id".to_owned()),
                true,
            ),
            (ActionTarget::GridSelection, no_default, true),
        ];
        for (target, placeholder, expected) in cases {
            assert_eq!(
                target.can_fill(&placeholder),
                expected,
                "{placeholder} on {}",
                target.name()
            );
        }
    }
}
