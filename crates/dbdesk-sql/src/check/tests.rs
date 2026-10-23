use super::*;
use crate::split::split_statements;

/// Valid PostgreSQL statements: none may get a syntax error. Each construct `sqlparser`
/// rejects is suppressed by the known-gaps list, which this corpus keeps honest.
const VALID_POSTGRES: &str = r#"
select id,name from users where active=true and created_at>=now()-interval '7 days' order by 1 desc limit 10;
SELECT a::int, b->>'key', c#>>'{a,b}', d @> '{"x":1}'::jsonb, e ? 'k', f ?| array['a','b'] FROM t WHERE g<>1 AND i=-1;
select E'it\'s', B'1010', X'1F', U&'d\0061t', $$dollar; 'quoted' body$$, $tag$ nested $$ inside $tag$ from t;
select count(*) filter (where x > 1) over (partition by y order by z rows between unbounded preceding and current row) from t;
with recursive tree(id, parent) as (select id, parent from nodes where parent is null union all select n.id, n.parent from nodes n join tree on n.parent = tree.id) select * from tree;
insert into orders (customer_id, total) values (1, 10.5), (2, 20) on conflict (id) do update set total = excluded.total returning id;
update accounts a set balance = balance - 10 from transfers t where t.account_id = a.id and t.amount between 1 and 100;
delete from sessions using users where sessions.user_id = users.id and users.disabled;
SELECT DISTINCT ON (customer_id) customer_id, created_at FROM orders ORDER BY customer_id, created_at DESC;
select * from a left outer join b on a.id = b.a_id inner join c using (id) cross join lateral (select max(x) from d where d.id = a.id) m;
select * from jobs where state = 'ready' for update skip locked;
select * from jobs for no key update;
select * from jobs for key share;
select $1, $2::text from t where y = $3;
select percentile_cont(0.5) within group (order by x) from t;
select x from generate_series(1, 10) as g(x);
select * from rows from (generate_series(1,2), generate_series(3,4)) as r(a,b);
select * from t1 join t2 using (id) as j;
select xmlelement(name foo);
select * from t where a between symmetric 1 and 2;
select a is distinct from b, a isnull from t;
select now() at time zone 'utc', extract(year from now()), substring('abc' from 2 for 1);
select a from t group by rollup (a), cube (b);
merge into t using s on t.id = s.id when matched then update set a = s.a when not matched then insert (id) values (s.id);
with moved as (delete from a returning *) insert into b select * from moved;
values (1, 'a'), (2, 'b');
select x::double precision, y::timestamp with time zone from t;
insert into t default values;
update t set a = default;
select 10 % 3, 2 ^ 3, ~1, 1 << 2;
"#;

const VALID_SQLITE: &str = r#"
select [my col], `other col`, "third" from [my table] where x = ?1 and y = :name and z = @p and w = $v;
insert or replace into kv (k, v) values ('a', x'00ff'), ('b', null);
replace into kv values ('c', 1);
insert into t (a) select a from u where true on conflict(a) do nothing;
select * from items indexed by items_name where name glob 'a*' limit 5, 10;
select * from items not indexed;
select a is not b, a is b, a is not null from t;
with t(x) as (values (1), (2)) select x from t;
select iif(a > 1, 'big', 'small'), a -> '$.b', a ->> '$.c', group_concat(b, ', ') from items group by a;
update items set price = price * 1.1 where id in (select id from items order by price limit 3) returning *;
update or ignore t set a = 1;
select * from t, json_each(t.j);
select * from pragma_table_info('t');
select a from t where a match 'x';
"#;

const VALID_MYSQL: &str = r#"
select `id`, name from `shop`.`users` where active = true and created_at >= now() - interval 7 day order by 1 desc limit 10;
select "double", 'it\'s', 'C:\\temp', x'1F', b'1010', _utf8mb4'text' collate utf8mb4_bin from t # trailing comment
;
select a from t limit 5, 10;
select a from t limit 10 offset 5;
select sql_calc_found_rows * from t;
select json_extract(doc, '$.a'), doc->'$.b', doc->>'$.c' from docs;
select count(*), group_concat(distinct name order by name separator ', ') from t group by k with rollup;
select * from a left join b on a.id = b.a_id straight_join c on c.id = b.c_id;
select * from t force index (idx_a) where a = 1;
select * from t use index (idx_a) ignore index (idx_b);
select * from t where a regexp '^x' and b rlike 'y' and c sounds like 'z';
select if(a > 1, 'big', 'small'), ifnull(b, 0), case when c then 1 else 2 end from t;
select @row := @row + 1, @@session.sql_mode from t;
select * from t where match (title, body) against ('db' in boolean mode);
select * from t for update nowait;
select * from t lock in share mode;
select * from t partition (p0);
with recursive n(i) as (select 1 union all select i + 1 from n where i < 5) select * from n;
select row_number() over w from t window w as (order by a);
select a div 2, a mod 3, a xor b, a <=> b from t;
select cast(a as unsigned), convert(b, char), convert(c using utf8mb4) from t;
select date_add(now(), interval 1 month), timestampdiff(day, a, b) from t;
insert into t (a, b) values (1, 'x'), (2, 'y') on duplicate key update b = values(b);
insert into t set a = 1, b = 2;
insert ignore into t (a) values (1);
insert into t (a) select a from u;
replace into kv (k, v) values ('a', 1);
replace into kv set k = 'b', v = 2;
update t set a = a + 1 where id = 1 order by id limit 1;
update low_priority ignore t set a = 1;
update a join b on a.id = b.a_id set a.x = b.x;
delete from t where id = 1 order by id limit 1;
delete a from a join b on a.id = b.a_id where b.gone;
delete quick ignore from t where id = 2;
values row(1, 'a'), row(2, 'b');
"#;

fn syntax_errors(corpus: &str, dialect: Dialect) -> Vec<(String, Problem)> {
    split_statements(corpus, dialect)
        .into_iter()
        .flat_map(|range| {
            let statement = &corpus[range];
            check_statement(statement, dialect)
                .into_iter()
                .filter(|problem| problem.source == ProblemSource::Syntax)
                .map(|problem| (statement.to_owned(), problem))
        })
        .collect()
}

#[test]
fn valid_statements_get_no_syntax_error() {
    assert_eq!(syntax_errors(VALID_POSTGRES, Dialect::Postgres), []);
    assert_eq!(syntax_errors(VALID_SQLITE, Dialect::Sqlite), []);
    assert_eq!(syntax_errors(VALID_MYSQL, Dialect::Mysql), []);
}

#[test]
fn marks_the_token_a_syntax_error_points_at() {
    let cases = [
        (
            "select 'a' from t where x = = 1",
            "=",
            "Syntax error: expected an expression, found =",
        ),
        (
            "select a from t where",
            "where",
            "Syntax error: expected an expression at the end of the statement",
        ),
        (
            "select 1,\n  é = = 2",
            "=",
            "Syntax error: expected an expression, found =",
        ),
    ];
    for (sql, marked, message) in cases {
        let problems = check_statement(sql, Dialect::Postgres);
        let error = problems
            .iter()
            .find(|problem| problem.severity == Severity::Error)
            .unwrap_or_else(|| panic!("no error in {sql:?}"));
        assert_eq!(&sql[error.range.clone()], marked, "{sql}");
        assert_eq!(error.message, message, "{sql}");
    }
    // Only data queries and changes are parsed: other statements are where the parser falls
    // short of the database.
    assert!(check_statement("selec 1", Dialect::Postgres).is_empty());
    assert!(check_statement("create tabel t ()", Dialect::Postgres).is_empty());
    assert!(check_statement("do $$ begin perform 1; end $$", Dialect::Postgres).is_empty());
}

fn lints(sql: &str, dialect: Dialect) -> Vec<(LintRule, &str)> {
    check_statement(sql, dialect)
        .into_iter()
        .filter_map(|problem| match problem.source {
            ProblemSource::Lint(rule) => Some((rule, &sql[problem.range])),
            _ => None,
        })
        .collect()
}

/// A statement, its dialect, and the rules it should break with the text each marks.
type LintCase = (&'static str, Dialect, &'static [(LintRule, &'static str)]);

#[test]
fn lint_rules_flag_what_they_describe_and_nothing_else() {
    use LintRule::*;
    let cases: [LintCase; 10] = [
        (
            "select * from t where a = null",
            Dialect::Postgres,
            &[(NullComparison, "= null")],
        ),
        (
            "select * from t where NULL <> a",
            Dialect::Postgres,
            &[(NullComparison, "NULL <>")],
        ),
        (
            "select * from t where a != null",
            Dialect::Sqlite,
            &[(NullComparison, "!= null")],
        ),
        (
            "select * from t where a is null and b is not null",
            Dialect::Postgres,
            &[],
        ),
        (
            "update t set a = null, b = (select null) where c = 1",
            Dialect::Postgres,
            &[],
        ),
        (
            "update t set a = null where b = null",
            Dialect::Postgres,
            &[(NullComparison, "= null")],
        ),
        (
            "update t set a = 1",
            Dialect::Postgres,
            &[(UpdateWithoutWhere, "update")],
        ),
        (
            "DELETE FROM t",
            Dialect::Sqlite,
            &[(DeleteWithoutWhere, "DELETE")],
        ),
        (
            "insert into t select distinct * from u",
            Dialect::Postgres,
            &[(SelectStarInInsert, "*")],
        ),
        (
            "insert into t (a) select a from (select * from u) s",
            Dialect::Postgres,
            &[],
        ),
    ];
    for (sql, dialect, expected) in cases {
        assert_eq!(lints(sql, dialect), expected, "{sql}");
    }
}

#[test]
fn rule_ids_read_back() {
    for rule in LintRule::ALL {
        assert_eq!(LintRule::from_id(rule.id()), Some(rule));
    }
    assert_eq!(LintRule::from_id("no-such-rule"), None);
}

#[test]
fn still_reports_mysql_syntax_errors_around_its_gaps() {
    for sql in [
        "select a from t where",
        "select a, from t",
        "update t set where id = 1",
    ] {
        let problems = check_statement(sql, Dialect::Mysql);
        assert!(
            problems
                .iter()
                .any(|problem| problem.source == ProblemSource::Syntax),
            "{sql}: {problems:?}"
        );
    }
}
