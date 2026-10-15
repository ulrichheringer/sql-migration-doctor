use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    line: usize,
    severity: &'static str,
    reason: &'static str,
}

fn main() {
    let mut args = env::args_os().skip(1);
    let Some(input) = args.next() else {
        eprintln!("Usage: sql-migration-doctor <migration.sql|directory>");
        process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("Usage: sql-migration-doctor <migration.sql|directory>");
        process::exit(2);
    }

    let root = PathBuf::from(input);
    let mut files = Vec::new();
    if let Err(error) = collect_sql_files(&root, &mut files) {
        eprintln!("{}: {error}", root.display());
        process::exit(2);
    }
    if files.is_empty() {
        eprintln!("{}: no .sql files found", root.display());
        process::exit(2);
    }

    let mut found = false;
    for file in files {
        match fs::read_to_string(&file) {
            Ok(sql) => {
                for finding in analyze(&sql) {
                    found = true;
                    println!(
                        "{}:{}: [{}] {}",
                        file.display(),
                        finding.line,
                        finding.severity,
                        finding.reason
                    );
                }
            }
            Err(error) => {
                eprintln!("{}: {error}", file.display());
                process::exit(2);
            }
        }
    }
    if !found {
        println!("No known risky migration patterns found.");
    } else {
        process::exit(1);
    }
}

fn collect_sql_files(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
        {
            files.push(path.to_owned());
        }
        return Ok(());
    }
    if !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "path does not exist",
        ));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            collect_sql_files(&child, files)?;
        } else if child
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
        {
            files.push(child);
        }
    }
    files.sort();
    Ok(())
}

fn strip_comments(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while let Some(ch) = chars.next() {
        let next = chars.peek().copied();
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                result.push('\n');
            } else {
                result.push(' ');
            }
        } else if in_block_comment {
            if ch == '*' && next == Some('/') {
                chars.next();
                result.push_str("  ");
                in_block_comment = false;
            } else {
                result.push(if ch == '\n' { '\n' } else { ' ' });
            }
        } else if in_single {
            result.push(ch);
            if ch == '\'' {
                if next == Some('\'') {
                    result.push(chars.next().unwrap());
                } else {
                    in_single = false;
                }
            }
        } else if in_double {
            result.push(ch);
            if ch == '"' {
                if next == Some('"') {
                    result.push(chars.next().unwrap());
                } else {
                    in_double = false;
                }
            }
        } else if ch == '-' && next == Some('-') {
            chars.next();
            result.push_str("  ");
            in_line_comment = true;
        } else if ch == '/' && next == Some('*') {
            chars.next();
            result.push_str("  ");
            in_block_comment = true;
        } else {
            if ch == '\'' {
                in_single = true;
            } else if ch == '"' {
                in_double = true;
            }
            result.push(ch);
        }
    }
    result
}

fn analyze(sql: &str) -> Vec<Finding> {
    let sql = strip_comments(sql);
    let mut findings = Vec::new();
    let mut statement = String::new();
    let mut line = 1;
    let mut statement_line = 1;

    for ch in sql.chars() {
        if statement.trim().is_empty() && !ch.is_whitespace() {
            statement.clear();
            statement_line = line;
        }
        statement.push(ch);
        if ch == ';' {
            inspect_statement(&statement, statement_line, &mut findings);
            statement.clear();
        }
        if ch == '\n' {
            line += 1;
        }
    }
    inspect_statement(&statement, statement_line, &mut findings);
    findings
}

fn inspect_statement(statement: &str, start_line: usize, findings: &mut Vec<Finding>) {
    let normalized = statement
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return;
    }
    let add_column_not_null = normalized.contains("add column")
        && normalized.contains("not null")
        && !normalized.contains("default");
    let rules = [
        (
            normalized.contains("drop table"),
            "drop table",
            "HIGH",
            "Dropping a table removes its data; confirm backups and dependent objects.",
        ),
        (
            normalized.starts_with("truncate"),
            "truncate",
            "HIGH",
            "TRUNCATE removes all rows and takes a strong table lock.",
        ),
        (
            normalized.contains("drop column"),
            "drop column",
            "HIGH",
            "Dropping a column permanently discards its data.",
        ),
        (
            normalized.contains("create index")
                && !normalized.contains("create index concurrently"),
            "create index",
            "WARN",
            "CREATE INDEX can block writes; use CREATE INDEX CONCURRENTLY when appropriate.",
        ),
        (
            normalized.contains("set not null") && !normalized.contains("add column"),
            "set not null",
            "WARN",
            "SET NOT NULL may scan the table and hold a strong lock.",
        ),
        (
            normalized.contains("vacuum full"),
            "vacuum full",
            "HIGH",
            "VACUUM FULL rewrites the table and holds an exclusive lock.",
        ),
        (
            normalized.contains("alter column") && normalized.contains(" type "),
            "alter column",
            "WARN",
            "Changing a column type can rewrite the table and hold a strong lock.",
        ),
        (
            normalized.contains("add constraint")
                && (normalized.contains("foreign key") || normalized.contains("references"))
                && !normalized.contains("not valid"),
            "add constraint",
            "WARN",
            "Adding a foreign key can scan the table and lock writes; consider NOT VALID then VALIDATE CONSTRAINT.",
        ),
    ];

    for (matches, keyword, severity, reason) in rules {
        if matches {
            findings.push(Finding {
                line: line_of(statement, start_line, keyword),
                severity,
                reason,
            });
        }
    }

    if add_column_not_null {
        findings.push(Finding {
            line: line_of(statement, start_line, "add column"),
            severity: "WARN",
            reason: "Adding a NOT NULL column without a default can fail on populated tables or require a table scan.",
        });
    }
    if normalized.contains("delete from") && !normalized.contains(" where ") {
        findings.push(Finding {
            line: line_of(statement, start_line, "delete from"),
            severity: "HIGH",
            reason: "DELETE without WHERE removes every row from the target table.",
        });
    }
    if normalized.contains("update ") && !normalized.contains(" where ") {
        findings.push(Finding {
            line: line_of(statement, start_line, "update "),
            severity: "WARN",
            reason: "UPDATE without WHERE changes every row in the target table.",
        });
    }
}

fn line_of(statement: &str, start_line: usize, keyword: &str) -> usize {
    let lower = statement.to_ascii_lowercase();
    lower
        .find(keyword)
        .map(|index| start_line + statement[..index].matches('\n').count())
        .unwrap_or(start_line)
}

#[cfg(test)]
mod tests {
    use super::{analyze, strip_comments};

    #[test]
    fn detects_destructive_statements_and_index_lock() {
        let findings = analyze("DROP TABLE users;\nCREATE INDEX users_email_idx ON users(email);");
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, "HIGH");
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[1].line, 2);
    }

    #[test]
    fn recognizes_multiline_column_change_and_add_not_null() {
        let findings = analyze("ALTER TABLE accounts\n  ALTER COLUMN balance TYPE numeric;\nALTER TABLE accounts ADD COLUMN active boolean NOT NULL;");
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[1].line, 3);
    }

    #[test]
    fn ignores_comments_and_concurrent_indexes() {
        let sql =
            "-- DROP TABLE users;\nCREATE INDEX CONCURRENTLY users_email_idx ON users(email);";
        assert!(analyze(sql).is_empty());
        assert_eq!(
            strip_comments("select 1; -- comment\n"),
            "select 1;           \n"
        );
    }
}
