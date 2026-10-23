use dbdesk_core::conformance::{self, UnquotedNames};
use dbdesk_core::{FolderKind, NodeKind, ServerConfig, TlsMode};

use super::*;

/// Point the tests at servers, e.g. `mysql://dbdesk:dbdesk@localhost:55306/dbdesk` from the
/// repository's `docker-compose.yml` (see `test-servers.env`). Each test runs against every
/// server that is set and is skipped when none is, so `cargo test` works offline.
const TEST_URL_VARS: &[&str] = &["DBDESK_TEST_MYSQL_URL", "DBDESK_TEST_MARIADB_URL"];

/// `SLEEP()` alone in a statement returns 1 when killed instead of failing, so it sits in a
/// condition, where a kill makes the whole statement fail.
const LONG_RUNNING: &str = "SELECT 1 FROM DUAL WHERE SLEEP(60) = 0";

struct TestServer {
    variable: &'static str,
    config: MysqlConfig,
    password: Option<Password>,
}

fn test_servers() -> Vec<TestServer> {
    let servers: Vec<TestServer> = TEST_URL_VARS
        .iter()
        .filter_map(|variable| {
            let url = std::env::var(variable).ok()?;
            Some(parse_url(variable, &url))
        })
        .collect();
    if servers.is_empty() {
        eprintln!("skipping: none of {TEST_URL_VARS:?} is set");
    }
    servers
}

/// Reads `mysql://user:password@host:port/database`.
fn parse_url(variable: &'static str, url: &str) -> TestServer {
    let rest = url.strip_prefix("mysql://").expect("mysql:// URL");
    let (credentials, rest) = rest.split_once('@').expect("user:password@ in URL");
    let (user, password) = credentials.split_once(':').expect("password in URL");
    let (address, database) = rest.split_once('/').expect("/database in URL");
    let (host, port) = address.split_once(':').expect("host:port in URL");
    TestServer {
        variable,
        config: MysqlConfig {
            server: ServerConfig {
                host: host.to_owned(),
                port: port.parse().expect("numeric port"),
                database: database.to_owned(),
                user: user.to_owned(),
                tls: TlsMode::default(),
            },
        },
        password: Some(Password::new(password)),
    }
}

impl TestServer {
    async fn connect(&self) -> MysqlConnection {
        self.connect_with(TlsMode::default())
            .await
            .unwrap_or_else(|error| panic!("{}: {error:?}", self.variable))
    }

    async fn connect_with(&self, tls: TlsMode) -> Result<MysqlConnection, DriverError> {
        let mut config = self.config.clone();
        config.server.tls = tls;
        connect(&config, self.password.as_ref()).await
    }
}

/// One text value from the first row of `sql`.
async fn first_value(connection: &MysqlConnection, sql: &str) -> Option<String> {
    connection
        .conn
        .lock()
        .await
        .query_first::<mysql_async::Row, _>(sql)
        .await
        .unwrap()
        .and_then(|row| row.get_opt::<Option<String>, _>(row.len() - 1))
        .and_then(Result::ok)
        .flatten()
}

#[tokio::test]
async fn conforms_to_the_connection_lifecycle() {
    for server in test_servers() {
        let connection = server.connect().await;

        let product = connection.server_info().product.clone();
        assert!(
            ["MySQL", "MariaDB"].contains(&product.as_str()),
            "{product}"
        );
        conformance::check_lifecycle(Box::new(connection)).await;
    }
}

#[tokio::test]
async fn wrong_password_is_a_connect_error() {
    for server in test_servers() {
        let result = connect(&server.config, Some(&Password::new("definitely wrong"))).await;

        conformance::check_connect_failure(result);
    }
}

#[tokio::test]
async fn unreachable_server_is_a_connect_error() {
    let config = MysqlConfig {
        server: ServerConfig {
            host: "127.0.0.1".to_owned(),
            // Port 1 (tcpmux) is reserved and has no listener on any normal machine, so the
            // connection is refused immediately instead of timing out.
            port: 1,
            database: "mysql".to_owned(),
            user: "root".to_owned(),
            tls: TlsMode::Disable,
        },
    };

    conformance::check_connect_failure(connect(&config, None).await);
}

#[tokio::test]
async fn runs_pages_cancels_and_reports_error_positions() {
    for server in test_servers() {
        let mut connection = server.connect().await;
        let names = match first_value(&connection, "SELECT @@lower_case_table_names").await {
            Some(setting) if setting != "0" => UnquotedNames::IgnoreCase,
            _ => UnquotedNames::ExactCase,
        };

        conformance::check_execution(&mut connection).await;
        conformance::check_transactions(&mut connection).await;
        conformance::check_describe_table(&mut connection, names).await;
        conformance::check_validate(&mut connection).await;
        conformance::check_cancel(&mut connection, LONG_RUNNING).await;

        Box::new(connection).close().await.unwrap();
    }
}

/// One object of every kind the explorer shows. Run one statement at a time: a procedure's
/// body holds semicolons.
const FIXTURE: &[&str] = &[
    "DROP EVENT IF EXISTS dbdesk_cleanup",
    "DROP VIEW IF EXISTS dbdesk_big_orders",
    "DROP FUNCTION IF EXISTS dbdesk_add",
    "DROP PROCEDURE IF EXISTS dbdesk_touch",
    "DROP TABLE IF EXISTS dbdesk_orders",
    "CREATE TABLE dbdesk_orders (
        id INT AUTO_INCREMENT PRIMARY KEY,
        customer VARCHAR(50) NOT NULL,
        total DECIMAL(10, 2),
        CONSTRAINT dbdesk_total_positive CHECK (total >= 0),
        INDEX dbdesk_orders_customer (customer)
    )",
    "CREATE TRIGGER dbdesk_orders_touch BEFORE UPDATE ON dbdesk_orders
        FOR EACH ROW SET NEW.total = NEW.total",
    "CREATE VIEW dbdesk_big_orders AS SELECT id, total FROM dbdesk_orders WHERE total > 100",
    "CREATE FUNCTION dbdesk_add(a INT, b INT) RETURNS INT DETERMINISTIC RETURN a + b",
    "CREATE PROCEDURE dbdesk_touch() BEGIN SELECT 1; SELECT 2; END",
    "CREATE EVENT dbdesk_cleanup ON SCHEDULE EVERY 1 DAY
        DO DELETE FROM dbdesk_orders WHERE total < 0",
];

fn labels_under<'a>(nodes: &'a [CatalogNode], parent: &CatalogPath) -> Vec<&'a str> {
    nodes
        .iter()
        .filter(|node| node.path.parent().as_ref() == Some(parent))
        .map(|node| node.label.as_str())
        .collect()
}

fn folder(parent: &CatalogPath, folder: FolderKind) -> CatalogPath {
    parent.child(NodeKind::Folder(folder), folder.label())
}

#[tokio::test]
async fn walks_the_catalog_and_shows_definitions() {
    for server in test_servers() {
        let mut connection = server.connect().await;
        for statement in FIXTURE {
            connection
                .conn
                .lock()
                .await
                .query_drop(*statement)
                .await
                .unwrap_or_else(|error| panic!("{}: {statement}: {error}", server.variable));
        }

        let nodes = conformance::walk_catalog(&mut connection, 5).await;
        conformance::check_invalid_path(&mut connection).await;

        let schema = CatalogPath::root().child(NodeKind::Schema, &server.config.server.database);
        let root = labels_under(&nodes, &CatalogPath::root());
        assert!(
            root.contains(&server.config.server.database.as_str()),
            "{root:?}"
        );
        let information_schema = nodes
            .iter()
            .find(|node| {
                node.path == CatalogPath::root().child(NodeKind::Schema, "information_schema")
            })
            .expect("information_schema is listed");
        assert!(information_schema.system);

        let orders = folder(&schema, FolderKind::Tables).child(NodeKind::Table, "dbdesk_orders");
        let contains: [(CatalogPath, &str); 6] = [
            (folder(&schema, FolderKind::Tables), "dbdesk_orders"),
            (folder(&schema, FolderKind::Views), "dbdesk_big_orders"),
            (folder(&schema, FolderKind::Functions), "dbdesk_add"),
            (folder(&schema, FolderKind::Procedures), "dbdesk_touch"),
            (folder(&schema, FolderKind::Events), "dbdesk_cleanup"),
            (folder(&orders, FolderKind::Triggers), "dbdesk_orders_touch"),
        ];
        for (parent, label) in contains {
            assert!(
                labels_under(&nodes, &parent).contains(&label),
                "{label} under {parent}"
            );
        }
        let exactly: [(CatalogPath, &[&str]); 3] = [
            (
                folder(&orders, FolderKind::Columns),
                &["id", "customer", "total"],
            ),
            (
                folder(&orders, FolderKind::Indexes),
                &["dbdesk_orders_customer", "PRIMARY"],
            ),
            (
                folder(&orders, FolderKind::Constraints),
                &["dbdesk_total_positive", "PRIMARY"],
            ),
        ];
        for (parent, labels) in exactly {
            assert_eq!(labels_under(&nodes, &parent), labels, "under {parent}");
        }

        let definitions = [
            (orders.clone(), "CREATE TABLE `dbdesk_orders`"),
            (
                folder(&schema, FolderKind::Views).child(NodeKind::View, "dbdesk_big_orders"),
                "`dbdesk_big_orders`",
            ),
            (
                folder(&schema, FolderKind::Functions).child(NodeKind::Function, "dbdesk_add"),
                "a + b",
            ),
            (
                folder(&schema, FolderKind::Procedures).child(NodeKind::Procedure, "dbdesk_touch"),
                "SELECT 2",
            ),
            (
                folder(&schema, FolderKind::Events).child(NodeKind::Event, "dbdesk_cleanup"),
                "DELETE FROM dbdesk_orders",
            ),
            (
                folder(&orders, FolderKind::Triggers)
                    .child(NodeKind::Trigger, "dbdesk_orders_touch"),
                "SET NEW.total",
            ),
        ];
        for (path, expected) in definitions {
            let definition = connection.definition(&path).await.unwrap();
            assert!(definition.contains(expected), "{path}: {definition}");
        }

        Box::new(connection).close().await.unwrap();
    }
}

/// Both test servers make a self-signed certificate on first start, as MySQL 8 and MariaDB
/// 11.4 do by default. Connecting with TLS also proves `mysql_async`'s rustls connector finds
/// a crypto provider: it takes rustls's process-wide one, which only works while ring is the
/// only provider compiled in.
#[tokio::test]
async fn encrypts_as_the_tls_mode_says() {
    for server in test_servers() {
        for (tls, encrypted) in [
            (TlsMode::Disable, false),
            (TlsMode::Prefer, true),
            (TlsMode::Require, true),
        ] {
            let connection = server
                .connect_with(tls.clone())
                .await
                .unwrap_or_else(|error| panic!("{} {tls:?}: {error:?}", server.variable));

            let cipher = first_value(&connection, "SHOW SESSION STATUS LIKE 'Ssl_cipher'").await;
            assert_eq!(
                cipher.is_some_and(|cipher| !cipher.is_empty()),
                encrypted,
                "{} {tls:?}",
                server.variable
            );
            Box::new(connection).close().await.unwrap();
        }

        // The certificate isn't signed by a public authority.
        let unverified = server
            .connect_with(TlsMode::VerifyFull { ca_file: None })
            .await;
        conformance::check_connect_failure(unverified);
    }
}

#[test]
fn tells_mysql_and_mariadb_apart_by_version() {
    let cases = [
        ("8.4.3", "MySQL", "8.4.3"),
        ("11.4.4-MariaDB-ubu2404", "MariaDB", "11.4.4"),
    ];
    for (version, product, number) in cases {
        let info = server_info(version.to_owned());
        assert_eq!(
            (info.product.as_str(), info.version.as_str()),
            (product, number)
        );
    }
}
