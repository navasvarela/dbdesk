use dbdesk_core::{
    ConnectionConfig, DataSource, DataSourceId, FolderKind, NodeKind, SafetyLevel, SourceColor,
    SqliteConfig,
};
use dbdesk_engine::DdlFailure;
use dbdesk_sql::ObjectName;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

/// A form that asks for fixed operations, as a real helper's fields would produce them.
struct Fixed {
    operations: Result<Vec<DdlOperation>, DdlError>,
    destruction: Option<Destruction>,
}

impl ObjectForm for Fixed {
    fn title(&self) -> String {
        "Drop table orders".to_owned()
    }

    fn show(&mut self, ui: &mut Ui, _context: &FormContext<'_>) {
        ui.label("the form's fields");
    }

    fn operations(&self) -> Result<Vec<DdlOperation>, DdlError> {
        self.operations.clone()
    }

    fn changed(&self) -> CatalogPath {
        tables_folder()
    }

    fn destruction(&self, _dialect: Dialect) -> Option<Destruction> {
        self.destruction.clone()
    }
}

fn tables_folder() -> CatalogPath {
    CatalogPath::root()
        .child(NodeKind::Schema, "main")
        .child(NodeKind::Folder(FolderKind::Tables), "tables")
}

fn drop_orders() -> Result<Vec<DdlOperation>, DdlError> {
    Ok(vec![DdlOperation::DropTable {
        table: ObjectName::new(Some("main"), "orders"),
        cascade: false,
    }])
}

fn shop(safety: SafetyLevel, color: Option<SourceColor>) -> DataSource {
    DataSource {
        id: DataSourceId::generate(),
        name: "shop".to_owned(),
        color,
        safety,
        project: None,
        mcp_login: None,
        config: ConnectionConfig::Sqlite(SqliteConfig {
            path: "shop.db".into(),
            read_only: false,
        }),
    }
}

struct Open {
    dialog: ObjectDialog,
    actions: Vec<ObjectDialogAction>,
}

/// The dialog in a harness, with dbdesk's fonts installed on the first frame (its buttons
/// draw icons). Every action it returns is kept, in order.
fn harness(data_source: &DataSource, form: Fixed) -> Harness<'static, Open> {
    let open = Open {
        dialog: ObjectDialog::new(Target::new(data_source), Box::new(form)),
        actions: Vec::new(),
    };
    let mut fonts_installed = false;
    let mut harness = Harness::new_ui_state(
        move |ui, open: &mut Open| {
            if !fonts_installed {
                crate::fonts::install(ui.ctx());
                fonts_installed = true;
                return;
            }
            if let Some(action) = open.dialog.show(ui.ctx(), &CatalogTree::default()) {
                open.actions.push(action);
            }
        },
        open,
    );
    harness.run_steps(3);
    harness
}

fn applied(data_source: &DataSource) -> ObjectDialogAction {
    ObjectDialogAction::Execute {
        data_source: data_source.id,
        statements: vec!["DROP TABLE main.orders".to_owned()],
    }
}

#[test]
fn shows_the_sql_and_runs_it_only_on_execute() {
    let shop = shop(SafetyLevel::Normal, None);
    let mut harness = harness(
        &shop,
        Fixed {
            operations: drop_orders(),
            destruction: None,
        },
    );
    harness.get_by_label("the form's fields");
    assert!(
        harness
            .query_all_by_value("DROP TABLE main.orders;\n")
            .next()
            .is_some()
    );
    assert!(harness.state().actions.is_empty());

    harness.get_by_label("Execute").click();
    harness.run_steps(2);
    assert_eq!(harness.state().actions, [applied(&shop)]);
    // While it runs, nothing else can be sent and the dialog can't be cancelled.
    assert!(
        harness
            .get_by_label("Cancel")
            .accesskit_node()
            .is_disabled()
    );

    let failure = DdlFailure {
        statement: Some(0),
        message: "no such table: main.orders".to_owned(),
        applied: 0,
    };
    let refreshed = harness.state_mut().dialog.apply(&Event::DdlApplied {
        data_source: shop.id,
        result: Err(failure),
    });
    assert_eq!(refreshed, None);
    harness.run_steps(2);
    harness.get_by_label("no such table: main.orders");

    harness.get_by_label("Execute").click();
    harness.run_steps(2);
    let refreshed = harness.state_mut().dialog.apply(&Event::DdlApplied {
        data_source: shop.id,
        result: Ok(()),
    });
    assert_eq!(refreshed, Some(tables_folder()));
}

#[test]
fn opens_the_sql_in_a_console_instead() {
    let shop = shop(SafetyLevel::Normal, None);
    let mut harness = harness(
        &shop,
        Fixed {
            operations: drop_orders(),
            destruction: None,
        },
    );
    harness.get_by_label("Open in console").click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().actions,
        [ObjectDialogAction::OpenInConsole {
            data_source: shop.id,
            text: "DROP TABLE main.orders;\n".to_owned(),
        }]
    );
}

#[test]
fn a_careful_data_source_asks_before_running() {
    let shop = shop(SafetyLevel::Careful, None);
    let mut harness = harness(
        &shop,
        Fixed {
            operations: drop_orders(),
            destruction: None,
        },
    );
    harness.get_by_label("Execute").click();
    harness.run_steps(2);
    harness.get_by_label("\u{201c}shop\u{201d} is careful, and this changes the schema.");
    assert!(harness.state().actions.is_empty());
    harness.get_by_label("Run").click();
    harness.run_steps(2);
    assert_eq!(harness.state().actions, [applied(&shop)]);
}

#[test]
fn a_read_only_data_source_offers_only_the_console_and_copy() {
    let shop = shop(SafetyLevel::ReadOnly, None);
    let harness = harness(
        &shop,
        Fixed {
            operations: drop_orders(),
            destruction: None,
        },
    );
    assert!(
        harness
            .get_by_label("Execute")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        !harness
            .get_by_label("Open in console")
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label("\u{201c}shop\u{201d} is read-only, so this can't run here.");
}

#[test]
fn a_red_data_source_wants_the_name_typed_before_dropping() {
    let shop = shop(SafetyLevel::Normal, Some(SourceColor::Red));
    let mut harness = harness(
        &shop,
        Fixed {
            operations: drop_orders(),
            destruction: Some(Destruction {
                name: "orders".to_owned(),
                summary: "Drops the table main.orders, with every row in it.".to_owned(),
                consequences: vec![],
            }),
        },
    );
    harness.get_by_label("Drops the table main.orders, with every row in it.");
    assert!(
        harness
            .get_by_label("Execute")
            .accesskit_node()
            .is_disabled()
    );
    let prompt = "\u{201c}shop\u{201d} is marked red. Type orders to confirm:";
    harness
        .get_by_role_and_label(Role::TextInput, prompt)
        .focus();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, prompt)
        .type_text("orders");
    harness.run_steps(2);
    assert!(
        !harness
            .get_by_label("Execute")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn says_why_an_incomplete_form_has_no_sql() {
    let shop = shop(SafetyLevel::Normal, None);
    let harness = harness(
        &shop,
        Fixed {
            operations: Err(DdlError::NoColumns),
            destruction: None,
        },
    );
    harness.get_by_label("A table needs at least one column.");
    assert!(
        harness
            .get_by_label("Execute")
            .accesskit_node()
            .is_disabled()
    );
    assert!(harness.get_by_label("Copy").accesskit_node().is_disabled());
}

#[test]
fn stops_waiting_when_the_connection_fails() {
    let shop = shop(SafetyLevel::Normal, None);
    let mut harness = harness(
        &shop,
        Fixed {
            operations: drop_orders(),
            destruction: None,
        },
    );
    harness.get_by_label("Execute").click();
    harness.run_steps(2);
    harness.state_mut().dialog.apply(&Event::SessionChanged {
        id: shop.id,
        state: SessionState::Failed("connection refused".to_owned()),
    });
    harness.run_steps(2);
    harness.get_by_label("Could not connect: connection refused");
    assert!(
        !harness
            .get_by_label("Execute")
            .accesskit_node()
            .is_disabled()
    );
}
