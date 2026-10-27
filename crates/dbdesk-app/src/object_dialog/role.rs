//! New user…, New role… and Edit…: one form for PostgreSQL's roles, where a user is a role
//! that may log in. The password is hashed into a SCRAM verifier as it is typed, so the
//! operations only ever hold the verifier.

use std::collections::BTreeSet;

use dbdesk_core::{CatalogPath, FolderKind, NodeKind, RoleAttribute, RoleDescription};
use dbdesk_sql::{DdlError, DdlOperation, PasswordChange, RoleSpec, ScramVerifier};
use egui::{DragValue, Grid, TextEdit, Ui, WidgetType};

use super::{FormContext, ObjectForm};
use crate::catalog_tree::CatalogTree;
use crate::widgets;

const FIELD_WIDTH: f32 = 260.0;
/// Where the connection limit field starts when Limit connections is ticked.
const DEFAULT_CONNECTION_LIMIT: u32 = 10;

/// The roles folder at the top of the tree, where every role is listed.
pub(super) fn roles_folder() -> CatalogPath {
    CatalogPath::root().child(
        NodeKind::Folder(FolderKind::Roles),
        FolderKind::Roles.label(),
    )
}

/// The role at `path`, a node of the roles folder.
pub(super) fn role_name(path: &CatalogPath) -> Option<&str> {
    match path.segments() {
        [folder, role]
            if folder.kind == NodeKind::Folder(FolderKind::Roles)
                && role.kind == NodeKind::Role =>
        {
            Some(role.name.as_str())
        }
        _ => None,
    }
}

/// What New starts as: the same dialog, with LOGIN ticked for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoleKind {
    User,
    Role,
}

impl RoleKind {
    fn noun(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Role => "role",
        }
    }

    fn of(attributes: &BTreeSet<RoleAttribute>) -> Self {
        if attributes.contains(&RoleAttribute::Login) {
            Self::User
        } else {
            Self::Role
        }
    }
}

/// The attribute as the dialog names it; its keyword and what it does are in its tooltip.
fn attribute_label(attribute: RoleAttribute) -> &'static str {
    match attribute {
        RoleAttribute::Login => "Can log in",
        RoleAttribute::Superuser => "Superuser",
        RoleAttribute::CreateDb => "Create databases",
        RoleAttribute::CreateRole => "Create roles",
        RoleAttribute::Inherit => "Inherit privileges",
        RoleAttribute::Replication => "Replication",
        RoleAttribute::BypassRls => "Bypass row security",
    }
}

/// A password typed twice, and its verifier once both match.
#[derive(Default)]
struct PasswordFields {
    password: String,
    confirmation: String,
    verifier: Option<ScramVerifier>,
}

impl PasswordFields {
    /// `label` names the first field: "Password", or "New password" when editing.
    fn show(&mut self, ui: &mut Ui, label: &str, enabled: bool) {
        let mut changed = false;
        for (text, field) in [
            (label, &mut self.password),
            ("Confirm password", &mut self.confirmation),
        ] {
            let label = ui.label(text);
            changed |= ui
                .add_enabled(
                    enabled,
                    TextEdit::singleline(field)
                        .password(true)
                        .desired_width(FIELD_WIDTH),
                )
                .labelled_by(label.id)
                .changed();
            ui.end_row();
        }
        // Hashing takes thousands of rounds, so it runs once per change, not every frame.
        if changed {
            self.verifier = (!self.password.is_empty() && self.password == self.confirmation)
                .then(|| ScramVerifier::hash(&self.password));
        }
    }

    /// `None` when nothing was typed.
    fn verifier(&self) -> Result<Option<ScramVerifier>, DdlError> {
        if self.password.is_empty() && self.confirmation.is_empty() {
            return Ok(None);
        }
        self.verifier
            .clone()
            .map(Some)
            .ok_or(DdlError::PasswordsDiffer)
    }
}

/// Everything about a role but its name and password.
struct RoleFields {
    attributes: BTreeSet<RoleAttribute>,
    connection_limit: Option<u32>,
    valid_until: String,
    member_of: BTreeSet<String>,
}

impl RoleFields {
    fn new(kind: RoleKind) -> Self {
        let mut attributes: BTreeSet<RoleAttribute> = RoleAttribute::ALL
            .into_iter()
            .filter(|attribute| attribute.is_default())
            .collect();
        if kind == RoleKind::User {
            attributes.insert(RoleAttribute::Login);
        }
        Self {
            attributes,
            connection_limit: None,
            valid_until: String::new(),
            member_of: BTreeSet::new(),
        }
    }

    fn described(role: &RoleDescription) -> Self {
        Self {
            attributes: role.attributes.clone(),
            connection_limit: role.connection_limit,
            valid_until: role.valid_until.clone().unwrap_or_default(),
            member_of: role.member_of.clone(),
        }
    }

    fn spec(&self) -> RoleSpec {
        let valid_until = self.valid_until.trim();
        RoleSpec {
            attributes: self.attributes.clone(),
            connection_limit: self.connection_limit,
            valid_until: (!valid_until.is_empty()).then(|| valid_until.to_owned()),
            member_of: self.member_of.clone(),
        }
    }

    /// The rows after the name and password, in the same grid.
    fn show_rows(&mut self, ui: &mut Ui, catalog: &CatalogTree, own_name: &str) {
        ui.label("Connections");
        ui.horizontal(|ui| {
            let mut limited = self.connection_limit.is_some();
            if ui.checkbox(&mut limited, "Limit to").changed() {
                self.connection_limit = limited.then_some(DEFAULT_CONNECTION_LIMIT);
            }
            if let Some(limit) = &mut self.connection_limit {
                ui.add(DragValue::new(limit));
            }
        });
        ui.end_row();
        let label = ui.label("Valid until");
        ui.add(
            TextEdit::singleline(&mut self.valid_until)
                .desired_width(FIELD_WIDTH)
                .hint_text("never, or e.g. 2027-01-01"),
        )
        .labelled_by(label.id);
        ui.end_row();
        ui.label("Member of");
        let roles = other_roles(catalog, own_name);
        membership_picker(ui, &mut self.member_of, &roles);
        ui.end_row();
    }

    fn show_attributes(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            for attribute in RoleAttribute::ALL {
                let mut on = self.attributes.contains(&attribute);
                let checkbox = ui
                    .checkbox(&mut on, attribute_label(attribute))
                    .on_hover_text(format!(
                        "{}: {}",
                        attribute.keyword(),
                        attribute.description()
                    ));
                if checkbox.changed() {
                    if on {
                        self.attributes.insert(attribute);
                    } else {
                        self.attributes.remove(&attribute);
                    }
                }
            }
        });
    }
}

/// Every role in the loaded roles folder but `own_name`, PostgreSQL's own `pg_` roles
/// included: granting one (`pg_read_all_data`, say) is how a role gets its privileges.
fn other_roles(catalog: &CatalogTree, own_name: &str) -> Vec<String> {
    catalog
        .loaded_children(&roles_folder())
        .into_iter()
        .flatten()
        .filter(|node| node.kind() == Some(NodeKind::Role) && node.label != own_name)
        .map(|node| node.label.clone())
        .collect()
}

fn membership_picker(ui: &mut Ui, chosen: &mut BTreeSet<String>, roles: &[String]) {
    let text = if chosen.is_empty() {
        "None".to_owned()
    } else {
        chosen.iter().cloned().collect::<Vec<_>>().join(", ")
    };
    let menu = ui.menu_button(text, |ui| {
        if roles.is_empty() {
            ui.weak("No other roles yet");
        }
        for role in roles {
            let mut ticked = chosen.contains(role);
            if ui.checkbox(&mut ticked, role).changed() {
                if ticked {
                    chosen.insert(role.clone());
                } else {
                    chosen.remove(role);
                }
            }
        }
    });
    widgets::name(
        menu.response,
        WidgetType::Button,
        "Roles it belongs to",
        true,
    );
}

pub(crate) struct NewRole {
    kind: RoleKind,
    name: String,
    password: PasswordFields,
    fields: RoleFields,
}

impl NewRole {
    /// On the data source (the root) or its roles folder.
    pub(crate) fn new(path: &CatalogPath, kind: RoleKind) -> Option<Self> {
        (path.is_root() || *path == roles_folder()).then(|| Self {
            kind,
            name: String::new(),
            password: PasswordFields::default(),
            fields: RoleFields::new(kind),
        })
    }
}

impl ObjectForm for NewRole {
    fn title(&self) -> String {
        format!("New {}", self.kind.noun())
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        Grid::new("new_role").num_columns(2).show(ui, |ui| {
            let label = ui.label("Name");
            ui.add(TextEdit::singleline(&mut self.name).desired_width(FIELD_WIDTH))
                .labelled_by(label.id);
            ui.end_row();
            self.password.show(ui, "Password", true);
            self.fields.show_rows(ui, context.catalog, self.name.trim());
        });
        self.fields.show_attributes(ui);
    }

    fn catalog_requests(&self) -> Vec<CatalogPath> {
        vec![roles_folder()]
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        Ok(vec![DdlOperation::CreateRole {
            name: self.name.trim().to_owned(),
            spec: self.fields.spec(),
            password: self.password.verifier()?,
        }])
    }

    fn changed(&self) -> CatalogPath {
        roles_folder()
    }
}

enum Described {
    Loading,
    /// The role as it was described, to compare the fields with.
    Loaded(RoleSpec),
    Failed(String),
}

pub(crate) struct EditRole {
    path: CatalogPath,
    name: String,
    described: Described,
    fields: RoleFields,
    password: PasswordFields,
    remove_password: bool,
}

impl EditRole {
    /// On a role.
    pub(crate) fn new(path: CatalogPath) -> Option<Self> {
        let name = role_name(&path)?.to_owned();
        Some(Self {
            path,
            name,
            described: Described::Loading,
            fields: RoleFields::new(RoleKind::Role),
            password: PasswordFields::default(),
            remove_password: false,
        })
    }

    fn password_change(&self) -> Result<PasswordChange, DdlError> {
        if self.remove_password {
            return Ok(PasswordChange::Remove);
        }
        Ok(match self.password.verifier()? {
            Some(verifier) => PasswordChange::Set(verifier),
            None => PasswordChange::Keep,
        })
    }
}

impl ObjectForm for EditRole {
    fn title(&self) -> String {
        let kind = match &self.described {
            Described::Loaded(before) => RoleKind::of(&before.attributes),
            Described::Loading | Described::Failed(_) => RoleKind::Role,
        };
        format!("Edit {} {}", kind.noun(), self.name)
    }

    fn show(&mut self, ui: &mut Ui, context: &FormContext<'_>) {
        match &self.described {
            Described::Loading => {
                ui.weak("Loading the role…");
                return;
            }
            Described::Failed(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                return;
            }
            Described::Loaded(_) => {}
        }
        Grid::new("edit_role").num_columns(2).show(ui, |ui| {
            self.password
                .show(ui, "New password", !self.remove_password);
            ui.label("");
            ui.checkbox(&mut self.remove_password, "Remove password")
                .on_hover_text("PASSWORD NULL: password logins fail until a new one is set");
            ui.end_row();
            self.fields.show_rows(ui, context.catalog, &self.name);
        });
        self.fields.show_attributes(ui);
        ui.weak("Leave the password empty to keep the one it has.");
    }

    fn catalog_requests(&self) -> Vec<CatalogPath> {
        vec![roles_folder()]
    }

    fn role_request(&self) -> Option<CatalogPath> {
        Some(self.path.clone())
    }

    fn role_described(&mut self, result: &Result<RoleDescription, String>) {
        match result {
            Ok(role) => {
                self.fields = RoleFields::described(role);
                self.described = Described::Loaded(RoleSpec::from(role));
            }
            Err(error) => {
                self.described = Described::Failed(format!("Could not load the role: {error}"));
            }
        }
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        let Described::Loaded(before) = &self.described else {
            return Err(DdlError::NothingChanged);
        };
        Ok(vec![DdlOperation::AlterRole {
            name: self.name.clone(),
            before: before.clone(),
            after: self.fields.spec(),
            password: self.password_change()?,
        }])
    }

    fn changed(&self) -> CatalogPath {
        roles_folder()
    }
}
