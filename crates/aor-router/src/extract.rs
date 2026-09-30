//! Closed handler extractor set. Application types customize data, not extraction order.
use crate::{AppError, Context, Params, RequestId};
use serde::de::{self, DeserializeOwned, IntoDeserializer, Visitor};
pub struct Path<T>(pub T);
pub struct Query<T>(pub T);
pub struct Form<T>(pub T);
pub struct Json<T>(pub T);
pub struct Body(pub aor_http::Body);
/// Reserved extractors fail closed until the Level 3 authenticated boundary exists.
pub struct Session {
    _private: (),
}
pub struct Principal {
    _private: (),
}
pub trait FromPath: Sized {
    fn from_path(parameters: &Params) -> Result<Self, AppError>;
}
#[doc(hidden)]
pub struct Extraction {
    path: Params,
    query: String,
    headers: Vec<(String, Vec<u8>)>,
    body: Option<aor_http::Body>,
    id: RequestId,
}
impl Extraction {
    pub fn new(ctx: Context) -> Self {
        Self {
            path: ctx.path,
            query: ctx
                .request
                .target
                .split_once('?')
                .map_or("", |(_, q)| q)
                .into(),
            headers: ctx.request.headers,
            body: Some(ctx.request.body),
            id: ctx.request_id,
        }
    }
    fn media(&self, expected: &str) -> Result<(), AppError> {
        let mut headers = self
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("content-type"));
        let Some((_, v)) = headers.next() else {
            return Err(AppError::UnsupportedMediaType);
        };
        if headers.next().is_some() {
            return Err(AppError::UnsupportedMediaType);
        }
        let media = std::str::from_utf8(v)
            .map_err(|_| AppError::UnsupportedMediaType)?
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        if media.eq_ignore_ascii_case(expected) {
            Ok(())
        } else {
            Err(AppError::UnsupportedMediaType)
        }
    }
    fn body(&mut self) -> Result<aor_http::Body, AppError> {
        self.body.take().ok_or(AppError::BadJson)
    }
}
mod sealed {
    pub trait Sealed {}
}
#[doc(hidden)]
pub trait Extract: sealed::Sealed + Sized {
    fn extract(
        parts: &mut Extraction,
    ) -> impl std::future::Future<Output = Result<Self, AppError>> + Send;
}
impl<T: FromPath + Send> sealed::Sealed for Path<T> {}
impl<T: FromPath + Send> Extract for Path<T> {
    async fn extract(p: &mut Extraction) -> Result<Self, AppError> {
        T::from_path(&p.path).map(Self)
    }
}
impl<T: DeserializeOwned + Send> sealed::Sealed for Query<T> {}
impl<T: DeserializeOwned + Send> Extract for Query<T> {
    async fn extract(p: &mut Extraction) -> Result<Self, AppError> {
        url_decode(&p.query).map(Self)
    }
}
impl<T: DeserializeOwned + Send> sealed::Sealed for Form<T> {}
impl<T: DeserializeOwned + Send> Extract for Form<T> {
    async fn extract(p: &mut Extraction) -> Result<Self, AppError> {
        p.media("application/x-www-form-urlencoded")?;
        let bytes = p
            .body()?
            .collect()
            .await
            .map_err(|_| AppError::BodyTooLarge)?;
        url_decode(std::str::from_utf8(&bytes).map_err(|_| AppError::BadForm)?)
            .map(Self)
            .map_err(|_| AppError::BadForm)
    }
}
impl<T: DeserializeOwned + Send> sealed::Sealed for Json<T> {}
impl<T: DeserializeOwned + Send> Extract for Json<T> {
    async fn extract(p: &mut Extraction) -> Result<Self, AppError> {
        p.media("application/json")?;
        let bytes = p
            .body()?
            .collect()
            .await
            .map_err(|_| AppError::BodyTooLarge)?;
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|_| AppError::BadJson)
    }
}
impl sealed::Sealed for Body {}
impl Extract for Body {
    async fn extract(p: &mut Extraction) -> Result<Self, AppError> {
        p.body().map(Self)
    }
}
impl sealed::Sealed for RequestId {}
impl Extract for RequestId {
    async fn extract(p: &mut Extraction) -> Result<Self, AppError> {
        Ok(p.id.clone())
    }
}
impl sealed::Sealed for Session {}
impl Extract for Session {
    async fn extract(_: &mut Extraction) -> Result<Self, AppError> {
        Err(AppError::AuthenticationUnavailable)
    }
}
impl sealed::Sealed for Principal {}
impl Extract for Principal {
    async fn extract(_: &mut Extraction) -> Result<Self, AppError> {
        Err(AppError::AuthenticationUnavailable)
    }
}
pub fn url_decode<T: DeserializeOwned>(input: &str) -> Result<T, AppError> {
    let values = crate::query(input).map_err(|_| AppError::BadQuery)?;
    T::deserialize(de::value::MapDeserializer::new(
        values.iter().map(|(k, v)| (k.as_str(), Scalar(v))),
    ))
    .map_err(|_| AppError::BadQuery)
}
struct Scalar<'a>(&'a str);
macro_rules! number{($($name:ident,$visit:ident,$t:ty);*$(;)?)=>{$(fn $name<V:Visitor<'de>>(self,visitor:V)->Result<V::Value,Self::Error>{visitor.$visit(self.0.parse::<$t>().map_err(de::Error::custom)?)})*};}
impl<'de> de::Deserializer<'de> for Scalar<'de> {
    type Error = de::value::Error;
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_borrowed_str(self.0)
    }
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_enum(self.0.into_deserializer())
    }
    number!(deserialize_bool,visit_bool,bool;deserialize_i8,visit_i8,i8;deserialize_i16,visit_i16,i16;deserialize_i32,visit_i32,i32;deserialize_i64,visit_i64,i64;deserialize_u8,visit_u8,u8;deserialize_u16,visit_u16,u16;deserialize_u32,visit_u32,u32;deserialize_u64,visit_u64,u64;deserialize_f32,visit_f32,f32;deserialize_f64,visit_f64,f64);
    serde::forward_to_deserialize_any! {char str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct identifier ignored_any}
}
impl<'de> IntoDeserializer<'de, de::value::Error> for Scalar<'de> {
    type Deserializer = Self;
    fn into_deserializer(self) -> Self {
        self
    }
}
/// Generate a named path struct with explicit field types, including application IDs.
#[macro_export]
macro_rules! route_path{($vis:vis $name:ident { $($field:ident:$ty:ty),* $(,)? })=>{
 #[derive(Debug)] $vis struct $name{$(pub $field:$ty),*}
 impl $crate::FromPath for $name{fn from_path(params:&$crate::Params)->Result<Self,$crate::AppError>{Ok(Self{$($field:params.get::<$ty>(stringify!($field))?),*})}}
};}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer(String);
impl std::str::FromStr for SemVer {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > 128 {
            return Err(AppError::BadPath);
        }
        let (core, build) = s.split_once('+').map_or((s, None), |(a, b)| (a, Some(b)));
        let (core, pre) = core
            .split_once('-')
            .map_or((core, None), |(a, b)| (a, Some(b)));
        let nums: Vec<_> = core.split('.').collect();
        if nums.len() != 3
            || nums.iter().any(|n| {
                n.is_empty()
                    || n.len() > 1 && n.starts_with('0')
                    || !n.bytes().all(|b| b.is_ascii_digit())
                    || n.parse::<u64>().is_err()
            })
        {
            return Err(AppError::BadPath);
        }
        for (parts, leading_zero) in [(pre, true), (build, false)] {
            if let Some(parts) = parts {
                for part in parts.split('.') {
                    if part.is_empty()
                        || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                        || leading_zero
                            && part.len() > 1
                            && part.starts_with('0')
                            && part.bytes().all(|b| b.is_ascii_digit())
                    {
                        return Err(AppError::BadPath);
                    }
                }
            }
        }
        Ok(Self(s.into()))
    }
}
impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
