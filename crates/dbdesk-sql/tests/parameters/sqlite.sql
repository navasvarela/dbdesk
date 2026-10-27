-- The :name parameters dbdesk finds in SQLite; see postgres.sql for the format.

-- expect: :id
SELECT * FROM t WHERE id = :id;

-- expect: :x :x
SELECT :x, :x;

-- expect: none
SELECT "a:b", [c:d], `e:f`, 'g:h' FROM t;

-- expect: none
SELECT ?1, ?, @x, $y FROM t;

-- expect: :since :since
SELECT * FROM t WHERE created >= :since AND created < date(:since, '+1 day');
