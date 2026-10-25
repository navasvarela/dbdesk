-- SQLite statements and what dbdesk_sql::classify must call them. See classify_corpus.rs.

-- Reading.
-- expect: read
SELECT 1;
-- expect: read
SELECT [order] FROM "t";
-- expect: read
SELECT * FROM main.t;
-- expect: read
WITH recent AS (SELECT * FROM orders) SELECT count(*) FROM recent;
-- expect: read
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5) SELECT * FROM n;
-- expect: read
EXPLAIN QUERY PLAN SELECT * FROM t;
-- expect: read
VALUES (1, 2);

-- Writing.
-- expect: write
INSERT INTO t (a) VALUES (1);
-- expect: write
INSERT OR REPLACE INTO t (a) VALUES (1);
-- expect: write
REPLACE INTO t (a) VALUES (1);
-- expect: write
INSERT INTO t (a) VALUES (1) ON CONFLICT (a) DO UPDATE SET a = excluded.a;
-- expect: write
UPDATE t SET a = 1 WHERE id = 2;
-- expect: write
DELETE FROM t;
-- expect: write
WITH gone AS (SELECT 1) DELETE FROM t WHERE id IN (SELECT * FROM gone);
-- expect: write
WITH ids AS (SELECT 1) UPDATE t SET a = 2 WHERE id IN (SELECT * FROM ids);
-- expect: write
WITH ids AS (SELECT 1) INSERT INTO t SELECT * FROM ids;

-- Changing the schema.
-- expect: schema
CREATE TABLE t (id INTEGER PRIMARY KEY);
-- expect: schema
CREATE TABLE IF NOT EXISTS copy AS SELECT * FROM t;
-- expect: schema
CREATE INDEX i ON t (a);
-- expect: schema
CREATE VIEW v AS SELECT 1;
-- expect: schema
CREATE TRIGGER tr AFTER INSERT ON t BEGIN DELETE FROM u; END;
-- expect: schema
CREATE VIRTUAL TABLE docs USING fts5(body);
-- expect: schema
ALTER TABLE t ADD COLUMN b TEXT;
-- expect: schema
ALTER TABLE t RENAME TO u;
-- expect: schema
DROP TABLE t;
-- expect: schema
DROP INDEX i;

-- Transactions.
-- expect: transaction
BEGIN;
-- expect: transaction
BEGIN IMMEDIATE;
-- expect: transaction
COMMIT;
-- expect: transaction
ROLLBACK;
-- expect: transaction
SAVEPOINT a;

-- Administration.
-- expect: admin
PRAGMA journal_mode = 'DELETE';
-- expect: unparseable
PRAGMA journal_mode = DELETE;
-- expect: unparseable
PRAGMA table_info(t);
-- expect: admin
PRAGMA query_only = 0;
-- expect: admin
ATTACH DATABASE '/tmp/other.db' AS other;
-- expect: unparseable
DETACH DATABASE other;
-- expect: admin
VACUUM;
-- expect: admin
SELECT load_extension('/tmp/evil.so');
