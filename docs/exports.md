# Exports and your own formats

Status: **in progress** (2026-10-29); W7b to W7i are built. This is the design of W7 and W8 in
[A well-rounded app](well-rounded.md#w7--exports); the [roadmap](roadmap.md#well-rounded-track)
tracks them.

Getting rows out of the database is half of what a database tool is for: into a spreadsheet for
someone who asked, into a pull request as a Markdown table, into a test fixture as `INSERT`s, into
a data pipeline as Parquet. DbRover writes CSV and JSON, with no options, from the grid only. W7
adds the formats people ask for, their options, and every place an export should start. W8 lets
the user define formats of their own in plain files, as H1 and H2 did for actions.

## What exists today

- **The grid's Export menu** has **Loaded rows as CSV…** and **as JSON…** (the rows as sorted and
  filtered in the grid), and **All rows as CSV…** and **as JSON…**, which run the statement again
  on the console's connection and stream every page to the file (W5d). An export can be
  cancelled, is recorded in the history, and is written beside its final name and renamed when
  complete, so a failed one never leaves half a file.
- **CSV has no options.** Commas, a header line, CRLF, quotes only where needed, `NULL` as an
  empty field, UTF-8 without a BOM: RFC 4180, which Excel opens with mangled accents.
- **JSON** is an array of objects with every value a string (ADR 0004), keys in column order, a
  repeated column name suffixed `_2`.
- **Copying:** ⌘C and **Copy selection** copy the selected cells as TSV, quoted the way
  spreadsheets read them back, with `NULL` written out. **Copy row** joins a row with tabs but
  quotes nothing, so a value with a tab or a line break pastes into the wrong cells. **There is
  no way to copy the column headers**, an M4 follow-up still open.
- **Elsewhere:** CSV import (comma, semicolon or tab), the history's CSV and JSON export (W4f),
  the data sources' TOML export (W1), and H2's command actions, which get the selected cells on
  stdin as JSON, CSV or TSV.
- **Missing:** exporting a table from the explorer, running a query straight to a file, copying
  in any other format, and any of it through MCP. DbRover has no command palette; menus and the
  keymap are where commands live.
- **Types:** results are text (ADR 0004). SQLite, MySQL and MariaDB, SQL Server, DuckDB and
  ClickHouse report each column's declared type name; PostgreSQL's simple query protocol reports
  none.

## What others do

From each tool's current documentation (October 2026):

- **DataGrip** has *data extractors*: CSV, TSV and pipe-separated with configurable separators,
  quoting, `NULL` text and header; SQL `INSERT` and `UPDATE` statements; and bundled scripts for
  Excel, HTML, JSON, Markdown, XML and more. Users write their own extractors in Groovy or
  JavaScript, getting the rows, the columns and a value formatter. An extractor is chosen once
  and then used by both **Export to File** and **Copy to Clipboard**. **Execute to File** runs a
  query straight to a file without loading the grid, and the Database Explorer exports whole
  tables. ([Data extractors](https://www.jetbrains.com/help/datagrip/data-extractors.html),
  [Export data](https://www.jetbrains.com/help/datagrip/export-data.html))
- **DBeaver's Data Transfer** wizard writes CSV, HTML, JSON, Markdown, SQL, TXT and XML, plus XLSX
  and Parquet in its paid editions. CSV has a delimiter, row delimiter, header position, quoting
  rules and `NULL` text; the output step has the encoding, **Insert BOM** (off by default), a
  file name pattern (`${table}_${timestamp}`), splitting by size and compression. **Export from
  query** reads from the database rather than the grid, and **Advanced copy** copies as any of
  those formats. ([Data export](https://dbeaver.com/docs/dbeaver/Data-export/))
- **TablePlus** exports CSV, JSON, SQL and Excel, and **Copy Rows As** offers CSV, CSV with
  header, JSON, HTML, Markdown and `INSERT`.
  ([Import and export](https://tableplus.com/docs/gui-tools/import-and-export))
- **Excel** opens a UTF-8 CSV correctly only if it starts with a byte order mark
  ([Microsoft](https://support.microsoft.com/en-us/office/opening-csv-utf-8-files-correctly-in-excel-8a935af5-3416-4edd-ba7e-3dfd2bc4a032)),
  and splits fields on the system's list separator, which is a semicolon in many European
  locales. The `sep=;` first line some tools write is undocumented by Microsoft and is read as
  data by most other programs. Google Sheets detects the separator; Numbers asks.

## Goals

- **The formats people ask for, with the options that matter:** CSV that opens correctly in
  Excel, a real `.xlsx`, Markdown for pull requests and chat, SQL to move rows between databases,
  JSON Lines and Parquet for pipelines, HTML for a page or an email.
- **One choice of format everywhere:** the same formats and options for a file, the clipboard,
  a whole table, a query run straight to a file and MCP's answers.
- **Any size:** every file export streams; no format holds all the rows.
- **Formats of your own, in plain files** (W8): named presets, templates and programs in
  `exports.toml`, beside `actions.toml`, read and reloaded the same way.

Not goals: reports with grouping, totals, charts, page layout or schedules. DbRover isn't a BI
tool. Grouping and totals are SQL's job (`GROUP BY`, `ROLLUP`, window functions), parameters are
the `:name` parameters consoles already ask for, and a program in W8 can turn rows into anything
else, a PDF included.

## What stays the same

- **Results are text** (ADR 0004). Formats that have types (`.xlsx`, Parquet, SQL literals) take
  them from the column's declared type where the driver reports one, and otherwise from the text
  itself, by rules written down below and tested. Values are never reformatted: a date is written
  as the database printed it, or as that same date in a typed format. Formatting dates and
  numbers differently is done in SQL (`to_char`, `FORMAT`, `strftime`).
- **Files are written beside their final name and renamed when complete**, cancelled from the
  toolbar, and recorded in the history as **Export**, as W5d's are.
- **Safety levels:** an export reads, so it runs everywhere, in the database's read-only mode on a
  read-only data source, as W5d's does. Exports write nothing to the database.

## Formats

In priority order. Each has the options listed and nothing more; every option has a default, so
**Export** with nothing changed writes a sensible file.

| Format | Extension | Options | File | Clipboard | MCP |
| --- | --- | --- | --- | --- | --- |
| CSV | `.csv` | separator, quoting, header, `NULL` text, line ending, BOM | ✓ | ✓ | ✓ |
| TSV | `.tsv` | header, `NULL` text | ✓ | ✓ (⌘C) | — |
| Excel | `.xlsx` | header, sheet name, freeze header, autofilter | ✓ | — | — |
| Markdown | `.md` | `NULL` text | ✓ | ✓ | ✓ |
| SQL `INSERT` | `.sql` | table, rows per statement | ✓ | ✓ | — |
| JSON | `.json` | (none) | ✓ | ✓ | its own, as today |
| JSON Lines | `.jsonl` | (none) | ✓ | ✓ | ✓ |
| HTML | `.html` | fragment or whole page, `NULL` text | ✓ | ✓ (as text) | — |
| SQL `UPDATE` | `.sql` | (table and key from the result) | ✓ | ✓ | — |
| Parquet | `.parquet` | compression | ✓ | — | — |

- **CSV:**
  - **Separator:** comma (default), semicolon, tab, pipe, or any single character.
  - **Quoting:** where needed (default), or every field.
  - **Header line:** on (default) or off.
  - **`NULL` as:** an empty field (default), `NULL`, `\N`, or any text.
  - **Line ending:** CRLF (default, as RFC 4180 asks) or LF.
  - **Byte order mark:** off (default; most programs other than Excel read it as part of the
    first column's name), or on.
  - **Encoding:** always UTF-8. A BOM is what Excel needs; other encodings are left out until
    someone asks.
  - **CSV for Excel** is a preset in the format list: comma, CRLF, a BOM. Its description says
    that Excel in a European locale may want a semicolon instead. No `sep=` line.
- **TSV** is what ⌘C copies, as a file: tabs, the header on, `NULL` written as `NULL`, fields
  quoted the way spreadsheets read them back.
- **Excel (`.xlsx`)** is written with
  [`rust_xlsxwriter`](https://crates.io/crates/rust_xlsxwriter) in its constant-memory mode, a
  row at a time. A bold header row, frozen and with an autofilter (both on by default), and column
  widths guessed from the first page of rows. Each cell is typed:
  - A column whose declared type is a number, a boolean, a date or a timestamp gets numbers,
    booleans or dates; one declared as text, or of a type DbRover doesn't map, stays text.
  - A column with no declared type (PostgreSQL) is typed cell by cell: a plain decimal number
    becomes a number, unless it has a leading zero (`007`, a code rather than a quantity) or more
    than 15 significant digits (beyond what Excel's numbers hold exactly, as with long ids); an
    ISO date or timestamp becomes a date shown in ISO format; everything else is text.
  - A timestamp with a time zone or offset stays text: Excel has no time zones, and converting it
    would change what it says.
  - Excel holds 1,048,576 rows a sheet. Past that, rows go on in a new sheet (`Rows 2`), and the
    status says so.
- **Markdown** is a GitHub-flavoured table: `|` escaped, line breaks as `<br>`, `NULL` shown as
  `NULL` (as in the grid) unless changed.
- **SQL `INSERT`** statements for a table: the one the result came from when it came from one
  (the grid already knows), otherwise a name typed in the dialog, quoted for the data source's
  dialect. One row per statement by default, or up to 1,000 rows (SQL Server's limit for one
  `VALUES`). Built by `dbrover_sql::dml`'s `RowSql`, as the grid's edits and the CSV import are.
  Values are string literals, as the grid's edits send them, except in columns whose declared
  type is numeric, where a number is written bare.
- **JSON** is today's: an array of objects, values as strings. **JSON Lines** is one object a
  line, the same objects, and the format a pipeline or `jq -c` reads best.
- **HTML** is a `<table>` with a `<thead>`, every value escaped, as a fragment to paste into an
  email or a page, or a whole UTF-8 page with a little CSS that prints well. Copying it puts the
  markup on the clipboard as text: egui's clipboard holds only text (see Later).
- **SQL `UPDATE`** statements, one a row, setting every column but the key and naming the row by
  its primary key, as the grid's edits do. Offered only for the rows the grid could edit: a
  `SELECT` from one table whose result holds its whole primary key. Elsewhere it is greyed out,
  saying why.
- **Parquet** is written by DuckDB, which DbRover already contains: rows go into a table of
  text columns in an in-memory DuckDB, which can spill to a temporary folder rather than hold
  everything in memory. When the last row is in, each column is given the narrowest type every
  one of its values converts to (`BIGINT`, `DECIMAL`, `DOUBLE`, `BOOLEAN`, `DATE`, `TIMESTAMP`,
  `TIMESTAMPTZ`, else `VARCHAR`), with a declared type taking precedence, and `COPY … TO` writes
  the file with zstd (default) or Snappy compression. No new crate.

## Where exports start

- **The grid's Export… button** opens the export dialog: which rows (**Loaded rows** as sorted
  and filtered, or **All rows**), the format, its options, and a preview of the file's first
  lines for text formats (for Excel and Parquet, each column's type instead). **Export…** then
  asks where to save it. The dialog starts from the format and options used last.
- **Copy as ›** in the cell's right-click menu and the Edit menu copies the selected cells (the
  selected columns of the selected rows, as ⌘C does) in any format the clipboard takes. **Copy
  with headers** (⇧⌘C) copies TSV with the header line. **Copy row** quotes values as **Copy
  selection** does. Each **Copy as** format has a keymap id, `grid.copy-as.<format>`, unbound
  unless the user binds it. Copying takes the loaded rows only; it never runs the query.
- **Export data…** on a table or view in the explorer opens the same dialog with **All rows**,
  and runs `SELECT * FROM` the table on the explorer's connection, streaming, as an import runs
  there.
- **Run to file…** (⌥⌘↵, and in the Code menu) runs the statement at the cursor straight into a
  file, never loading the grid. It asks for the format and file first, then for `:name`
  parameters as a run does, then goes through the usual confirmations. Progress and **Cancel**
  show in the results toolbar, and the result panel says *Exported 1,204,331 rows to …* where
  rows would be.
- **MCP:** the `query` tool takes an optional `format`: `json` (today's answer, the default),
  `csv`, `markdown` or `jsonl`, and answers with the rows as that text, within the project's row
  limit and with `truncated` as now. MCP clients never write files: an assistant that wants a file
  writes the text it got itself, with its own permissions.
- **The keymap and menus:** **Export…**, **Run to file…** and each **Copy as** format are app
  commands, in the menus and bindable like every other, through `run_command`.
- **Left as they are:** the history's export keeps CSV and JSON, and the data sources' export
  stays TOML.

## Design

### A crate for formats

A new crate, `dbrover-export`, holds the formats, their options and the streaming writer. It
depends on `dbrover-core` and `dbrover-sql` (for the SQL formats' quoting), and brings in
`rust_xlsxwriter` (W7) and `minijinja` (W8), so neither weighs on crates that don't write files.
`dbrover-store`'s history export uses it in place of today's `ExportWriter`.

### One trait per kind of output

- **`TextFormat`** writes a header, then rows, then a footer, to any `io::Write`:

  ```rust
  pub trait TextFormat: Send {
      fn header(&mut self, out: &mut dyn Write, columns: &[ExportColumn]) -> io::Result<()>;
      fn row(&mut self, out: &mut dyn Write, row: &Row) -> io::Result<()>;
      fn footer(&mut self, out: &mut dyn Write) -> io::Result<()>;
  }
  ```

  CSV, TSV, Markdown, JSON, JSON Lines, HTML and the SQL formats implement it, and so do W8's
  templates. The same value writes a file (through a `BufWriter<File>`), the clipboard (into a
  `Vec<u8>`) and MCP's answer, which is what keeps them identical. A format keeps whatever state
  it needs (JSON's comma, a multi-row `INSERT`'s batch) and flushes it in `footer`.
- **Binary formats** (`.xlsx`, Parquet) write only files, and own them: they get rows a page at a
  time and finish into the path, through `dbrover-export`'s `FileFormat` trait. Parquet's writer
  is in the DuckDB driver, which owns DuckDB, and the engine adapts it to `FileFormat`, so
  `dbrover-export` doesn't depend on DuckDB; the others are in `dbrover-export`.
- **`ExportWriter`** stays the one way to write a file: it takes an `ExportSpec` (a format and
  its options), keeps W5d's `.partial` file and rename, and counts rows. Both loaded-rows and
  all-rows exports go through it, as now.
- **`ExportColumn`** is a column's name, its declared type name, and a `ValueKind` (integer,
  decimal, float, boolean, date, timestamp, timestamp with time zone, text, or undeclared when
  the driver reports no type). The kind is a `dbrover-core` type; `dbrover_sql::Dialect::value_kind`
  maps each dialect's type names to it, with a table-driven test per driver, since what differs
  between dialects lives in `dbrover-sql`'s `dialect` module. `.xlsx`,
  Parquet and `INSERT` use the kind; the text rules for columns without one are pure functions
  with their own tests.
- **Formats are an enum,** `ExportFormat`, matched exhaustively in one place, `ExportSpec::text_format`,
  which builds the writer for each; everything else (the dialog's option panel, the extension, the
  file dialog's filter, which outputs a format allows) is a method on it, so adding a format is one
  compiler-checked change.

### Options and where they live

- **Options are types,** one struct per format (`CsvOptions`, `XlsxOptions`, …), each with
  `Default` and serde, so illegal combinations can't be written (no quoting option on JSON).
- **The dialog remembers** the last format and each format's last options in the app's saved
  state (`app.ron`), as it remembers the window layout and recent files. They are conveniences,
  not configuration.
- **Named options are W8's presets** in `exports.toml`, where the user writes them down to keep,
  share and version. W7 adds nothing to `settings.toml`.

### Engine commands

| Command | Event | Session |
| --- | --- | --- |
| `ExportRows { console, export }` (now an `ExportSpec`) | `ExportFinished` | none (blocking pool) |
| `ExportAllRows { console, export }` (now an `ExportSpec`; also used by Run to file) | `ExportProgress`, then `ExportFinished` | the console's |
| `ExportTable { data_source, table, export }` | `ExportProgress`, then `ExportFinished` | the explorer's |

Copying formats the rows in the app: they are already there, and few. MCP formats its rows in the
MCP crate with the same `TextFormat`s.

## W8 — Your own formats

The [hackability plan](hackability.md) puts export formats among the extension points, opened
by command actions in H2 and by plugins in H3. W8 gives them a file of their own, `exports.toml`,
next to `actions.toml` in the config folder, read like it (H1a's reader: at start-up and on
**Reload**, each entry checked on its own, problems listed in the status bar and in **View ›
Exports…**). Each entry has an `id`, a `name` and an `extension`, and is one of three kinds:

```toml
# A preset: a built-in format with its options.
[[export]]
id = "excel-semicolon"
name = "CSV for Excel (semicolon)"
format = "csv"
separator = ";"
bom = true

# A template: a header, a template per row, and a footer.
[[export]]
id = "confluence"
name = "Confluence table"
extension = "txt"
header = "||{% for column in columns %} {{ column.name }} ||{% endfor %}\n"
row = "|{% for value in row %} {{ value | default('') }} |{% endfor %}\n"
footer = ""

# A program: the rows as JSON Lines on stdin, the file from its stdout.
[[export]]
id = "pdf-report"
name = "PDF report"
extension = "pdf"
command = ["/Users/me/bin/rows-to-pdf", "--title", "{table}"]
```

They appear in the export dialog's format list and in **Copy as ›** (a program's output only
where it is text), each with a keymap id, `export.<id>`.

- **Templates** use [MiniJinja](https://docs.rs/minijinja), Jinja's syntax, rendered a row at a
  time so a template streams like every other format. `header` gets `columns` (name and type),
  `data_source`, `table` (when there is one), `parameters` (the run's `:name` values) and
  `exported_at`; `row` gets `row` (the values in order), `record` (by column name), `columns` and
  `index`; `footer` gets `count`. Templates have no file access, no includes and a fuel limit, so
  a runaway loop stops with a problem rather than hanging. With `escape = "html"` every value is
  HTML-escaped unless marked safe; filters added for exports: `sql_literal`, `sql_name`, `csv`,
  `json`. A template can also be a file in `exports/`, with the three parts as `{% block %}`s.
- **Programs** run as H2's command actions do: no shell, a timeout, a stop, no secret in any
  argument or variable, and trusted by their exact definition before the first run, recorded in
  `trusted_commands.toml`. They get the rows as JSON Lines on stdin, streamed, and their stdout
  goes to the file as it arrives (no 4 MB cap, unlike H2's viewer output), so a program can
  export any number of rows. A non-zero exit removes the partial file and shows stderr.
- **Where it stops:** no grouping, totals, sorting or computed columns in templates beyond what
  Jinja's expressions do on one row, no reports made of several queries, no charts, no scheduled
  exports. Each is SQL's job, or a program's.

The details are confirmed when W8 starts, after W7 has shipped and the format trait has settled.

## Layers

Each layer is one pull request in a stack, and is useful and tested on its own.

**W7 — Exports**

- **W7a — Plan:** this document and the roadmap entries.
- **W7b — Text formats** (built): the `dbrover-export` crate; `TextFormat`; CSV with its options, TSV,
  Markdown, JSON, JSON Lines and HTML; `ExportSpec` and `ExportWriter` moved from the store, which
  uses them for the history; `ValueKind` from declared types. Golden tests: each format's output
  for awkward values (quotes, separators, line breaks, `|`, `<`, `NULL`, empty strings, emoji) and
  the paged-equals-whole test W5d has, for every format. The grid offers the new formats from its
  existing menu.
- **W7c — SQL statements** (built): `INSERT` and `UPDATE` formats over `RowSql`, golden tests per dialect,
  and a round trip on SQLite: export as `INSERT`, run into an empty copy, compare.
- **W7d — Excel** (built): `.xlsx` through `rust_xlsxwriter`: cell typing, header, freeze, autofilter,
  widths, sheets past a million rows. Tests read the file back with
  [`calamine`](https://crates.io/crates/calamine) (a dev-dependency) and check each cell's type and
  value; one writes a little over a sheet's worth of rows, in the slow tests.
- **W7e — Parquet** (built): the DuckDB writer, its type inference, compression; tests read the file
  back through DuckDB and check its schema and values.
- **W7f — The export dialog** (built): format, options, preview, loaded or all rows, the options
  remembered; kittest tests on SQLite.
- **W7g — Copy as** (built): the submenu in the cell menu and the Edit menu, **Copy with headers**, **Copy
  row** quoting as **Copy selection** does, the keymap ids; kittest tests reading the clipboard.
- **W7h — The explorer and Run to file** (built): **Export data…** on tables and views, `ExportTable`,
  **Run to file…**; engine tests on every engine with a test server (more rows than a page, a
  cancel, a read-only data source), kittest tests on SQLite.
- **W7i — MCP** (built): the `query` tool's `format`, tested in the MCP crate's tests.
- **W7j — Manual:** a chapter on exporting and copying, the troubleshooting entry for Excel and
  CSV, [Testing](testing.md), and the demo note.

**W8 — Your own formats**

- **W8a — Plan:** confirm this section's details.
- **W8b — Reading `exports.toml`:** the format and its checks in `dbrover-export` and the store,
  **View › Exports…**, **Create exports.toml**; presets in the dialog and **Copy as ›**.
- **W8c — Templates:** MiniJinja, the context, the filters, the fuel limit; tests of each
  problem message and of templates with hostile values.
- **W8d — Programs:** JSON Lines to a trusted program, its stdout to the file; engine tests with
  the test program H2 built, UI tests with its scripted runner.
- **W8e — Manual:** the chapter on `exports.toml`, with a preset, a template and a program, and
  the demo note.

## Decisions

Agreed on 2026-10-29.

| Question | Decision | Consequence |
| --- | --- | --- |
| Formats in W7 | **CSV with options, TSV, `.xlsx`, Markdown, SQL `INSERT` and `UPDATE`, JSON, JSON Lines, HTML and Parquet.** XML is left to W8's templates. | One milestone covers what DataGrip, DBeaver and TablePlus offer between them, Parquet included. |
| Typing `.xlsx` cells | **Declared types first, then the text:** numbers and ISO dates become numbers and dates, leading zeros, long digit strings and offsets stay text. | Sums and sorts work in Excel; ids and codes are never mangled. |
| BOM | **Off for CSV, on in the CSV for Excel preset.** | Files stay as RFC 4180 and other tools expect; Excel users pick one item. |
| Custom formats | **`exports.toml` with presets, header/row/footer templates (MiniJinja) and programs (JSON Lines on stdin).** | Simple formats need no script; anything else is a program, with H2's safeguards. |
| Templates stream | **A template per row, not one over every row.** | Any size works; a footer can't see the rows, only their count. |
| MCP | **Formatted text in the `query` tool's answer; no files.** | Nothing new to permit, and the safety model stays as it is. |
| Where options live | **The last used, in `app.ron`; named ones in `exports.toml` (W8).** | Nothing new in `settings.toml`; what is worth keeping is a file the user can share. |
| Reformatting dates and numbers | **No.** Values are written as the database printed them. | Exact; formatting belongs in SQL. |
| Parquet | **Through DuckDB, already in DbRover,** not the `parquet` and `arrow` crates. | No new dependency; DuckDB's writer is well tested. |
| Rich HTML on the clipboard | **Not in W7.** | Pasting a table into Google Sheets or Word takes TSV, which they read; rich paste comes later. |

## Risks

- **Excel mangling values anyway.** Typing a column wrongly would turn ids into rounded numbers or
  codes into dates. The typing rules are pure functions with table-driven tests of the values
  known to go wrong (leading zeros, 16-digit ids, `1e5`, `NaN`, `-0`, dates before 1900, offsets),
  and the round trip through `calamine` checks every cell's type.
- **Big exports holding memory.** `.xlsx` uses constant-memory mode, which writes each row to a
  temporary file; Parquet's DuckDB is given a memory limit and a temporary folder in the cache
  directory. An engine test exports more rows than a page in every format.
- **New dependencies.** `rust_xlsxwriter` (MIT or Apache-2.0, actively maintained, no `unsafe`,
  needing only `zip`, already in DbRover's dependency tree) and, in W8, `minijinja` (Apache-2.0, no
  dependencies, `unsafe` inside its value model, by an author with a long record). Both are checked
  against the minimum Rust version and the licence list when added. Download size is measured
  before and after each.
- **SQL that doesn't run elsewhere.** `INSERT`s use the source's dialect and quoting; moving rows
  between different databases may need edits, and the manual says so.
- **Templates and programs from someone else.** `exports.toml` is the user's own file, like
  `actions.toml`; templates can't touch files or run programs, programs are trusted one by one,
  and a data source export carries no exports, as it carries no actions.
- **CI time.** The new tests are store and crate tests that need no server, except W7h's engine
  tests, which reuse W5's test sources and add seconds per server. The million-row sheet test runs
  only with the slow tests.

## Exit

**W7:** I export a PostgreSQL query's every row to `.xlsx` and open it in Excel: numbers sum, dates
sort, a zip code keeps its leading zero, and the header is frozen. **CSV for Excel** opens in Excel
with accents intact. I copy a selection as a Markdown table into a pull request, and as `INSERT`s
into a SQLite console that runs them. **Export data…** on a ten-million-row table writes Parquet
without DbRover's memory growing, and **Run to file…** writes a query's rows without the grid
loading them. An assistant asks MCP for rows as CSV.

**W8:** I add a semicolon CSV preset, a Confluence table template and a program that makes a
PDF to `exports.toml`, and each appears in the export dialog and where it applies in **Copy as ›**;
a template with an infinite loop stops with a problem; the program asks once before it first runs.

## Later

- Rich HTML on the clipboard (through `arboard`, which egui already uses) so a paste into Google
  Sheets, Docs or Word keeps the table.
- JSON with typed values (numbers and booleans) from declared types.
- Exporting several tables at once from the explorer, and a file name pattern for them.
- Encodings other than UTF-8, compressed output, splitting files by size.
- Exporting the history in every format.
- Importing `.xlsx`, JSON Lines and Parquet, the counterpart of W5e.
