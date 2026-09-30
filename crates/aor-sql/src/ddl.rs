use super::*;
fn ty(p: &mut Parser, s: &Schema, d: Dialect) -> Result<Type> {
    let n = p.ident()?;
    Ok(match n.as_str() {
        "boolean" | "bool" => Type::Bool,
        "integer" | "int" | "int4" => Type::I32,
        "bigint" | "int8" => Type::I64,
        "text" => Type::Text,
        "uuid" => {
            if d == Dialect::Sqlite {
                return p.err("SQLite uses TEXT for UUIDs");
            }
            Type::Uuid
        }
        "timestamptz" => {
            if d == Dialect::Sqlite {
                return p.err("SQLite uses TEXT for UTC timestamps");
            }
            Type::Timestamp
        }
        "timestamp" => {
            p.expect("with")?;
            p.expect("time")?;
            p.expect("zone")?;
            if d == Dialect::Sqlite {
                return p.err("SQLite uses TEXT for UTC timestamps");
            }
            Type::Timestamp
        }
        "bytea" | "blob" => Type::Bytes,
        n if s.enums.contains_key(n) => Type::Enum(n.into()),
        _ => return p.err(format!("unsupported SQL type {n}")),
    })
}
fn verify_columns(p: &Parser, t: &Table, cols: &[String]) -> Result<()> {
    for c in cols {
        if !t.columns.contains_key(c) {
            return p.err(format!("unknown column {c}"));
        }
    }
    Ok(())
}
fn constraint(p: &mut Parser, s: &Schema, t: &Table) -> Result<Vec<String>> {
    let cols = if p.eat("primary") {
        p.expect("key")?;
        p.names()?
    } else if p.eat("unique") {
        p.names()?
    } else if p.eat("foreign") {
        p.expect("key")?;
        let cols = p.names()?;
        reference(p, s, cols.len())?;
        cols
    } else if p.eat("check") {
        p.expect("(")?;
        let c = p.ident()?;
        verify_columns(p, t, std::slice::from_ref(&c))?;
        if !["=", "<>", "!=", ">", "<", ">=", "<="].contains(&p.peek()) {
            return p.err("CHECK subset requires column comparison with literal");
        }
        p.pos += 1;
        literal(p)?;
        p.expect(")")?;
        vec![c]
    } else {
        return p.err("unsupported constraint");
    };
    verify_columns(p, t, &cols)?;
    Ok(cols)
}
fn literal(p: &mut Parser) -> Result<()> {
    if p.eat("-") {
        if !p.peek().bytes().all(|b| b.is_ascii_digit()) || p.peek().is_empty() {
            return p.err("expected number");
        }
        p.pos += 1;
        return Ok(());
    }
    if p.tokens.get(p.pos).is_some_and(|t| t.quoted)
        || ["null", "true", "false"].contains(&p.peek())
        || (!p.peek().is_empty() && p.peek().bytes().all(|b| b.is_ascii_digit()))
    {
        p.pos += 1;
        Ok(())
    } else if p.eat("now") {
        p.expect("(")?;
        p.expect(")")
    } else if p.eat("current_timestamp") {
        Ok(())
    } else {
        p.err("unsupported default literal")
    }
}
fn reference(p: &mut Parser, s: &Schema, n: usize) -> Result<()> {
    p.expect("references")?;
    let name = p.ident()?;
    let cols = p.names()?;
    let t = s.tables.get(&name).ok_or_else(|| Error {
        line: 1,
        message: format!("unknown referenced table {name}"),
    })?;
    verify_columns(p, t, &cols)?;
    if cols.len() != n {
        return p.err("foreign key arity mismatch");
    }
    if p.eat("on") {
        p.expect("delete")?;
        if !(p.eat("cascade") || p.eat("restrict")) {
            p.expect("set")?;
            p.expect("null")?
        }
    }
    Ok(())
}
fn column(p: &mut Parser, s: &Schema, d: Dialect) -> Result<Column> {
    let name = p.ident()?;
    let ty = ty(p, s, d)?;
    let mut nullable = true;
    loop {
        if p.eat("not") {
            p.expect("null")?;
            nullable = false
        } else if p.eat("null") {
        } else if p.eat("primary") {
            p.expect("key")?;
            nullable = false
        } else if p.eat("unique") {
        } else if p.eat("default") {
            literal(p)?
        } else if p.peek() == "references" {
            reference(p, s, 1)?
        } else {
            break;
        }
    }
    Ok(Column { name, ty, nullable })
}
pub(super) fn apply(s: &mut Schema, sql: &str, d: Dialect) -> Result<()> {
    let mut p = Parser::new(sql)?;
    while !p.peek().is_empty() {
        if p.eat("create") {
            if p.eat("table") {
                let name = p.ident()?;
                if s.tables.contains_key(&name) {
                    return p.err(format!("table {name} already exists"));
                }
                p.expect("(")?;
                let mut t = Table::default();
                loop {
                    if p.eat("constraint") {
                        let name = p.ident()?;
                        let cols = constraint(&mut p, s, &t)?;
                        if t.constraints.insert(name, cols).is_some() {
                            return p.err("duplicate constraint");
                        }
                    } else if ["primary", "unique", "foreign", "check"].contains(&p.peek()) {
                        let primary = p.peek() == "primary";
                        let cols = constraint(&mut p, s, &t)?;
                        if primary {
                            for c in &cols {
                                t.columns.get_mut(c).unwrap().nullable = false
                            }
                        }
                        let key = format!("__{}", t.constraints.len());
                        t.constraints.insert(key, cols);
                    } else {
                        let col = column(&mut p, s, d)?;
                        if t.columns.insert(col.name.clone(), col).is_some() {
                            return p.err("duplicate column");
                        }
                    }
                    if !p.eat(",") {
                        break;
                    }
                }
                p.expect(")")?;
                s.tables.insert(name, t);
            } else if p.eat("type") {
                if d == Dialect::Sqlite {
                    return p.err("SQLite does not support enums");
                }
                let name = p.ident()?;
                p.expect("as")?;
                p.expect("enum")?;
                p.expect("(")?;
                let mut values = vec![p.string()?];
                while p.eat(",") {
                    let value = p.string()?;
                    if values.contains(&value) {
                        return p.err("duplicate enum label");
                    }
                    values.push(value)
                }
                p.expect(")")?;
                if s.enums.insert(name, values).is_some() {
                    return p.err("enum already exists");
                }
            } else {
                p.eat("unique");
                p.expect("index")?;
                if p.eat("concurrently") && d == Dialect::Sqlite {
                    return p.err("CONCURRENTLY requires PostgreSQL");
                }
                let name = p.ident()?;
                p.expect("on")?;
                let table = p.ident()?;
                let cols = p.names()?;
                let t = s.tables.get(&table).ok_or_else(|| Error {
                    line: 1,
                    message: format!("unknown table {table}"),
                })?;
                verify_columns(&p, t, &cols)?;
                if s.indexes.insert(name, (table, cols)).is_some() {
                    return p.err("index already exists");
                }
            }
        } else if p.eat("alter") {
            if p.eat("type") {
                let name = p.ident()?;
                p.expect("add")?;
                p.expect("value")?;
                let value = p.string()?;
                let values = s.enums.get_mut(&name).ok_or_else(|| Error {
                    line: 1,
                    message: format!("unknown enum {name}"),
                })?;
                if values.contains(&value) {
                    return p.err("duplicate enum label");
                }
                values.push(value);
            } else {
                p.expect("table")?;
                let name = p.ident()?;
                let Some(mut t) = s.tables.get(&name).cloned() else {
                    return p.err(format!("unknown table {name}"));
                };
                if p.eat("add") {
                    if p.eat("constraint") {
                        let name = p.ident()?;
                        let cols = constraint(&mut p, s, &t)?;
                        if t.constraints.insert(name, cols).is_some() {
                            return p.err("constraint already exists");
                        }
                    } else {
                        p.eat("column");
                        let col = column(&mut p, s, d)?;
                        if t.columns.insert(col.name.clone(), col).is_some() {
                            return p.err("column already exists");
                        }
                    }
                } else if p.eat("drop") {
                    if p.eat("constraint") {
                        let name = p.ident()?;
                        if t.constraints.remove(&name).is_none() {
                            return p.err("unknown constraint");
                        }
                    } else {
                        p.eat("column");
                        let c = p.ident()?;
                        if t.constraints.values().any(|cols| cols.contains(&c))
                            || s.indexes
                                .values()
                                .any(|(table, cols)| table == &name && cols.contains(&c))
                        {
                            return p
                                .err("drop dependent constraints/indexes before dropping column");
                        }
                        if t.columns.remove(&c).is_none() {
                            return p.err("unknown column");
                        }
                    }
                } else if p.eat("rename") {
                    if p.eat("column") {
                        let old = p.ident()?;
                        p.expect("to")?;
                        let new = p.ident()?;
                        if t.columns.contains_key(&new) {
                            return p.err("column already exists");
                        }
                        let Some(mut c) = t.columns.remove(&old) else {
                            return p.err("unknown column");
                        };
                        c.name = new.clone();
                        t.columns.insert(new.clone(), c);
                        for cols in t.constraints.values_mut() {
                            for c in cols {
                                if *c == old {
                                    *c = new.clone()
                                }
                            }
                        }
                        for (table, cols) in s.indexes.values_mut() {
                            if *table == name {
                                for c in cols {
                                    if *c == old {
                                        *c = new.clone()
                                    }
                                }
                            }
                        }
                    } else {
                        p.expect("to")?;
                        let new = p.ident()?;
                        if s.tables.contains_key(&new) {
                            return p.err("table already exists");
                        }
                        s.tables.remove(&name);
                        s.tables.insert(new.clone(), t);
                        for (table, _) in s.indexes.values_mut() {
                            if *table == name {
                                *table = new.clone()
                            }
                        }
                        p.expect(";")?;
                        continue;
                    }
                } else if p.eat("alter") {
                    p.eat("column");
                    let c = p.ident()?;
                    let Some(col) = t.columns.get_mut(&c) else {
                        return p.err("unknown column");
                    };
                    if p.eat("set") {
                        if p.eat("not") {
                            p.expect("null")?;
                            col.nullable = false
                        } else {
                            p.expect("default")?;
                            literal(&mut p)?
                        }
                    } else if p.eat("drop") {
                        if p.eat("not") {
                            p.expect("null")?;
                            col.nullable = true
                        } else {
                            p.expect("default")?
                        }
                    } else {
                        p.expect("type")?;
                        col.ty = ty(&mut p, s, d)?
                    }
                } else {
                    return p.err("unsupported ALTER TABLE action");
                }
                s.tables.insert(name, t);
            }
        } else if p.eat("drop") {
            if p.eat("table") {
                let name = p.ident()?;
                if s.tables.remove(&name).is_none() {
                    return p.err("unknown table");
                }
                s.indexes.retain(|_, (t, _)| *t != name);
            } else if p.eat("index") {
                let name = p.ident()?;
                if s.indexes.remove(&name).is_none() {
                    return p.err("unknown index");
                }
            } else {
                p.expect("type")?;
                let name = p.ident()?;
                if s.tables
                    .values()
                    .flat_map(|t| t.columns.values())
                    .any(|c| c.ty == Type::Enum(name.clone()))
                {
                    return p.err("enum still used by a column");
                }
                if s.enums.remove(&name).is_none() {
                    return p.err("unknown enum");
                }
            }
        } else {
            return p.err("unsupported migration statement (DDL only)");
        }
        p.expect(";")?;
    }
    Ok(())
}
