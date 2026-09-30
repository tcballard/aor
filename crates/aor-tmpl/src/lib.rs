//! Interpreted templates with derived context schemas and escaped text output.
//! The supported grammar is deliberately explicit; unsupported tags are errors.
extern crate self as aor_tmpl;
pub use aor_macros::TemplateContext;
use std::collections::BTreeMap;
pub mod theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Schema {
    Text,
    Number,
    Bool,
    Trusted,
    List(Box<Schema>),
    Object(BTreeMap<String, Schema>),
}
#[derive(Clone, Debug)]
pub enum Value {
    Text(String),
    Number(String),
    Bool(bool),
    Trusted(TrustedHtml),
    List(Vec<Value>),
    Object(BTreeMap<String, Value>),
}
pub trait TemplateContext {
    fn schema() -> Schema;
    fn value(&self) -> Value;
}
impl TemplateContext for String {
    fn schema() -> Schema {
        Schema::Text
    }
    fn value(&self) -> Value {
        Value::Text(self.clone())
    }
}
impl TemplateContext for &str {
    fn schema() -> Schema {
        Schema::Text
    }
    fn value(&self) -> Value {
        Value::Text((*self).to_owned())
    }
}
impl TemplateContext for bool {
    fn schema() -> Schema {
        Schema::Bool
    }
    fn value(&self) -> Value {
        Value::Bool(*self)
    }
}
macro_rules! numbers {($($t:ty),*)=>{$(impl TemplateContext for $t{fn schema()->Schema{Schema::Number}fn value(&self)->Value{Value::Number(self.to_string())}})*};}
numbers!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);
impl<T: TemplateContext> TemplateContext for Vec<T> {
    fn schema() -> Schema {
        Schema::List(Box::new(T::schema()))
    }
    fn value(&self) -> Value {
        Value::List(self.iter().map(TemplateContext::value).collect())
    }
}
#[derive(Clone, Debug)]
pub struct TrustedHtml(String);
impl TrustedHtml {
    /// Named sanitiser: this initial implementation accepts plain text only and escapes it.
    /// There is no unchecked HTML constructor.
    pub fn sanitise_text(text: &str) -> Self {
        Self(escape(text))
    }
}
/// Constructs a link from a canonical local ASCII path and escaped label.
/// Rejects schemes, protocol-relative URLs, traversal, attributes and query strings.
pub fn sanitise_local_link(path: &str, label: &str) -> Result<TrustedHtml, &'static str> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.len() > 2048
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-".contains(&b))
    {
        return Err("link requires a canonical local path");
    }
    Ok(TrustedHtml(format!(
        "<a href=\"{}\">{}</a>",
        path,
        escape(label)
    )))
}
impl TemplateContext for TrustedHtml {
    fn schema() -> Schema {
        Schema::Trusted
    }
    fn value(&self) -> Value {
        Value::Trusted(self.clone())
    }
}
pub fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub line: usize,
    pub message: String,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "template:{}: {}", self.line, self.message)
    }
}
impl std::error::Error for Error {}
fn err(line: usize, message: impl Into<String>) -> Error {
    Error {
        line,
        message: message.into(),
    }
}
#[derive(Clone, Debug)]
struct Expression {
    path: Vec<String>,
    filter: Option<String>,
    line: usize,
}
#[derive(Clone, Debug)]
enum Node {
    Text(String),
    Print(Expression),
    If(Expression, Vec<Node>, Vec<Node>),
    For(String, Expression, Vec<Node>),
}
#[derive(Clone, Debug)]
pub struct Template {
    nodes: Vec<Node>,
}
#[derive(Clone, Debug)]
enum Token {
    Text(String),
    Print(String, usize),
    Tag(String, usize),
}
fn expression(s: &str, line: usize) -> Result<Expression, Error> {
    let mut parts = s.split('|');
    let path = parts.next().unwrap_or("").trim();
    let filter = parts.next().map(str::trim);
    if parts.next().is_some()
        || path.is_empty()
        || path.split('.').any(|p| {
            p.is_empty()
                || !p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || p.as_bytes()[0].is_ascii_digit()
        })
    {
        return Err(err(line, "expected a field path and at most one filter"));
    }
    if filter.is_some_and(|f| !["trusted", "upper", "lower"].contains(&f)) {
        return Err(err(line, "unknown filter"));
    }
    Ok(Expression {
        path: path.split('.').map(str::to_owned).collect(),
        filter: filter.map(str::to_owned),
        line,
    })
}
/// Dynamic output is supported in HTML text nodes only, not attributes or script/style.
/// Templates are application code; interpolation context is checked, not guessed.
fn tokenize(input: &str) -> Result<Vec<Token>, Error> {
    if input.len() > 1024 * 1024 {
        return Err(err(1, "template exceeds 1 MiB"));
    }
    let mut tokens = Vec::new();
    let mut offset = 0;
    let mut line = 1;
    while offset < input.len() {
        let rest = &input[offset..];
        let next = ["{{", "{%", "{#"].iter().filter_map(|m| rest.find(m)).min();
        let Some(next) = next else {
            tokens.push(Token::Text(rest.to_owned()));
            break;
        };
        let text = &rest[..next];
        tokens.push(Token::Text(text.to_owned()));
        line += text.bytes().filter(|b| *b == b'\n').count();
        offset += next;
        let open = &input[offset..offset + 2];
        let close = match open {
            "{{" => "}}",
            "{%" => "%}",
            _ => "#}",
        };
        let end = input[offset + 2..]
            .find(close)
            .ok_or_else(|| err(line, "unclosed template delimiter"))?
            + offset
            + 2;
        let raw = input[offset + 2..end].trim();
        if open != "{#" {
            let prefix = &input[..offset];
            let in_tag = prefix
                .rfind('<')
                .is_some_and(|lt| prefix.rfind('>').is_none_or(|gt| lt > gt));
            let lower = prefix.to_ascii_lowercase();
            let raw_text = ["script", "style", "textarea", "title"].iter().any(|tag| {
                lower.rfind(&format!("<{tag}")).is_some_and(|start| {
                    lower
                        .rfind(&format!("</{tag}"))
                        .is_none_or(|end| start > end)
                })
            });
            if in_tag || raw_text {
                return Err(err(line, "dynamic output is restricted to HTML text nodes"));
            }
            tokens.push(if open == "{{" {
                Token::Print(raw.to_owned(), line)
            } else {
                Token::Tag(raw.to_owned(), line)
            });
        }
        line += input[offset..end + 2]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        offset = end + 2;
    }
    Ok(tokens)
}
type ParsedNodes = (Vec<Node>, Option<(String, usize)>);
fn parse_nodes(tokens: &[Token], index: &mut usize, depth: usize) -> Result<ParsedNodes, Error> {
    if depth > 32 {
        return Err(err(1, "template nesting exceeds 32"));
    }
    let mut nodes = Vec::new();
    while *index < tokens.len() {
        let token = &tokens[*index];
        *index += 1;
        match token {
            Token::Text(s) => nodes.push(Node::Text(s.clone())),
            Token::Print(s, l) => nodes.push(Node::Print(expression(s, *l)?)),
            Token::Tag(tag, l) => {
                if ["else", "endif", "endfor"].contains(&tag.as_str()) {
                    return Ok((nodes, Some((tag.clone(), *l))));
                }
                if let Some(expr) = tag.strip_prefix("if ") {
                    let condition = expression(expr, *l)?;
                    if condition.filter.is_some() {
                        return Err(err(*l, "filters are not conditions"));
                    }
                    let (yes, end) = parse_nodes(tokens, index, depth + 1)?;
                    let (no, end) = if end.as_ref().is_some_and(|(s, _)| s == "else") {
                        parse_nodes(tokens, index, depth + 1)?
                    } else {
                        (vec![], end)
                    };
                    if end.is_none_or(|(s, _)| s != "endif") {
                        return Err(err(*l, "if requires endif"));
                    }
                    nodes.push(Node::If(condition, yes, no));
                } else if let Some(expr) = tag.strip_prefix("for ") {
                    let (name, expr) = expr
                        .split_once(" in ")
                        .ok_or_else(|| err(*l, "expected for name in list"))?;
                    let name_expr = expression(name, *l)?;
                    if name_expr.path.len() != 1 || name_expr.filter.is_some() {
                        return Err(err(*l, "invalid loop binding"));
                    }
                    let expr = expression(expr, *l)?;
                    if expr.filter.is_some() {
                        return Err(err(*l, "filters are not lists"));
                    }
                    let (body, end) = parse_nodes(tokens, index, depth + 1)?;
                    if end.is_none_or(|(s, _)| s != "endfor") {
                        return Err(err(*l, "for requires endfor"));
                    }
                    nodes.push(Node::For(name.to_owned(), expr, body));
                } else {
                    return Err(err(*l, format!("unsupported tag: {tag}")));
                }
            }
        }
    }
    Ok((nodes, None))
}
fn schema_at<'a>(
    schema: &'a Schema,
    locals: &'a BTreeMap<String, Schema>,
    e: &Expression,
) -> Result<&'a Schema, Error> {
    let mut current = if let Some(local) = locals.get(&e.path[0]) {
        local
    } else {
        let Schema::Object(fields) = schema else {
            return Err(err(e.line, "root context must be a struct"));
        };
        fields
            .get(&e.path[0])
            .ok_or_else(|| err(e.line, format!("unknown field {}", e.path[0])))?
    };
    for part in &e.path[1..] {
        let Schema::Object(fields) = current else {
            return Err(err(e.line, "field access on a scalar"));
        };
        current = fields
            .get(part)
            .ok_or_else(|| err(e.line, format!("unknown field {part}")))?;
    }
    Ok(current)
}
fn validate(
    nodes: &[Node],
    schema: &Schema,
    locals: &BTreeMap<String, Schema>,
) -> Result<(), Error> {
    for node in nodes {
        match node {
            Node::Text(_) => {}
            Node::Print(e) => {
                let ty = schema_at(schema, locals, e)?;
                match e.filter.as_deref() {
                    Some("trusted") if *ty == Schema::Trusted => {}
                    Some("upper" | "lower") if *ty == Schema::Text => {}
                    None if matches!(ty, Schema::Text | Schema::Bool | Schema::Number) => {}
                    _ => {
                        return Err(err(
                            e.line,
                            "invalid output type or filter; trusted requires TrustedHtml",
                        ));
                    }
                }
            }
            Node::If(e, yes, no) => {
                if *schema_at(schema, locals, e)? != Schema::Bool {
                    return Err(err(e.line, "if requires bool"));
                }
                validate(yes, schema, locals)?;
                validate(no, schema, locals)?;
            }
            Node::For(name, e, body) => {
                let Schema::List(inner) = schema_at(schema, locals, e)? else {
                    return Err(err(e.line, "for requires a list"));
                };
                let mut locals = locals.clone();
                locals.insert(name.clone(), inner.as_ref().clone());
                validate(body, schema, &locals)?;
            }
        }
    }
    Ok(())
}
fn value_at<'a>(
    value: &'a Value,
    locals: &'a BTreeMap<String, Value>,
    e: &Expression,
) -> Result<&'a Value, Error> {
    let mut current = if let Some(local) = locals.get(&e.path[0]) {
        local
    } else {
        let Value::Object(fields) = value else {
            return Err(err(e.line, "invalid root value"));
        };
        fields
            .get(&e.path[0])
            .ok_or_else(|| err(e.line, "missing value"))?
    };
    for part in &e.path[1..] {
        let Value::Object(fields) = current else {
            return Err(err(e.line, "invalid object value"));
        };
        current = fields
            .get(part)
            .ok_or_else(|| err(e.line, "missing value"))?;
    }
    Ok(current)
}
fn render(
    nodes: &[Node],
    value: &Value,
    locals: &BTreeMap<String, Value>,
    out: &mut String,
    budget: &mut usize,
) -> Result<(), Error> {
    for node in nodes {
        if *budget == 0 {
            return Err(err(1, "template evaluation limit exceeded"));
        }
        *budget -= 1;
        match node {
            Node::Text(s) => out.push_str(s),
            Node::Print(e) => {
                let value = value_at(value, locals, e)?;
                let text = match value {
                    Value::Text(s) | Value::Number(s) => s.clone(),
                    Value::Bool(b) => b.to_string(),
                    Value::Trusted(t) => t.0.clone(),
                    _ => return Err(err(e.line, "invalid scalar")),
                };
                match e.filter.as_deref() {
                    Some("trusted") => out.push_str(&text),
                    Some("upper") => out.push_str(&escape(&text.to_uppercase())),
                    Some("lower") => out.push_str(&escape(&text.to_lowercase())),
                    _ => out.push_str(&escape(&text)),
                }
            }
            Node::If(e, yes, no) => {
                let Value::Bool(b) = value_at(value, locals, e)? else {
                    return Err(err(e.line, "invalid bool"));
                };
                render(if *b { yes } else { no }, value, locals, out, budget)?;
            }
            Node::For(name, e, body) => {
                let Value::List(values) = value_at(value, locals, e)? else {
                    return Err(err(e.line, "invalid list"));
                };
                for item in values {
                    let mut locals = locals.clone();
                    locals.insert(name.clone(), item.clone());
                    render(body, value, &locals, out, budget)?;
                }
            }
        }
        if out.len() > 8 * 1024 * 1024 {
            return Err(err(1, "render exceeds 8 MiB"));
        }
    }
    Ok(())
}
impl Template {
    pub fn parse(input: &str) -> Result<Self, Error> {
        let tokens = tokenize(input)?;
        let (nodes, end) = parse_nodes(&tokens, &mut 0, 0)?;
        if let Some((end, line)) = end {
            return Err(err(line, format!("unexpected {end}")));
        }
        Ok(Self { nodes })
    }
    pub fn check<C: TemplateContext>(&self) -> Result<(), Error> {
        validate(&self.nodes, &C::schema(), &BTreeMap::new())
    }
    pub fn render<C: TemplateContext>(&self, context: &C) -> Result<String, Error> {
        self.check::<C>()?;
        let mut out = String::new();
        render(
            &self.nodes,
            &context.value(),
            &BTreeMap::new(),
            &mut out,
            &mut 100_000,
        )?;
        Ok(out)
    }
    /// Development reload: parse and type-check current file on each request.
    pub fn render_file<C: TemplateContext>(
        path: &std::path::Path,
        context: &C,
    ) -> Result<String, Error> {
        let source = std::fs::read_to_string(path).map_err(|e| err(1, e.to_string()))?;
        Self::parse(&source)?.render(context)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(TemplateContext)]
    struct Item {
        name: String,
    }
    #[derive(TemplateContext)]
    struct Page {
        title: String,
        show: bool,
        items: Vec<Item>,
        html: TrustedHtml,
    }
    fn page() -> Page {
        Page {
            title: "<script>alert('x')</script>".into(),
            show: true,
            items: vec![Item { name: "A&B".into() }],
            html: TrustedHtml::sanitise_text("<b>"),
        }
    }
    #[test]
    fn typed_loops_branches_and_escaping() {
        let t=Template::parse("<h1>{{ title }}</h1>{% if show %}{% for item in items %}<p>{{ item.name }}</p>{% endfor %}{% else %}hidden{% endif %}{{ html | trusted }}").unwrap();
        assert_eq!(
            t.render(&page()).unwrap(),
            "<h1>&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;</h1><p>A&amp;B</p>&lt;b&gt;"
        );
    }
    #[test]
    fn schema_checks_even_unrendered_branch() {
        let e = Template::parse("{% if show %}yes{% else %}{{ missing }}{% endif %}")
            .unwrap()
            .check::<Page>()
            .unwrap_err();
        assert!(e.message.contains("unknown field"));
    }
    #[test]
    fn raw_strings_and_wrong_types_rejected() {
        for s in [
            "{{ title | trusted }}",
            "{% if title %}x{% endif %}",
            "{% for i in title %}x{% endfor %}",
            "{{ items }}",
        ] {
            assert!(Template::parse(s).unwrap().check::<Page>().is_err());
        }
    }
    #[test]
    fn dynamic_attributes_scripts_and_unsupported_tags_rejected() {
        for s in [
            "<a href=\"{{ title }}\">x</a>",
            "<script>{{ title }}</script>",
            "<style>{{ title }}</style>",
            "{% include 'file' %}",
            "{% endif %}",
            "{{ title",
        ] {
            assert!(Template::parse(s).is_err(), "{s}");
        }
    }
    #[test]
    fn unicode_and_line_diagnostics() {
        let e = Template::parse("hello 🐧\n{{ absent }}")
            .unwrap()
            .check::<Page>()
            .unwrap_err();
        assert_eq!(e.line, 2);
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    #[test]
    fn local_link_sanitiser_rejects_url_and_attribute_injection() {
        for path in [
            "javascript:alert(1)",
            "//evil.example",
            "/../secret",
            "/x\" onclick=\"bad",
            "/%2fsecret",
            "/x?next=bad",
        ] {
            assert!(sanitise_local_link(path, "label").is_err(), "{path}");
        }
        assert_eq!(
            sanitise_local_link("/editions/a-note", "<script>\"&")
                .unwrap()
                .0,
            "<a href=\"/editions/a-note\">&lt;script&gt;&quot;&amp;</a>"
        );
    }
}
