//! Renders the app headlessly into PNG screenshots, so pictures of the UI are regenerated from
//! code instead of taken by hand.
//!
//! Ignored by default, because it needs a GPU adapter and rewrites files:
//!
//! ```text
//! cargo test -p dbdesk-app regenerate_screenshots -- --ignored
//! ```
//!
//! Images go to `DBDESK_SCREENSHOTS_DIR`, or `target/screenshots` without it. Every scene is
//! rendered in the light and the dark theme, against a SQLite fixture database. When
//! `DBDESK_TEST_POSTGRES_URL` is set, a PostgreSQL data source joins it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use dbdesk_core::{
    ConnectionConfig, DataSource, DataSourceId, Password, PostgresConfig, SqliteConfig,
};
use dbdesk_store::DataSourceFile;
use egui::accesskit::Role;
use egui::{Theme, Vec2};
use egui_kittest::kittest::Queryable;
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

struct Scene {
    name: &'static str,
    /// Drives a freshly started app into the state to capture.
    play: fn(&mut Harness<'static, DbDeskApp>, &Fixture),
}

const SCENES: &[Scene] = &[
    Scene {
        name: "welcome",
        play: welcome,
    },
    Scene {
        name: "workspace",
        play: workspace,
    },
    Scene {
        name: "console-error",
        play: console_error,
    },
    Scene {
        name: "data-source-dialog",
        play: data_source_dialog,
    },
];

#[test]
#[ignore = "needs a GPU adapter and rewrites the screenshots; run with --ignored"]
fn regenerate_screenshots() {
    let out_dir = output_dir();
    std::fs::create_dir_all(&out_dir).unwrap();
    for scene in SCENES {
        for theme in [Theme::Light, Theme::Dark] {
            let fixture = Fixture::new();
            let mut harness = fixture.start(theme);
            (scene.play)(&mut harness, &fixture);
            // Take the pointer away, so no cursor or hover highlight ends up in the image.
            harness.event(egui::Event::PointerGone);
            harness.run_steps(SETTLE_STEPS);
            let path = out_dir.join(format!("{}-{}.png", scene.name, theme_name(theme)));
            harness.render().unwrap().save(&path).unwrap();
            println!("wrote {}", path.display());
        }
    }
}

fn output_dir() -> PathBuf {
    std::env::var_os("DBDESK_SCREENSHOTS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots"),
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
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
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
            color: None,
            config: ConnectionConfig::Sqlite(SqliteConfig {
                path: shop,
                read_only: false,
            }),
        }];
        data_sources.extend(postgres.iter().map(|(data_source, _)| data_source.clone()));
        DataSourceFile::new(dir.path().join("data_sources.toml"))
            .save(&data_sources)
            .unwrap();
        Self { dir, postgres }
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
        step_until_visible(&mut harness, "Expand shop");
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
        color: None,
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
    harness.get_by_label("New query console").click();
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

fn welcome(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    harness.run_ok();
}

fn workspace(harness: &mut Harness<'static, DbDeskApp>, fixture: &Fixture) {
    expand(harness, &["shop", "main", "tables", "views"]);
    if let Some((postgres, _)) = &fixture.postgres {
        expand(harness, &[postgres.name.as_str()]);
    }
    open_console(harness, "shop", BEST_CUSTOMERS);
    run_all(harness, "8 rows");
}

fn console_error(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    open_console(harness, "shop", MISSING_TABLE);
    run_all(harness, "no such table");
}

fn data_source_dialog(harness: &mut Harness<'static, DbDeskApp>, _: &Fixture) {
    harness.get_by_label("+").click();
    harness.run_ok();
    harness.get_by_label("PostgreSQL").click();
    harness.run_ok();
}
