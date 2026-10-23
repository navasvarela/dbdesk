# Introduction

<img src="images/logo.svg" alt="" width="72">

dbdesk is a desktop tool for browsing and querying databases, in the spirit of JetBrains
DataGrip. It connects to **PostgreSQL** and **SQLite**, shows what is inside them as a tree, and
gives you query consoles to write and run SQL, page through, sort and export the results, and
edit rows.

![dbdesk with the explorer, a query console and the log console](images/workspace.png)

The window has three areas, which you can drag around and resize:

- **Explorer**, on the left: your saved connections ("data sources") and, under each, its
  databases, schemas, tables and so on.
- **Consoles**, on the right: the Welcome tab and one tab per query console.
- **Log console**, along the bottom: what dbdesk is doing — connections opening and closing,
  queries running and how long they took, errors.

The thin bar at the very bottom shows whether dbdesk's background engine is running and how
many data sources the explorer is connected to.

This manual describes dbdesk as it is today. It is young software: things that are planned but
not built yet are not described here, apart from a few warnings in
[Troubleshooting](troubleshooting.md).
