use super::*;
#[derive(Clone, Debug)]
pub struct CheckedQuery {
    pub columns: Vec<Column>,
    pub parameters: Vec<Column>,
}
#[derive(Clone)]
struct Expr {
    ty: Option<Type>,
    nullable: bool,
    parameter: Option<usize>,
    integer: bool,
    name: String,
}
impl Expr {
    fn typed(ty: Type, nullable: bool, name: impl Into<String>) -> Self {
        Self {
            ty: Some(ty),
            nullable,
            parameter: None,
            integer: false,
            name: name.into(),
        }
    }
}
struct Checker<'a> {
    p: Parser,
    s: &'a Schema,
    d: Dialect,
    scope: BTreeMap<String, Table>,
    ctes: BTreeMap<String, Table>,
    params: BTreeMap<usize, Column>,
    used_parameters: std::collections::BTreeSet<usize>,
    depth: usize,
}
pub fn check(schema: &Schema, sql: &str, dialect: Dialect) -> Result<CheckedQuery> {
    let mut c = Checker {
        p: Parser::new(sql)?,
        s: schema,
        d: dialect,
        scope: BTreeMap::new(),
        ctes: BTreeMap::new(),
        params: BTreeMap::new(),
        used_parameters: Default::default(),
        depth: 0,
    };
    let columns = c.statement()?;
    c.p.end()?;
    let mut parameters = Vec::new();
    for n in 1..=c.used_parameters.last().copied().unwrap_or(0) {
        let Some(p) = c.params.remove(&n) else {
            return c.p.err(format!("parameter ${n} has no inferred type"));
        };
        parameters.push(p)
    }
    Ok(CheckedQuery {
        columns,
        parameters,
    })
}
impl Checker<'_> {
    fn statement(&mut self) -> Result<Vec<Column>> {
        self.depth += 1;
        if self.depth > 32 {
            return self.p.err("SQL nesting exceeds 32");
        }
        if self.p.eat("with") {
            loop {
                let name = self.p.ident()?;
                self.p.expect("as")?;
                self.p.expect("(")?;
                let scope = std::mem::take(&mut self.scope);
                let cols = self.statement()?;
                self.scope = scope;
                self.p.expect(")")?;
                let table = Table {
                    columns: cols.into_iter().map(|c| (c.name.clone(), c)).collect(),
                    constraints: BTreeMap::new(),
                };
                if self.ctes.insert(name, table).is_some() {
                    return self.p.err("duplicate CTE");
                };
                if !self.p.eat(",") {
                    break;
                }
            }
        }
        let result = if self.p.eat("select") {
            self.select()
        } else if self.p.eat("insert") {
            self.insert()
        } else if self.p.eat("update") {
            self.update()
        } else if self.p.eat("delete") {
            self.delete()
        } else {
            self.p
                .err("unsupported query; use sql_unchecked! for audited SQL")
        };
        self.depth -= 1;
        result
    }
    fn table(&mut self, nullable: bool) -> Result<String> {
        let name = self.p.ident()?;
        let Some(mut t) = self
            .ctes
            .get(&name)
            .or_else(|| self.s.tables.get(&name))
            .cloned()
        else {
            return self.p.err(format!("unknown table {name}"));
        };
        let alias = if self.p.eat("as") {
            self.p.ident()?
        } else {
            name.clone()
        };
        if nullable {
            for c in t.columns.values_mut() {
                c.nullable = true
            }
        }
        if self.scope.insert(alias, t).is_some() {
            return self.p.err("duplicate table alias");
        };
        Ok(name)
    }
    fn bind(&mut self, e: &mut Expr, ty: &Type, nullable: bool) -> Result<()> {
        if e.integer && !matches!(ty, Type::I32 | Type::I64) {
            return self.p.err("integer literal requires integer context");
        }
        if let Some(n) = e.parameter {
            if let Some(previous) = self.params.get(&n) {
                if previous.ty != *ty {
                    return self
                        .p
                        .err(format!("parameter ${n} used with incompatible types"));
                }
            }
            self.params
                .entry(n)
                .and_modify(|c| c.nullable &= nullable)
                .or_insert(Column {
                    name: format!("p{n}"),
                    ty: ty.clone(),
                    nullable,
                });
            e.ty = Some(ty.clone());
        }
        if e.ty.as_ref().is_some_and(|t| t != ty) {
            return self
                .p
                .err(format!("type mismatch: expected {ty:?}, found {:?}", e.ty));
        }
        if e.ty.is_none() {
            e.ty = Some(ty.clone())
        }
        Ok(())
    }
    fn boolean(&mut self) -> Result<()> {
        let mut e = self.expr(0)?;
        self.bind(&mut e, &Type::Bool, false)
    }
    fn select(&mut self) -> Result<Vec<Column>> {
        self.p.eat("distinct");
        let projection = self.p.pos;
        let mut nesting = 0i32;
        let mut from = None;
        for i in projection..self.p.tokens.len() {
            match self.p.tokens[i].text.as_str() {
                "(" => nesting += 1,
                ")" => {
                    if nesting == 0 {
                        break;
                    }
                    nesting -= 1
                }
                "from" if nesting == 0 => {
                    from = Some(i);
                    break;
                }
                ";" if nesting == 0 => break,
                _ => {}
            }
        }
        let Some(from) = from else {
            return self.p.err("SELECT requires FROM");
        };
        self.p.pos = from + 1;
        self.table(false)?;
        loop {
            let left = if self.p.eat("left") {
                self.p.eat("outer");
                true
            } else {
                self.p.eat("inner");
                false
            };
            if !self.p.eat("join") {
                if left {
                    return self.p.err("expected JOIN");
                }
                break;
            }
            self.table(left)?;
            self.p.expect("on")?;
            self.boolean()?;
        }
        let after_from = self.p.pos;
        self.p.pos = projection;
        let columns = self.projection()?;
        self.p.expect("from")?;
        if self.p.pos != from + 1 {
            return self.p.err("invalid projection");
        };
        self.p.pos = after_from;
        if self.p.eat("where") {
            self.boolean()?
        }
        if self.p.eat("group") {
            self.p.expect("by")?;
            self.expr_list()?;
        }
        if self.p.eat("having") {
            self.boolean()?
        }
        if self.p.eat("order") {
            self.p.expect("by")?;
            self.order()?;
        }
        if self.p.eat("limit") {
            let mut e = self.expr(0)?;
            self.bind(&mut e, &Type::I64, false)?;
        }
        if self.p.eat("offset") {
            let mut e = self.expr(0)?;
            self.bind(&mut e, &Type::I64, false)?;
        }
        if self.p.eat("for") {
            if self.d == Dialect::Sqlite {
                return self.p.err("row locks require PostgreSQL");
            };
            self.p.expect("update")?;
            if self.p.eat("skip") {
                self.p.expect("locked")?
            } else {
                self.p.eat("nowait");
            }
        }
        Ok(columns)
    }
    fn projection(&mut self) -> Result<Vec<Column>> {
        let mut cols = Vec::new();
        loop {
            if self.p.peek() == "*" {
                return self.p.err("SELECT * is forbidden; name each column");
            };
            let e = self.expr(0)?;
            let name = if self.p.eat("as") {
                self.p.ident()?
            } else {
                e.name
            };
            if name.is_empty() {
                return self.p.err("computed result requires AS name");
            };
            if cols.iter().any(|c: &Column| c.name == name) {
                return self.p.err(format!("duplicate result name {name}"));
            }
            let Some(ty) = e.ty else {
                return self.p.err("cannot infer result type");
            };
            cols.push(Column {
                name,
                ty,
                nullable: e.nullable,
            });
            if !self.p.eat(",") {
                break;
            }
        }
        Ok(cols)
    }
    fn expr_list(&mut self) -> Result<()> {
        self.expr(0)?;
        while self.p.eat(",") {
            self.expr(0)?;
        }
        Ok(())
    }
    fn order(&mut self) -> Result<()> {
        loop {
            self.expr(0)?;
            if !self.p.eat("asc") {
                self.p.eat("desc");
            }
            if self.p.eat("nulls") && !(self.p.eat("first") || self.p.eat("last")) {
                return self.p.err("expected FIRST or LAST");
            }
            if !self.p.eat(",") {
                break;
            }
        }
        Ok(())
    }
    fn insert(&mut self) -> Result<Vec<Column>> {
        self.p.expect("into")?;
        let table = self.table(false)?;
        let names = self.p.names()?;
        let t = self.s.tables.get(&table).cloned().ok_or_else(|| Error {
            line: self.p.tokens[self.p.pos.saturating_sub(1)].line,
            message: "writes require a schema table, not a CTE".into(),
        })?;
        for n in &names {
            if !t.columns.contains_key(n) {
                return self.p.err(format!("unknown column {n}"));
            }
        }
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        if unique.len() != names.len() {
            return self.p.err("duplicate INSERT column");
        }
        self.p.expect("values")?;
        self.p.expect("(")?;
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                self.p.expect(",")?
            }
            let col = &t.columns[name];
            let mut e = self.expr(0)?;
            self.bind(&mut e, &col.ty, col.nullable)?;
            if e.nullable && !col.nullable && e.parameter.is_none() {
                return self.p.err("NULL assigned to NOT NULL column");
            }
        }
        self.p.expect(")")?;
        if self.p.eat("on") {
            self.p.expect("conflict")?;
            if self.p.peek() == "(" {
                let cols = self.p.names()?;
                for c in cols {
                    if !t.columns.contains_key(&c) {
                        return self.p.err(format!("unknown conflict column {c}"));
                    }
                }
            }
            self.p.expect("do")?;
            if !self.p.eat("nothing") {
                self.p.expect("update")?;
                self.p.expect("set")?;
                self.scope.insert("excluded".into(), t.clone());
                self.assignments(&t)?;
                if self.p.eat("where") {
                    self.boolean()?
                }
            }
        }
        self.scope.remove("excluded");
        self.returning()
    }
    fn assignments(&mut self, t: &Table) -> Result<()> {
        loop {
            let name = self.p.ident()?;
            let Some(col) = t.columns.get(&name) else {
                return self.p.err(format!("unknown column {name}"));
            };
            self.p.expect("=")?;
            let mut e = self.expr(0)?;
            self.bind(&mut e, &col.ty, col.nullable)?;
            if e.nullable && !col.nullable && e.parameter.is_none() {
                return self.p.err("nullable value assigned to NOT NULL column");
            };
            if !self.p.eat(",") {
                break;
            }
        }
        Ok(())
    }
    fn update(&mut self) -> Result<Vec<Column>> {
        let table = self.table(false)?;
        let t = self.s.tables.get(&table).cloned().ok_or_else(|| Error {
            line: self.p.tokens[self.p.pos.saturating_sub(1)].line,
            message: "writes require a schema table, not a CTE".into(),
        })?;
        self.p.expect("set")?;
        self.assignments(&t)?;
        if self.p.eat("where") {
            self.boolean()?
        }
        self.returning()
    }
    fn delete(&mut self) -> Result<Vec<Column>> {
        self.p.expect("from")?;
        self.table(false)?;
        if self.p.eat("where") {
            self.boolean()?
        }
        self.returning()
    }
    fn returning(&mut self) -> Result<Vec<Column>> {
        if self.p.eat("returning") {
            self.projection()
        } else {
            Ok(Vec::new())
        }
    }
    fn expr(&mut self, min: u8) -> Result<Expr> {
        self.depth += 1;
        if self.depth > 64 {
            return self.p.err("expression nesting exceeds 64");
        };
        let result = self.expr_inner(min);
        self.depth -= 1;
        result
    }
    fn expr_inner(&mut self, min: u8) -> Result<Expr> {
        let mut left = if self.p.eat("(") {
            let e = self.expr(0)?;
            self.p.expect(")")?;
            e
        } else if self.p.eat("not") {
            let mut e = self.expr(6)?;
            self.bind(&mut e, &Type::Bool, false)?;
            e
        } else if self.p.eat("-") {
            let e = self.expr(6)?;
            if !matches!(e.ty, Some(Type::I32 | Type::I64)) {
                return self.p.err("unary minus requires integer");
            };
            e
        } else if self.p.peek().starts_with('$') {
            let n = self.p.peek()[1..].parse::<usize>().map_err(|_| Error {
                line: 1,
                message: "bad parameter".into(),
            })?;
            if n == 0 || n > 256 {
                return self.p.err("parameter index must be 1..256");
            };
            self.p.pos += 1;
            self.used_parameters.insert(n);
            Expr {
                ty: self.params.get(&n).map(|c| c.ty.clone()),
                nullable: false,
                parameter: Some(n),
                integer: false,
                name: String::new(),
            }
        } else if self.p.tokens.get(self.p.pos).is_some_and(|t| t.quoted) {
            self.p.pos += 1;
            Expr::typed(Type::Text, false, "")
        } else if self.p.eat("null") {
            Expr {
                ty: None,
                nullable: true,
                parameter: None,
                integer: false,
                name: String::new(),
            }
        } else if self.p.eat("true") || self.p.eat("false") {
            Expr::typed(Type::Bool, false, "")
        } else if !self.p.peek().is_empty() && self.p.peek().bytes().all(|b| b.is_ascii_digit()) {
            self.p.pos += 1;
            Expr {
                ty: None,
                nullable: false,
                parameter: None,
                integer: true,
                name: String::new(),
            }
        } else {
            let name = self.p.ident()?;
            if self.p.eat("(") {
                self.function(&name)?
            } else {
                let (alias, column) = if self.p.eat(".") {
                    (Some(name), self.p.ident()?)
                } else {
                    (None, name)
                };
                let mut found = None;
                for (a, t) in &self.scope {
                    if alias.as_ref().is_some_and(|wanted| wanted != a) {
                        continue;
                    }
                    if let Some(c) = t.columns.get(&column) {
                        if found.is_some() {
                            return self.p.err(format!("ambiguous column {column}"));
                        }
                        found = Some(c.clone());
                    }
                }
                let Some(c) = found else {
                    return self.p.err(format!(
                        "unknown column {}{column}",
                        alias.map(|s| s + ".").unwrap_or_default()
                    ));
                };
                Expr::typed(c.ty, c.nullable, column)
            }
        };
        loop {
            if self.p.peek() == "is" && min <= 3 {
                self.p.pos += 1;
                self.p.eat("not");
                self.p.expect("null")?;
                left = Expr::typed(Type::Bool, false, "");
                continue;
            }
            let op = self.p.peek().to_owned();
            let prec = match op.as_str() {
                "or" => 1,
                "and" => 2,
                "=" | "<>" | "!=" | "<" | ">" | "<=" | ">=" | "like" => 3,
                "+" | "-" => 4,
                "*" | "/" => 5,
                _ => 0,
            };
            if prec == 0 || prec < min {
                break;
            }
            self.p.pos += 1;
            let mut right = self.expr(prec + 1)?;
            let ty = if prec <= 2 {
                Type::Bool
            } else if op == "like" {
                Type::Text
            } else {
                left.ty
                    .clone()
                    .or_else(|| right.ty.clone())
                    .unwrap_or(Type::I64)
            };
            self.bind(&mut left, &ty, false)?;
            self.bind(&mut right, &ty, false)?;
            if prec >= 4 && !matches!(ty, Type::I32 | Type::I64) {
                return self.p.err("arithmetic requires integers");
            }
            left = Expr::typed(
                if prec <= 3 { Type::Bool } else { ty },
                left.nullable || right.nullable,
                "",
            );
        }
        Ok(left)
    }
    fn function(&mut self, name: &str) -> Result<Expr> {
        let mut e = match name {
            "count" => {
                if !self.p.eat("*") {
                    self.expr(0)?;
                }
                self.p.expect(")")?;
                Expr::typed(Type::I64, false, "")
            }
            "row_number" | "rank" | "dense_rank" => {
                self.p.expect(")")?;
                Expr::typed(Type::I64, false, "")
            }
            "lower" | "upper" => {
                let mut e = self.expr(0)?;
                self.bind(&mut e, &Type::Text, false)?;
                self.p.expect(")")?;
                Expr::typed(Type::Text, e.nullable, "")
            }
            "min" | "max" | "sum" => {
                let mut e = self.expr(0)?;
                self.p.expect(")")?;
                if name == "sum" {
                    if e.ty != Some(Type::I32) {
                        return self
                            .p
                            .err("SUM subset accepts INT4 (INT8 sums return NUMERIC)");
                    };
                    e.ty = Some(Type::I64)
                }
                e.nullable = true;
                e.name.clear();
                e
            }
            "coalesce" => {
                let mut a = self.expr(0)?;
                self.p.expect(",")?;
                let mut b = self.expr(0)?;
                self.p.expect(")")?;
                let Some(ty) = a.ty.clone().or_else(|| b.ty.clone()) else {
                    return self.p.err("COALESCE needs a typed argument");
                };
                self.bind(&mut a, &ty, true)?;
                self.bind(&mut b, &ty, true)?;
                Expr::typed(ty, a.nullable && b.nullable, "")
            }
            _ => {
                return self
                    .p
                    .err(format!("unsupported function {name}; use sql_unchecked!"));
            }
        };
        if self.p.eat("over") {
            self.p.expect("(")?;
            if self.p.eat("partition") {
                self.p.expect("by")?;
                self.expr_list()?
            }
            if self.p.eat("order") {
                self.p.expect("by")?;
                self.order()?
            }
            self.p.expect(")")?;
        } else if ["row_number", "rank", "dense_rank"].contains(&name) {
            return self.p.err("ranking function requires OVER");
        }
        e.name.clear();
        Ok(e)
    }
}
