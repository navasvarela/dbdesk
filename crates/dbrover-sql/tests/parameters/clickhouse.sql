-- The :name parameters dbrover finds in ClickHouse; see postgres.sql for the format.

-- expect: :id
SELECT * FROM t WHERE id = :id;

-- expect: none
SELECT {id:UInt64}, {name: String};

-- expect: none
SELECT 'a:b', x::Int32 FROM t;

-- expect: :d
SELECT toDate(:d);
