-- The :name parameters dbdesk finds in SQL Server; see postgres.sql for the format.

-- expect: :id
SELECT * FROM orders WHERE id = :id;

-- expect: none
SELECT geography::Point(1, 2, 4326), [a:b], N'x:y', 'z:w' FROM t;

-- expect: none
DECLARE @x int = 1;

-- expect: :d
SELECT DATEADD(day, 1, :d);

-- expect: :top
SELECT TOP (:top) * FROM t;
