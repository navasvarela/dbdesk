-- The :name parameters dbdesk finds in MySQL and MariaDB; see postgres.sql for the format.

-- expect: :id
SELECT * FROM orders WHERE id=:id;

-- expect: :a :b
SELECT * FROM t WHERE id IN (:a,:b);

-- expect: none
SET @total := 0;

-- expect: none
SELECT @row := @row + 1 FROM t;

-- expect: none
SELECT 'x:y', "a:b", `c:d`, '10:30' FROM t;

-- expect: none
SELECT 1 # :hash_comment
;

-- expect: :p
SELECT 'it\'s', :p;

-- expect: :limit
SELECT * FROM t LIMIT :limit;
