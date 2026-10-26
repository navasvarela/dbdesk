//! Words that can't name a column or table without quotes in at least one dialect.
//!
//! Why not `sqlparser`'s keyword list: it holds every word any dialect gives meaning to,
//! 1,100 of them, `id`, `name` and `status` among them, and quoting those would make the
//! generated SQL hard to read. These are the words each database's documentation calls
//! reserved, as of the versions the test servers run:
//! - PostgreSQL 17: "reserved" and "reserved (can be function or type)" in its keyword
//!   appendix;
//! - SQL Server 2022: its reserved keywords;
//! - MySQL 8.4: the words marked (R) in its keyword list, which MariaDB's mostly match;
//! - SQLite 3: the keywords its parser has no fallback for;
//! - DuckDB 1.5: `duckdb_keywords()` of category `reserved` and `type_function`.
//!
//! ClickHouse lets almost any keyword name a column, so it adds none. A word missing here
//! makes a statement fail with the database's syntax error in the review panel; a word here
//! that needn't be only gets quotes.

const RESERVED: &str = "
    accessible add all alter analyse analyze and anti any array as asc asensitive asof
    asymmetric authorization autoincrement backup before begin between bigint binary blob both
    break browse bulk by call cascade case cast change char character check checkpoint close
    clustered coalesce collate collation column commit compute concurrently condition
    constraint contains containstable continue convert create cross cube cume_dist current
    current_catalog current_date current_role current_schema current_time current_timestamp
    current_user cursor database databases day_hour day_microsecond day_minute day_second dbcc
    deallocate dec decimal declare default deferrable delayed delete dense_rank deny desc
    describe deterministic disk distinct distinctrow distributed div do double drop dual dump
    each else elseif empty enclosed end errlvl escape escaped except exec execute exists exit
    explain external false fetch file fillfactor first_value float float4 float8 for force
    foreign freetext freetexttable freeze from full fulltext function generated get glob goto
    grant group grouping groups having high_priority holdlock hour_microsecond hour_minute
    hour_second identity identity_insert identitycol if ignore ilike in index infile initially
    inner inout insensitive insert int int1 int2 int3 int4 int8 integer intersect interval into
    io_after_gtids io_before_gtids is isnull iterate join json_table key keys kill lag lateral
    last_value lead leading leave left like limit linear lineno lines load localtime
    localtimestamp lock long longblob longtext loop low_priority map master_bind
    master_ssl_verify_server_cert match maxvalue mediumblob mediumint mediumtext merge
    middleint minute_microsecond minute_second mod modifies natural national no_write_to_binlog
    nocheck nonclustered not nothing notnull nth_value ntile null nullif numeric of off offset
    offsets on only open opendatasource openquery openrowset openxml optimize optimizer_costs
    option optionally or order out outer outfile over overlaps partition percent percent_rank
    pivot pivot_longer pivot_wider placing plan positional precision primary print proc
    procedure public purge qualify raiserror range rank read reads read_write readtext real
    reconfigure recursive references regexp release rename repeat replace replication require
    resignal restore restrict return returning revert revoke right rlike rollback row
    row_number rowcount rowguidcol rows rule save schema schemas second_microsecond
    securityaudit select semantickeyphrasetable semanticsimilaritydetailstable
    semanticsimilaritytable semi sensitive separator session_user set setuser show shutdown
    signal similar smallint some spatial specific sql sql_big_result sql_calc_found_rows
    sql_small_result sqlexception sqlstate sqlwarning ssl starting statistics stored
    straight_join struct summarize symmetric system system_user table tablesample terminated
    textsize then tinyblob tinyint tinytext to top trailing tran transaction trigger true
    truncate try_cast try_convert tsequal undo union unique unlock unpivot unsigned update
    updatetext usage use user using utc_date utc_time utc_timestamp values varbinary varchar
    varcharacter variadic varying verbose view virtual waitfor when where while window with
    within write writetext xor year_month zerofill
";

/// Whether `word` is reserved in at least one dialect, ignoring case.
pub(super) fn is_reserved(word: &str) -> bool {
    RESERVED
        .split_ascii_whitespace()
        .any(|reserved| reserved.eq_ignore_ascii_case(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_each_dialects_words_and_not_common_column_names() {
        for word in ["order", "user", "key", "Group", "top", "qualify", "lines"] {
            assert!(is_reserved(word), "{word}");
        }
        for word in [
            "id", "name", "status", "type", "value", "line", "date", "count",
        ] {
            assert!(!is_reserved(word), "{word}");
        }
    }
}
