//! Owned, deliberately bounded SQL grammar. Unknown syntax is an error, never a skip.
mod ddl;
mod query;
pub use query::{CheckedQuery, check};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dialect {
    Postgres,
    Sqlite,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    Bool,
    I32,
    I64,
    Text,
    Uuid,
    Timestamp,
    Bytes,
    Enum(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub ty: Type,
    pub nullable: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Table {
    pub columns: BTreeMap<String, Column>,
    pub constraints: BTreeMap<String, Vec<String>>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Schema {
    pub tables: BTreeMap<String, Table>,
    pub enums: BTreeMap<String, Vec<String>>,
    pub indexes: BTreeMap<String, (String, Vec<String>)>,
}
#[derive(Clone, Debug)]
pub struct Error {
    pub line: usize,
    pub message: String,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug)]
struct Token {
    text: String,
    line: usize,
    quoted: bool,
}
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}
impl Parser {
    fn new(sql: &str) -> Result<Self> {
        if sql.len() > 1024 * 1024 {
            return Err(Error {
                line: 1,
                message: "SQL exceeds 1 MiB".into(),
            });
        }
        let b = sql.as_bytes();
        let mut i = 0;
        let mut line = 1;
        let mut tokens = Vec::new();
        while i < b.len() {
            if b[i].is_ascii_whitespace() {
                if b[i] == b'\n' {
                    line += 1
                }
                i += 1;
                continue;
            }
            if b[i..].starts_with(b"--") {
                while i < b.len() && b[i] != b'\n' {
                    i += 1
                }
                continue;
            }
            if b[i..].starts_with(b"/*") {
                return Err(Error {
                    line,
                    message: "block comments are outside the SQL subset; use -- comments".into(),
                });
            }
            let start = i;
            let ln = line;
            let quoted = b[i] == b'\'';
            if quoted {
                i += 1;
                loop {
                    if i == b.len() {
                        return Err(Error {
                            line: ln,
                            message: "unterminated string".into(),
                        });
                    }
                    if b[i] == b'\n' {
                        line += 1
                    }
                    if b[i] == b'\'' {
                        i += 1;
                        if i < b.len() && b[i] == b'\'' {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
            } else if b[i].is_ascii_alphabetic() || b[i] == b'_' {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1
                }
            } else if b[i].is_ascii_digit() || b[i] == b'$' {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1
                }
            } else if b"(),;.*=<>!+-/".contains(&b[i]) {
                i += 1;
                if i < b.len()
                    && ((b[i] == b'=' && b"<>!".contains(&b[start]))
                        || (b[start] == b'<' && b[i] == b'>'))
                {
                    i += 1
                }
            } else {
                return Err(Error {
                    line,
                    message: format!("unsupported SQL byte {:?}", b[i] as char),
                });
            }
            let text = sql[start..i].to_owned();
            tokens.push(Token {
                text: if quoted {
                    text
                } else {
                    text.to_ascii_lowercase()
                },
                line: ln,
                quoted,
            });
            if tokens.len() > 65536 {
                return Err(Error {
                    line,
                    message: "too many SQL tokens".into(),
                });
            }
        }
        Ok(Self { tokens, pos: 0 })
    }
    fn peek(&self) -> &str {
        self.tokens
            .get(self.pos)
            .map(|t| t.text.as_str())
            .unwrap_or("")
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.peek() == s {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<()> {
        if self.eat(s) {
            Ok(())
        } else {
            self.err(format!("expected {s}, found {:?}", self.peek()))
        }
    }
    fn err<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(Error {
            line: self
                .tokens
                .get(self.pos)
                .or_else(|| self.tokens.last())
                .map_or(1, |t| t.line),
            message: message.into(),
        })
    }
    fn ident(&mut self) -> Result<String> {
        let s = self.peek().to_owned();
        if s.as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            self.pos += 1;
            Ok(s)
        } else {
            self.err("expected unquoted identifier")
        }
    }
    fn string(&mut self) -> Result<String> {
        if self.tokens.get(self.pos).is_some_and(|t| t.quoted) {
            let s = self.peek();
            let out = s[1..s.len() - 1].replace("''", "'");
            self.pos += 1;
            Ok(out)
        } else {
            self.err("expected SQL string")
        }
    }
    fn names(&mut self) -> Result<Vec<String>> {
        self.expect("(")?;
        let mut out = vec![self.ident()?];
        while self.eat(",") {
            out.push(self.ident()?)
        }
        self.expect(")")?;
        Ok(out)
    }
    fn end(&mut self) -> Result<()> {
        self.eat(";");
        if self.peek().is_empty() {
            Ok(())
        } else {
            self.err("unsupported trailing SQL; use sql_unchecked! for audited escape hatches")
        }
    }
}
impl Schema {
    pub fn apply(&mut self, sql: &str, dialect: Dialect) -> Result<()> {
        let mut candidate = self.clone();
        ddl::apply(&mut candidate, sql, dialect)?;
        *self = candidate;
        Ok(())
    }
    pub fn from_dir(path: impl AsRef<Path>, dialect: Dialect) -> std::result::Result<Self, String> {
        let files = migration_files(path.as_ref())?;
        let mut schema = Self::default();
        for file in files {
            let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
            schema
                .apply(&text, dialect)
                .map_err(|e| format!("{}:{e}", file.display()))?
        }
        Ok(schema)
    }
}
pub fn migration_files(path: &Path) -> std::result::Result<Vec<std::path::PathBuf>, String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        let p = entry.map_err(|e| e.to_string())?.path();
        if p.extension().is_some_and(|e| e == "sql") {
            files.push(p)
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(format!("{}: no migrations", path.display()));
    }
    let mut last = 0u64;
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy();
        let version = name
            .split('_')
            .next()
            .unwrap()
            .parse::<u64>()
            .map_err(|_| format!("{name}: expected VERSION_name.sql"))?;
        if version <= last {
            return Err(format!(
                "{name}: duplicate or non-increasing migration version"
            ));
        }
        last = version;
    }
    Ok(files)
}
