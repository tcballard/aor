use crate::{Error, Result};
use tokio_postgres::types::{IsNull, ToSql, Type};
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    I32(i32),
    I64(i64),
    Text(String),
    Enum(String),
    Uuid(uuid::Uuid),
    Timestamp(chrono::DateTime<chrono::Utc>),
    Bytes(Vec<u8>),
}
impl ToSql for Value {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            Self::Null => Ok(IsNull::Yes),
            Self::Bool(v) => v.to_sql(ty, out),
            Self::I32(v) => v.to_sql(ty, out),
            Self::I64(v) => v.to_sql(ty, out),
            Self::Text(v) | Self::Enum(v) => v.to_sql(ty, out),
            Self::Uuid(v) => v.to_sql(ty, out),
            Self::Timestamp(v) => v.to_sql(ty, out),
            Self::Bytes(v) => v.to_sql(ty, out),
        }
    }
    fn accepts(_: &Type) -> bool {
        true
    }
    tokio_postgres::types::to_sql_checked!();
}
pub trait IntoValue {
    fn into_value(self) -> Value;
}
pub trait Decode: Sized {
    fn decode(v: &Value) -> Result<Self>;
}
macro_rules! values {($($ty:ty=>$variant:ident),*)=>{$(impl IntoValue for $ty{fn into_value(self)->Value{Value::$variant(self)}}impl Decode for $ty{fn decode(v:&Value)->Result<Self>{if let Value::$variant(v)=v{Ok(v.clone())}else{Err(Error::Decode(format!("expected {}",stringify!($ty))))}}})*};}
values!(i64=>I64,String=>Text,uuid::Uuid=>Uuid,chrono::DateTime<chrono::Utc> =>Timestamp,Vec<u8> =>Bytes);
impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        self.map_or(Value::Null, IntoValue::into_value)
    }
}
impl<T: Decode> Decode for Option<T> {
    fn decode(v: &Value) -> Result<Self> {
        if *v == Value::Null {
            Ok(None)
        } else {
            T::decode(v).map(Some)
        }
    }
}
pub fn parameter(v: impl IntoValue, is_enum: bool) -> Value {
    match v.into_value() {
        Value::Text(s) if is_enum => Value::Enum(s),
        v => v,
    }
}
#[derive(Debug)]
pub struct Row(pub(crate) Vec<Value>);
impl Row {
    pub fn get<T: Decode>(&self, index: usize) -> Result<T> {
        T::decode(
            self.0
                .get(index)
                .ok_or_else(|| Error::Decode("column index out of range".into()))?,
        )
    }
}
pub(crate) fn pg_row(row: tokio_postgres::Row) -> Result<Row> {
    let mut out = Vec::new();
    for (i, col) in row.columns().iter().enumerate() {
        macro_rules! get {
            ($ty:ty,$variant:ident) => {
                row.try_get::<_, Option<$ty>>(i)?
                    .map_or(Value::Null, Value::$variant)
            };
        }
        out.push(match *col.type_() {
            Type::BOOL => get!(bool, Bool),
            Type::INT4 => get!(i32, I32),
            Type::INT8 => get!(i64, I64),
            Type::TEXT | Type::VARCHAR => get!(String, Text),
            Type::UUID => get!(uuid::Uuid, Uuid),
            Type::TIMESTAMPTZ => get!(chrono::DateTime<chrono::Utc>, Timestamp),
            Type::BYTEA => get!(Vec<u8>, Bytes),
            _ => {
                if matches!(col.type_().kind(), tokio_postgres::types::Kind::Enum(_)) {
                    struct EnumText(String);
                    impl<'a> tokio_postgres::types::FromSql<'a> for EnumText {
                        fn from_sql(
                            _: &Type,
                            raw: &'a [u8],
                        ) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>>
                        {
                            Ok(Self(std::str::from_utf8(raw)?.into()))
                        }
                        fn accepts(t: &Type) -> bool {
                            matches!(t.kind(), tokio_postgres::types::Kind::Enum(_))
                        }
                    }
                    row.try_get::<_, Option<EnumText>>(i)?
                        .map_or(Value::Null, |v| Value::Text(v.0))
                } else {
                    return Err(Error::Decode(format!(
                        "unsupported PostgreSQL result type {}",
                        col.type_()
                    )));
                }
            }
        });
    }
    Ok(Row(out))
}
pub(crate) fn sqlite_value(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match v {
        Value::Null => S::Null,
        Value::Bool(v) => S::Integer(i64::from(*v)),
        Value::I32(v) => S::Integer((*v).into()),
        Value::I64(v) => S::Integer(*v),
        Value::Text(v) | Value::Enum(v) => S::Text(v.clone()),
        Value::Uuid(v) => S::Text(v.to_string()),
        Value::Timestamp(v) => S::Text(v.to_rfc3339()),
        Value::Bytes(v) => S::Blob(v.clone()),
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}
impl Decode for bool {
    fn decode(v: &Value) -> Result<Self> {
        match v {
            Value::Bool(b) => Ok(*b),
            Value::I64(0) => Ok(false),
            Value::I64(1) => Ok(true),
            _ => Err(Error::Decode("expected bool".into())),
        }
    }
}
impl IntoValue for i32 {
    fn into_value(self) -> Value {
        Value::I32(self)
    }
}
impl Decode for i32 {
    fn decode(v: &Value) -> Result<Self> {
        match v {
            Value::I32(n) => Ok(*n),
            Value::I64(n) => i32::try_from(*n).map_err(|_| Error::Decode("i32 overflow".into())),
            _ => Err(Error::Decode("expected i32".into())),
        }
    }
}
