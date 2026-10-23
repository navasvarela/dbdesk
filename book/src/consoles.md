# Query consoles

A query console is a SQL editor tied to one data source, with the results of its latest run
underneath.

## Opening a console

- Right-click a data source in the explorer and choose **New query console**, or use the
  **New query console** card on the Welcome tab.
- Right-click a table or view and choose **Open data**: this opens a console with
  `SELECT * FROM` that object and runs it straight away.

Consoles open as tabs next to the Welcome tab, named after their data source. A second console
on the same data source is named `shop (2)`, and so on. If the data source has a colour, the tab
and a band along the top of the console carry it.

Each console has **its own connection**, separate from the explorer's and from other consoles'.
It connects the first time you run something ("Connecting…" in the toolbar), and closing the
console's tab closes the connection.

> Console text isn't saved yet: closing a console, or quitting dbdesk, discards what you typed.

## Running SQL

![A console with the results of a query](images/workspace.png)

The toolbar has three buttons; each has a shortcut that works while the editor has focus.

| Button | Shortcut | What runs |
| --- | --- | --- |
| **Run** (▷) | ⌘↵ | The selected text if there is a selection, otherwise the statement at the cursor. |
| **Run all** (▷▷) | ⇧⌘↵ | Every statement in the console, in order, stopping at the first one that fails. |
| **Cancel** (□) | ⌘. | Stops the running statement, or discards rows not yet loaded. |

On Linux and Windows, use Ctrl instead of ⌘.

**Which statement is "at the cursor"?** dbdesk splits the text into statements at semicolons,
the way the database would: semicolons inside strings, quoted names, comments, PostgreSQL's
dollar-quoted function bodies, and `BEGIN … END` or `CASE … END` blocks don't end a statement.
The statement at the cursor is the one the cursor is in, or the one just before it, so you can
press ⌘↵ with the cursor right after a semicolon.

The toolbar shows how the last run went: **Done in 12 ms**, **Failed** or **Cancelled**. While a
statement runs, Run and Run all are disabled.

## Results

Under the editor, a summary line describes the result, followed by the rows:

- `8 rows · 3 ms` for a query,
- `12 rows affected · 4 ms` for `INSERT`, `UPDATE`, `DELETE`, `MERGE`, `REPLACE` and `COPY`,
- `Done · 2 ms` for anything else, such as `CREATE TABLE`,
- or the error message, in red.

In the grid, numbers are right-aligned and `NULL` is shown in grey italics, so it can't be
mistaken for the text "NULL". Drag the edges of a column header to resize it. Hovering over a
column header shows the column's type when the database reports it (SQLite does; PostgreSQL
doesn't, yet). Right-click a cell to **Copy value** or **Copy row** (tab-separated, ready to
paste into a spreadsheet).

### Large results

Rows arrive 500 at a time. When there are more, the summary says so, and you get the next 500
by clicking **Load more rows** or by scrolling to the bottom of the grid.

![The first 500 rows of a larger result](images/results-paging.png)

A query that is still sending rows keeps its connection busy. **Cancel** discards the rows that
haven't been loaded (the summary then says `rest discarded`), and so does running something
else in the same console.

### Several statements

When a run includes more than one statement, numbered tabs above the summary pick which
statement's result to show. dbdesk shows the latest one that returned rows, or the one that
failed; failed statements' tabs are red. Hover over a tab to see its statement.

![A script of three statements](images/multiple-statements.png)

## Errors

When a statement fails, its error appears in red in the summary line and in the log console.
If the database says where in the statement the error is, the editor underlines that spot in
red until you edit the text.

![A failed query](images/console-error.png)

With **Run all**, the statements before the failing one have already run (and, outside a
transaction, are committed); the ones after it don't run at all.

## Statements that change every row

Before running an `UPDATE` or a `DELETE` without a `WHERE`, dbdesk asks first:

![The "Run anyway?" dialog](images/run-anyway.png)

Nothing in the run starts until you answer: **Run anyway** runs everything as asked, **Cancel**
runs nothing. The check looks at how each statement starts, so a `DELETE` inside a `WITH` query
isn't caught.
