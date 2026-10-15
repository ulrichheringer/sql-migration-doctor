# sql-migration-doctor

A dependency-free CLI that flags PostgreSQL migration statements likely to cause data loss, long locks, or expensive table work. Findings include the file and line; review them against your schema and PostgreSQL version.

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
