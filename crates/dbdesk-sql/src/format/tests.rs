use super::*;
use crate::split::split_statements;

/// PostgreSQL statements, written the way people type them: odd spacing, comments in awkward
/// places, and every kind of literal the lexer knows.
const POSTGRES_CORPUS: &str = r#"
select id,name,email from users where active=true and created_at>=now()-interval '7 days' order by created_at desc limit 10;
SELECT a::int, b->>'key', c#>>'{a,b}', d @> '{"x":1}'::jsonb, e ? 'k', f ?| array['a','b'] FROM t WHERE g<>1 AND h!=2 AND i=-1;
select E'it\'s', B'1010', X'1F', U&'d\0061t\+000061', $$dollar; 'quoted' body$$, $tag$ nested $$ inside $tag$ from t;
select count(*) filter (where x > 1) over (partition by y order by z rows between unbounded preceding and current row) as running from t;
with recursive tree(id, parent) as (select id, parent from nodes where parent is null union all select n.id, n.parent from nodes n join tree on n.parent = tree.id) select * from tree;
insert into orders (customer_id, total) values (1, 10.5), (2, 20) on conflict (id) do update set total = excluded.total returning id;
update accounts a set balance = balance - 10, updated_at = now() from transfers t where t.account_id = a.id and t.amount between 1 and 100;
delete from sessions using users where sessions.user_id = users.id and users.disabled;
SELECT DISTINCT ON (customer_id) customer_id, created_at FROM orders ORDER BY customer_id, created_at DESC;
select case when a > 1 then 'big' when a = 1 then 'one' else 'small' end as size, case when b then 1 end from t;
select * from a left outer join b on a.id = b.a_id inner join c using (id) cross join lateral (select max(x) from d where d.id = a.id) m natural join e;
select id from t where id in (select id from u where u.flag) and exists(select 1 from v where v.id = t.id) or not x;
select a from t1 union all select a from t2 except select a from t3 order by 1 offset 5 fetch first 10 rows only;
select * from jobs where state = 'ready' for update skip locked;
select $1, $2::text, x from t where y = $3;
select a -- trailing comment
, b /* block
comment */ , c from t -- after from
where /* inline */ d = 1;
SELECT 1 - -1, 2 - - 2, 'a' || 'b', x ~* '^a', y !~~ 'b%', array[1,2][1:2], (a).b, t.*, -x;
create table if not exists public.orders (id bigint generated always as identity primary key, customer_id int not null references customers(id) on delete cascade, total numeric(10, 2) check (total >= 0), note text default 'none', unique (customer_id, note));
create view big_orders as select * from orders where total > 100;
create function add(a int, b int) returns int language sql immutable as $$ select a + b; -- body
$$;
do $$ begin raise notice 'hello  %', 1; perform pg_sleep(0); end $$;
copy orders (id, total) from stdin with (format csv, header true);
alter table orders add column shipped_at timestamptz, alter column total set not null;
grant select, insert on orders to app_user;
explain analyze select * from orders where id = 1;
create index concurrently orders_customer_idx on orders using btree (customer_id) where total > 0;
select 'a''b', "Quoted ""Name""", "select" from "Table";
values (1, 'a'), (2, 'b');
select * from t where a like 'x\%' escape '\' and b similar to '%(b|c)%';
begin; commit;
select json_build_object('a', 1, 'b', array_agg(x order by x)) from t group by y having count(*) > 1 and sum(z) < 10;
"#;

/// SQLite statements.
const SQLITE_CORPUS: &str = r#"
select [my col], `other col`, "third" from [my table] where x = ?1 and y = :name and z = @p and w = $v;
insert or replace into kv (k, v) values ('a', x'00ff'), ('b', null);
replace into kv values ('c', 1);
insert into t (a) select a from u where true on conflict(a) do nothing;
create table if not exists items (id integer primary key autoincrement, name text not null collate nocase, price real check (price > 0), created text default (datetime('now')));
create trigger items_touch after update on items for each row begin update items set created = datetime('now') where id = new.id; insert into log values (new.id); end;
pragma foreign_keys = on;
attach database 'other.db' as other;
select * from items indexed by items_name where name glob 'a*' limit 5, 10;
with t(x) as (values (1), (2)) select x from t;
select iif(a > 1, 'big', 'small'), a -> '$.b', a ->> '$.c', group_concat(b, ', ') from items group by a;
update items set price = price * 1.1 where id in (select id from items order by price limit 3) returning *;
delete from items where rowid = 1;
select a from t order by a nulls last;
vacuum;
explain query plan select * from items where id = 1;
create view v as select id, name from items;
select 1 /* c */ -- d
;
"#;

fn format_all(text: &str, dialect: Dialect, options: &FormatOptions) -> String {
    let ranges = split_statements(text, dialect);
    format_ranges(text, &ranges, dialect, options)
        .unwrap_or_else(|error| panic!("{error}:\n{text}"))
        .into_text()
}

fn pretty(sql: &str) -> String {
    format_all(sql, Dialect::Postgres, &FormatOptions::default())
}

#[test]
fn formatting_the_corpus_never_changes_a_token_and_is_stable() {
    for (corpus, dialect) in [
        (POSTGRES_CORPUS, Dialect::Postgres),
        (SQLITE_CORPUS, Dialect::Sqlite),
    ] {
        for case in [
            KeywordCase::Upper,
            KeywordCase::Lower,
            KeywordCase::Preserve,
        ] {
            for indent_width in [2, 4] {
                let options = FormatOptions {
                    keyword_case: case,
                    indent_width,
                };
                let once = format_all(corpus, dialect, &options);
                assert!(same_meaning(corpus, &once, dialect), "{once}");
                let twice = format_all(&once, dialect, &options);
                assert_eq!(once, twice, "formatting again changed it");
            }
        }
    }
}

#[test]
fn every_statement_formats_on_its_own_too() {
    for (corpus, dialect) in [
        (POSTGRES_CORPUS, Dialect::Postgres),
        (SQLITE_CORPUS, Dialect::Sqlite),
    ] {
        for range in split_statements(corpus, dialect) {
            let statement = &corpus[range];
            let formatted = format_all(statement, dialect, &FormatOptions::default());
            assert!(same_meaning(statement, &formatted, dialect), "{formatted}");
        }
    }
}

#[test]
fn lays_out_queries_clause_by_clause() {
    let cases = [
        (
            "select id,name from users u join orders o on o.user_id=u.id where u.active and o.total between 1 and 10 or u.vip order by id desc limit 5",
            "SELECT\n    id,\n    name\nFROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE u.active\n    AND o.total BETWEEN 1 AND 10\n    OR u.vip\nORDER BY id DESC\nLIMIT 5",
        ),
        ("select * from t", "SELECT *\nFROM t"),
        (
            "select a from t where id in (select id from u where x)",
            "SELECT a\nFROM t\nWHERE id IN (\n    SELECT id\n    FROM u\n    WHERE x\n)",
        ),
        (
            "with a as (select 1) select * from a",
            "WITH a AS (\n    SELECT 1\n)\nSELECT *\nFROM a",
        ),
        (
            "insert into t (a, b) values (1, 2), (3, 4)",
            "INSERT INTO t (a, b)\nVALUES\n    (1, 2),\n    (3, 4)",
        ),
        (
            "update t set a=1, b=2 where id=3",
            "UPDATE t\nSET\n    a = 1,\n    b = 2\nWHERE id = 3",
        ),
        (
            "select case when a then 1 when b then 2 else 3 end as x from t",
            "SELECT CASE\n    WHEN a THEN 1\n    WHEN b THEN 2\n    ELSE 3\nEND AS x\nFROM t",
        ),
        (
            "select id, case when a then 1 else 2 end as x, case when b then 1 end from t",
            "SELECT\n    id,\n    CASE\n        WHEN a THEN 1\n        ELSE 2\n    END AS x,\n    CASE WHEN b THEN 1 END\nFROM t",
        ),
        (
            "select a, -- the a\nb from t",
            "SELECT\n    a, -- the a\n    b\nFROM t",
        ),
        (
            "create view v as select a, b from t",
            "CREATE VIEW v AS\nSELECT\n    a,\n    b\nFROM t",
        ),
        (
            "create table t (id int primary key, name text not null)",
            "CREATE TABLE t (\n    id int PRIMARY KEY,\n    name TEXT NOT NULL\n)",
        ),
    ];
    for (sql, expected) in cases {
        assert_eq!(pretty(sql), expected, "{sql}");
    }
}

#[test]
fn keeps_line_breaks_of_other_statements_and_never_touches_bodies() {
    let sql = "create function f() returns int as $$\n  select  1;   -- keep\n$$   language   sql";
    assert_eq!(
        pretty(sql),
        "CREATE FUNCTION f() RETURNS int AS $$\n  select  1;   -- keep\n$$ LANGUAGE sql"
    );
    let block = "do $$\nbegin\n  raise notice 'a  b';\nend\n$$";
    assert_eq!(
        pretty(block),
        "DO $$\nbegin\n  raise notice 'a  b';\nend\n$$"
    );
}

#[test]
fn keyword_case_and_indent_width_are_options() {
    let sql = "Select a, b From t";
    let format = |keyword_case, indent_width| {
        format_all(
            sql,
            Dialect::Postgres,
            &FormatOptions {
                keyword_case,
                indent_width,
            },
        )
    };
    assert_eq!(format(KeywordCase::Upper, 2), "SELECT\n  a,\n  b\nFROM t");
    assert_eq!(
        format(KeywordCase::Lower, 4),
        "select\n    a,\n    b\nfrom t"
    );
    assert_eq!(format(KeywordCase::Preserve, 1), "Select\n a,\n b\nFrom t");
}

#[test]
fn continuation_lines_follow_the_statement_indentation_and_the_rest_is_kept() {
    let text = "-- first\n  select a, b from t;\n\n/* between */ select 1";
    let formatted = format_all(text, Dialect::Postgres, &FormatOptions::default());
    assert_eq!(
        formatted,
        "-- first\n  SELECT\n      a,\n      b\n  FROM t;\n\n/* between */ SELECT 1"
    );
}

#[test]
fn offsets_stay_on_their_token() {
    let text = "select a,b from t";
    let ranges = split_statements(text, Dialect::Postgres);
    let formatted =
        format_ranges(text, &ranges, Dialect::Postgres, &FormatOptions::default()).unwrap();
    let new = formatted.text();
    assert_eq!(new, "SELECT\n    a,\n    b\nFROM t");
    // Inside `from`, after `b`, at the start and at the end.
    let cases = [
        (
            text.find("from").unwrap() + 2,
            new.find("FROM").unwrap() + 2,
        ),
        (text.find('b').unwrap() + 1, new.find('b').unwrap() + 1),
        (0, 0),
        (text.len(), new.len()),
    ];
    for (old, expected) in cases {
        assert_eq!(formatted.map_offset(old), expected, "offset {old}");
    }
}

#[test]
fn the_safety_check_catches_changed_meaning() {
    let cases = [
        ("select a::int", "select a: :int"),
        ("select B'1010'", "select B '1010'"),
        ("select a - -1", "select a --1"),
        ("select 'a  b'", "select 'a b'"),
        ("select 1 -- note\n, 2", "select 1, 2"),
        ("select x from t", "select X from t"),
        ("select $1", "select $ 1"),
        ("select a b", "select ab"),
    ];
    for (before, after) in cases {
        assert!(
            !same_meaning(before, after, Dialect::Postgres),
            "{before:?} -> {after:?}"
        );
    }
    assert!(same_meaning(
        "select a=1,b from t",
        "SELECT a = 1, b\nFROM t",
        Dialect::Postgres
    ));
}
