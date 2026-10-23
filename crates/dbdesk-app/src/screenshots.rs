//! Renders the app headlessly into PNG screenshots, so the user manual's pictures are
//! regenerated from code instead of taken by hand.
//!
//! Ignored by default, because it needs a GPU adapter and rewrites files:
//!
//! ```text
//! cargo test -p dbdesk-app regenerate_screenshots -- --ignored
//! ```
//!
//! Images go to `book/src/images`, or `DBDESK_SCREENSHOTS_DIR` when set. Scenes are rendered in
//! the light theme, or in each theme `DBDESK_SCREENSHOTS_THEMES` lists (`light,dark`), against a
//! SQLite fixture database. When `DBDESK_TEST_POSTGRES_URL` is set, a PostgreSQL data source
//! joins it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use dbdesk_core::{
    ConnectionConfig, DataSource, DataSourceId, Password, PostgresConfig, SourceColor, SqliteConfig,
};
use dbdesk_store::DataSourceFile;
use egui::accesskit::Role;
use egui::{Theme, Vec2};
use egui_kittest::kittest::{By, Queryable};
use egui_kittest::{Harness, HarnessBuilder};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::{Layer as _, reload};

use crate::app::DbDeskApp;
use crate::log_console::{self, LogConsoleLayer};
use crate::telemetry;
use crate::test_support::{MemorySecrets, start_app, step_until_visible};

/// A typical laptop window; wide enough for the explorer, a console and its results.
const WINDOW_SIZE: Vec2 = Vec2::new(1180.0, 740.0);
/// Rendered at 2x so the images stay sharp on high-density screens.
const PIXELS_PER_POINT: f32 = 2.0;
/// Enough frames for egui's fade-in animations and the dock's layout to settle.
const SETTLE_STEPS: usize = 8;

const SHOP_SCHEMA: &str = "
    CREATE TABLE customers (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        email TEXT UNIQUE,
        country TEXT NOT NULL
    );
    CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price NUMERIC NOT NULL);
    CREATE TABLE orders (
        id INTEGER PRIMARY KEY,
        customer_id INTEGER NOT NULL REFERENCES customers (id),
        placed_at TEXT NOT NULL,
        status TEXT NOT NULL
    );
    CREATE TABLE order_items (
        order_id INTEGER NOT NULL REFERENCES orders (id),
        product_id INTEGER NOT NULL REFERENCES products (id),
        quantity INTEGER NOT NULL,
        PRIMARY KEY (order_id, product_id)
    );
    CREATE INDEX orders_by_customer ON orders (customer_id);
    CREATE VIEW order_totals AS
        SELECT o.id, o.customer_id, SUM(i.quantity * p.price) AS total
        FROM orders o
        JOIN order_items i ON i.order_id = o.id
        JOIN products p ON p.id = i.product_id
        GROUP BY o.id;

    INSERT INTO customers (name, email, country) VALUES
        ('Ada Lovelace', 'ada@example.com', 'GB'),
        ('Grace Hopper', 'grace@example.com', 'US'),
        ('Edsger Dijkstra', 'edsger@example.com', 'NL'),
        ('Barbara Liskov', 'barbara@example.com', 'US'),
        ('Donald Knuth', 'don@example.com', 'US'),
        ('Margaret Hamilton', 'margaret@example.com', 'US'),
        ('Niklaus Wirth', 'niklaus@example.com', 'CH'),
        ('Frances Allen', NULL, 'US');
    INSERT INTO products (name, price) VALUES
        ('Mechanical keyboard', 129.00), ('Desk lamp', 45.50), ('Monitor arm', 89.90),
        ('Notebook', 6.20), ('Fountain pen', 32.00), ('Headphones', 199.00);
    WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 60)
    INSERT INTO orders (customer_id, placed_at, status)
        SELECT (i * 5) % 8 + 1, date('2026-01-01', '+' || (i * 3) || ' days'),
               CASE WHEN i % 11 = 0 THEN 'cancelled' WHEN i % 4 = 0 THEN 'open' ELSE 'shipped' END
        FROM n;
    INSERT INTO order_items (order_id, product_id, quantity)
        SELECT id, id % 6 + 1, id % 3 + 1 FROM orders
        UNION ALL SELECT id, (id + 2) % 6 + 1, 1 FROM orders WHERE id % 2 = 0;

    CREATE TABLE events (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, happened_at TEXT NOT NULL);
    WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1200)
    INSERT INTO events (kind, happened_at)
        SELECT CASE i % 3 WHEN 0 THEN 'login' WHEN 1 THEN 'search' ELSE 'checkout' END,
               datetime('2026-03-01', '+' || (i * 7) || ' minutes')
        FROM n;
";

const BEST_CUSTOMERS: &str = "-- Best customers this year
SELECT c.name, c.country, COUNT(*) AS orders, ROUND(SUM(t.total), 2) AS spent
FROM customers c
JOIN order_totals t ON t.customer_id = c.id
JOIN orders o ON o.id = t.id
WHERE o.status <> 'cancelled'
GROUP BY c.id
ORDER BY spent DESC;
";

const MISSING_TABLE: &str = "SELECT id, carrier\nFROM shipments\nWHERE delivered_at IS NULL;\n";

/// More rows than one page (500), so the results show "more available".
const ALL_EVENTS: &str = "SELECT * FROM events;\n";

const SCRIPT: &str = "SELECT count(*) AS customers FROM customers;
SELECT status, count(*) AS orders FROM orders GROUP BY status;
UPDATE orders SET status = 'shipped' WHERE id = 4;
";

const DELETE_EVERYTHING: &str = "DELETE FROM order_items;\n";

struct Scene {
    name: &'static str,
    data_sources: FixtureSources,
    /// Drives a freshly started app into the state to capture.
    play: fn(&mut Harness<'static, DbDeskApp>, &Fixture),
}

/// Which data sources a scene's app starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureSources {
    /// None at all, as on first launch.
    None,
    /// The `shop` SQLite database, plus `warehouse` on PostgreSQL when one is configured.
    Shop,
}

const SCENES: &[Scene] = &[
    Scene {
        name: "first-launch",
        data_sources: FixtureSources::None,
        play: settle,
    },
    Scene {
        name: "welcome",
        data_sources: FixtureSources::Shop,
        play: settle,
    },
    Scene {
        name: "data-source-dialog",
        data_sources: FixtureSources::Shop,
        play: data_source_dialog,
    },
    Scene {
        name: "explorer-menu",
        data_sources: FixtureSources::Shop,
        play: explorer_menu,
    },
    Scene {
        name: "explorer-filter",
        data_sources: FixtureSources::Shop,
        play: explorer_filter,
    },
    Scene {
        name: "definition",
        data_sources: FixtureSources::Shop,
        play: definition,
    },
    Scene {
        name: "workspace",
        data_sources: FixtureSources::Shop,
        play: workspace,
    },
    Scene {
        name: "results-paging",
        data_sources: FixtureSources::Shop,
        play: results_paging,
    },
    Scene {
        name: "multiple-statements",
        data_sources: FixtureSources::Shop,
        play: multiple_statements,
    },
    Scene {
        name: "console-error",
        data_sources: FixtureSources::Shop,
        play: console_error,
    },
    Scene {
        name: "run-anyway",
        data_sources: FixtureSources::Shop,
        play: run_anyway,
    },
];

#[test]
#[ignore = "needs a GPU adapter and rewrites the screenshots; run with --ignored"]
fn regenerate_screenshots() {
    let out_dir = output_dir();
    std::fs::create_dir_all(&out_dir).unwrap();
    let themes = themes();
    for scene in SCENES {
        for &theme in &themes {
            let fixture = Fixture::new(scene.data_sources);
            let mut harness = fixture.start(theme);
            (scene.play)(&mut harness, &fixture);
            // Take the pointer away, so no cursor or hover highlight ends up in the image.
            harness.event(egui::Event::PointerGone);
            harness.run_steps(SETTLE_STEPS);
            let file_name = match themes.as_slice() {
                [_] => format!("{}.png", scene.name),
                _ => format!("{}-{}.png", scene.name, theme_name(theme)),
            };
            let path = out_dir.join(file_name);
            harness.render().unwrap().save(&path).unwrap();
            println!("wrote {}", path.display());
        }
    }
}

/// The manual uses the light theme, which also prints well. Design reviews ask for both.
fn themes() -> Vec<Theme> {
    let Ok(names) = std::env::var("DBDESK_SCREENSHOTS_THEMES") else {
        return vec![Theme::Light];
    };
    names
        .split(',')
        .map(|name| match name.trim() {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            other => panic!("unknown theme {other:?} in DBDESK_SCREENSHOTS_THEMES"),
        })
        .collect()
}

fn output_dir() -> PathBuf {
    std::env::var_os("DBDESK_SCREENSHOTS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../book/src/images"),
        PathBuf::from,
    )
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

/// A temporary directory with the fixture database and a data source file pointing at it.
struct Fixture {
    dir: tempfile::TempDir,
    postgres: Option<(DataSource, Password)>,
    sources: FixtureSources,
}

impl Fixture {
    fn new(sources: FixtureSources) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if sources == FixtureSources::None {
            return Self {
                dir,
                postgres: None,
                sources,
            };
        }
        let shop = dir.path().join("shop.db");
        rusqlite::Connection::open(&shop)
            .unwrap()
            .execute_batch(SHOP_SCHEMA)
            .unwrap();
        let postgres = std::env::var("DBDESK_TEST_POSTGRES_URL")
            .ok()
            .map(|url| postgres_data_source(&url));
        let mut data_sources = vec![DataSource {
            id: DataSourceId::generate(),
            name: "shop".to_owned(),
            color: Some(SourceColor::Orange),
            config: ConnectionConfig::Sqlite(SqliteConfig {
                path: shop,
                read_only: false,
            }),
        }];
        data_sources.extend(postgres.iter().map(|(data_source, _)| data_source.clone()));
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(&data_sources)
            .unwrap();
        Self {
            dir,
            postgres,
            sources,
        }
    }

    fn start(&self, theme: Theme) -> Harness<'static, DbDeskApp> {
        let builder = HarnessBuilder::default()
            .with_size(WINDOW_SIZE)
            .with_pixels_per_point(PIXELS_PER_POINT)
            .with_theme(theme)
            .wgpu();
        let secrets = match &self.postgres {
            Some((data_source, password)) => {
                MemorySecrets::with_password(data_source.id, password.clone())
            }
            None => MemorySecrets::default(),
        };
        let mut harness = start_app(builder, self.dir.path(), log_feed(), secrets);
        let ready = match self.sources {
            FixtureSources::None => "No data sources yet",
            FixtureSources::Shop => "Expand shop",
        };
        step_until_visible(&mut harness, ready);
        harness
    }
}

/// Parses `postgres://user:password@host:port/database`, the form the tests' URL takes.
fn postgres_data_source(url: &str) -> (DataSource, Password) {
    let rest = url.strip_prefix("postgres://").unwrap();
    let (credentials, rest) = rest.split_once('@').unwrap();
    let (user, password) = credentials.split_once(':').unwrap();
    let (address, database) = rest.split_once('/').unwrap();
    let (host, port) = address.split_once(':').unwrap();
    let data_source = DataSource {
        id: DataSourceId::generate(),
        name: "warehouse".to_owned(),
        color: Some(SourceColor::Red),
        config: ConnectionConfig::Postgres(PostgresConfig {
            host: host.to_owned(),
            port: port.parse().unwrap(),
            database: database.to_owned(),
            user: user.to_owned(),
        }),
    };
    (data_source, Password::new(password))
}

/// A log feed that also receives what the engine logs on its own threads.
///
/// A thread-local subscriber would only see the UI thread, so one global subscriber is
/// installed for the whole run, and each app's log console layer is swapped into it.
fn log_feed() -> log_console::LogFeed {
    type Handle = reload::Handle<Option<LogConsoleLayer>, tracing_subscriber::Registry>;
    static HANDLE: OnceLock<Handle> = OnceLock::new();

    let (layer, feed) = log_console::pipeline();
    let handle = HANDLE.get_or_init(|| {
        let (reloadable, handle) = reload::Layer::new(None);
        let subscriber =
            tracing_subscriber::registry().with(reloadable.with_filter(telemetry::targets()));
        tracing::subscriber::set_global_default(subscriber).unwrap();
        handle
    });
    handle.reload(Some(layer)).unwrap();
    feed
}

fn expand(harness: &mut Harness<'static, DbDeskApp>, labels: &[&str]) {
    for label in labels {
        let expander = format!("Expand {label}");
        step_until_visible(harness, &expander);
        harness.get_by_label(&expander).click();
        harness.run_ok();
    }
}

fn open_console(harness: &mut Harness<'static, DbDeskApp>, data_source: &str, sql: &str) {
    harness.get_by_label(data_source).click_secondary();
    harness.run_ok();
    // The Welcome tab has a card of the same name; the context menu comes first.
    harness
        .get_all_by_label("New query console")
        .next()
        .unwrap()
        .click();
    harness.run_ok();
    let editor = harness.get_by_role(Role::MultilineTextInput);
    editor.focus();
    harness.run_ok();
    harness.get_by_role(Role::MultilineTextInput).type_text(sql);
    harness.run_ok();
}

fn run_all(harness: &mut Harness<'static, DbDeskApp>, finished: &str) {
    harness.get_by_label("Run all").click();
    step_until_visible(harness, finished);
}

fn settle(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    harness.run_ok();
}

fn data_source_dialog(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    harness
        .get_all_by_label("New data source")
        .next()
        .unwrap()
        .click();
    harness.run_ok();
    harness.get_by_label("PostgreSQL").click();
    harness.run_ok();
}

fn explorer_menu(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    expand(harness, &["shop", "main", "tables", "orders", "columns"]);
    step_until_visible(harness, "customer_id");
    harness.get_by_label("orders").click_secondary();
    harness.run_ok();
}

fn explorer_filter(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    expand(harness, &["shop", "main", "tables", "views"]);
    let filter = By::new().predicate(|node| node.placeholder() == Some("Filter loaded nodes"));
    harness.get(filter).focus();
    harness.run_ok();
    let filter = By::new().predicate(|node| node.placeholder() == Some("Filter loaded nodes"));
    harness.get(filter).type_text("order");
    harness.run_ok();
}

fn definition(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    expand(harness, &["shop", "main", "views"]);
    step_until_visible(harness, "order_totals");
    harness.get_by_label("order_totals").click_secondary();
    harness.run_ok();
    harness.get_by_label("Show definition").click();
    // The definition is the value of a read-only text field, not a label.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let loaded = || {
        By::new().predicate(|node| {
            node.value()
                .is_some_and(|text| text.contains("CREATE VIEW"))
        })
    };
    while harness.query_all(loaded()).next().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the definition never loaded"
        );
        harness.step();
    }
}

fn workspace(harness: &mut Harness<'static, DbDeskApp>, fixture: &Fixture) {
    expand(harness, &["shop", "main", "tables", "views"]);
    if let Some((postgres, _)) = &fixture.postgres {
        expand(harness, &[postgres.name.as_str()]);
    }
    open_console(harness, "shop", BEST_CUSTOMERS);
    run_all(harness, "8 rows");
}

fn results_paging(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    open_console(harness, "shop", ALL_EVENTS);
    run_all(harness, "500 rows, more available");
}

fn multiple_statements(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    open_console(harness, "shop", SCRIPT);
    run_all(harness, "Done in");
}

fn console_error(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    open_console(harness, "shop", MISSING_TABLE);
    run_all(harness, "no such table");
}

fn run_anyway(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    open_console(harness, "shop", DELETE_EVERYTHING);
    run_all(harness, "Run anyway");
}
