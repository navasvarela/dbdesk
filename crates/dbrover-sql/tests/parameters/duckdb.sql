-- The :name parameters dbrover finds in DuckDB; see postgres.sql for the format.

-- expect: :id
SELECT * FROM t WHERE id = :id;

-- expect: :n
SELECT :n::INTEGER;

-- expect: none
SELECT {'a': 1, 'b': x}, MAP {'k': v}, list[1:n], list[:n] FROM t;

-- expect: none
SELECT list_transform([1, 2], lambda x: x + 1), list_transform([1], y -> y);

-- expect: none
SELECT $1, ?, $name;

-- expect: :path
SELECT * FROM read_csv(:path);
