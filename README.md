# sql-migration-doctor

Scan PostgreSQL migrations for operations that may cause data loss, long locks, or performance problems. Each finding includes the file, line number, and reason.

## Install

Requires Rust 1.70 or newer.

```sh
cargo install --path .
```

## Use

Pass a migration file or a directory (searched recursively). The command exits with status 1 when it finds warnings.

```sh
sql-migration-doctor ./db/migrations
```
