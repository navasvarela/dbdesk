-- The :name parameters dbdesk finds in PostgreSQL, statement by statement: each `-- expect:`
-- line lists them in order, repeats included, or says `none`. Comments like this one, and
-- strings, never hold any.

-- expect: :id
SELECT * FROM orders WHERE id = :id;

-- expect: :since :limit
SELECT * FROM orders WHERE created_at >= :since ORDER BY id LIMIT :limit;

-- expect: :a :b :a
SELECT :a + :b, coalesce(:a, 0);

-- expect: :lo :hi :hi
SELECT * FROM t WHERE x BETWEEN:lo AND:hi OR y=:hi;

-- expect: :d :d
SELECT :d::date, :d :: text;

-- expect: none
SELECT '2026-01-01 10:00:00'::timestamp, 'a:b', "col:x", x::int, y :: text FROM t;

-- expect: none
SELECT (arr)[1:2], arr[:n], arr[lo:hi], arr[1][2:3] FROM t;

-- expect: none
SELECT f(a := 1), g(b => 2);

-- expect: :x
SELECT 1 -- :not_this
+ :x /* :nor_this, /* nested */ :nor_that */;

-- expect: none
SELECT $$ :inside $$, $tag$ :x $tag$, E'it\'s :x', U&':x';

-- expect: none
SELECT :'psql_variable', :"psql_identifier";

-- expect: :p :p
INSERT INTO t (a, b) VALUES (:p, upper(:p)) RETURNING *;

-- expect: :_x1 :ünïcode
SELECT ARRAY[1, 2] || :_x1, :ünïcode;

-- expect: none
SELECT a:b, 1:c, 'x':d, "y":e, (z):f FROM t;
