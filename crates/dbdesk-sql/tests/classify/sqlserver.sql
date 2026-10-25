-- SQL Server statements and what dbdesk_sql::classify must call them. See
-- classify_corpus.rs. SQL Server isn't served over MCP yet; its corpus keeps the rules
-- ready for when it is.

-- Reading.
-- expect: read
SELECT 1;
-- expect: read
SELECT TOP 5 * FROM [dbo].[orders];
-- expect: read
SELECT * FROM erp.dbo.orders;
-- expect: read
WITH recent AS (SELECT * FROM orders) SELECT count(*) FROM recent;

-- Writing.
-- expect: write
INSERT INTO t (a) VALUES (1);
-- expect: write
UPDATE t SET a = 1 WHERE id = 2;
-- expect: write
DELETE FROM t WHERE id = 2;
-- expect: write
WITH ids AS (SELECT 1 AS id) DELETE FROM t WHERE id IN (SELECT id FROM ids);
-- expect: write
MERGE INTO t USING u ON t.id = u.id WHEN MATCHED THEN DELETE;
-- expect: write
TRUNCATE TABLE t;

-- Changing the schema.
-- expect: schema
CREATE TABLE t (id INT PRIMARY KEY);
-- expect: schema
SELECT * INTO archive FROM orders;
-- expect: schema
CREATE INDEX i ON t (a);
-- expect: schema
CREATE VIEW v AS SELECT 1;
-- expect: schema
ALTER TABLE t ADD b INT;
-- expect: schema
DROP TABLE t;
-- expect: schema
CREATE SCHEMA reporting;
-- A routine's body runs to the end of its batch.
-- expect: schema
CREATE PROCEDURE p AS SELECT 1;
GO

-- Transactions.
-- expect: transaction
BEGIN TRANSACTION;
-- expect: transaction
COMMIT;
-- expect: transaction
ROLLBACK;
-- expect: unparseable
SAVE TRANSACTION a;

-- Administration.
-- expect: admin
GRANT SELECT ON t TO someone;
-- expect: admin
REVOKE SELECT ON t FROM someone;
-- expect: unparseable
CREATE LOGIN intruder WITH PASSWORD = 'x';
-- expect: admin
CREATE DATABASE other;
-- expect: admin
DROP DATABASE other;
-- expect: admin
SET NOCOUNT ON;
-- expect: admin
EXEC sp_configure 'show advanced options', 1;
-- expect: admin
EXECUTE xp_cmdshell 'dir';
-- expect: admin
KILL 52;
-- expect: admin
USE other;
