use aor_db::{DateTime, Dialect, Pool, PoolOptions, Utc, Uuid, sql};
use serde::Deserialize;
use std::io;
#[allow(dead_code)]
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/schema.rs"));
}
sql!(
    ListPg,
    postgres,
    "migrations/postgres",
    "SELECT slug, title, published_at FROM editions ORDER BY published_at DESC LIMIT $1"
);
sql!(
    GetPg,
    postgres,
    "migrations/postgres",
    "SELECT title, body, published_at FROM editions WHERE slug = $1"
);
sql!(
    SavePg,
    postgres,
    "migrations/postgres",
    "INSERT INTO editions (id, slug, title, body, published_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (slug) DO UPDATE SET title = excluded.title, body = excluded.body, published_at = excluded.published_at, version = editions.version + 1 RETURNING id, version"
);
sql!(
    ListLocal,
    sqlite,
    "migrations/sqlite",
    "SELECT slug, title, published_at FROM editions ORDER BY published_at DESC LIMIT $1"
);
sql!(
    GetLocal,
    sqlite,
    "migrations/sqlite",
    "SELECT title, body, published_at FROM editions WHERE slug = $1"
);
sql!(
    SaveLocal,
    sqlite,
    "migrations/sqlite",
    "INSERT INTO editions (id, slug, title, body, published_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (slug) DO UPDATE SET title = excluded.title, body = excluded.body, published_at = excluded.published_at, version = editions.version + 1 RETURNING id, version"
);
#[derive(Clone)]
pub struct Store(pub Pool);
#[derive(aor_tmpl::TemplateContext)]
pub struct Edition {
    pub title: String,
    pub body: String,
    pub date: String,
}
#[derive(aor_tmpl::TemplateContext)]
pub struct Summary {
    pub title: String,
    pub date: String,
    pub link: aor_tmpl::TrustedHtml,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportEdition {
    pub slug: String,
    pub title: String,
    pub body: String,
    pub published_at: String,
}
impl Store {
    pub fn open(local: Option<&str>) -> io::Result<Option<Self>> {
        if let Some(path) = local {
            return Ok(Some(Self(
                Pool::sqlite(path, PoolOptions::default()).map_err(io::Error::other)?,
            )));
        }
        match std::env::var("AOR_DATABASE_URL") {
            Ok(url) => Ok(Some(Self(
                Pool::postgres(&url, PoolOptions::default()).map_err(io::Error::other)?,
            ))),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(e) => Err(io::Error::other(e)),
        }
    }
    pub async fn migrate(&self) -> io::Result<usize> {
        let sql = match self.0.dialect() {
            Dialect::Postgres => include_str!("../migrations/postgres/001_editions.sql"),
            Dialect::Sqlite => include_str!("../migrations/sqlite/001_editions.sql"),
        };
        let migrations = aor_migrate::embedded(&[("001_editions.sql", sql)], self.0.dialect())
            .map_err(io::Error::other)?;
        aor_migrate::apply(&self.0, &migrations)
            .await
            .map_err(io::Error::other)
    }
    pub async fn list(&self) -> aor_db::Result<Vec<Summary>> {
        let mut conn = self.0.acquire().await?;
        let records = match self.0.dialect() {
            Dialect::Postgres => ListPg::query(&mut conn, 100)
                .await?
                .into_iter()
                .map(|r| {
                    (
                        r.slug,
                        r.title,
                        r.published_at.format("%Y-%m-%d UTC").to_string(),
                    )
                })
                .collect::<Vec<_>>(),
            Dialect::Sqlite => ListLocal::query(&mut conn, 100)
                .await?
                .into_iter()
                .map(|r| (r.slug, r.title, r.published_at))
                .collect(),
        };
        records
            .into_iter()
            .map(|(slug, title, date)| {
                let link = aor_tmpl::sanitise_local_link(&format!("/editions/{slug}"), &title)
                    .map_err(|e| aor_db::Error::Decode(e.into()))?;
                Ok(Summary { title, date, link })
            })
            .collect()
    }
    pub async fn edition(&self, slug: String) -> aor_db::Result<Option<Edition>> {
        let mut conn = self.0.acquire().await?;
        Ok(match self.0.dialect() {
            Dialect::Postgres => GetPg::query(&mut conn, slug)
                .await?
                .into_iter()
                .next()
                .map(|r| Edition {
                    title: r.title,
                    body: r.body,
                    date: r.published_at.format("%Y-%m-%d %H:%M UTC").to_string(),
                }),
            Dialect::Sqlite => GetLocal::query(&mut conn, slug)
                .await?
                .into_iter()
                .next()
                .map(|r| Edition {
                    title: r.title,
                    body: r.body,
                    date: r.published_at,
                }),
        })
    }
    pub async fn import(&self, file: &std::path::Path) -> io::Result<usize> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(file)?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(io::Error::other("import exceeds 8 MiB"));
        }
        let editions: Vec<ImportEdition> =
            serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if editions.len() > 1000 {
            return Err(io::Error::other("import exceeds 1,000 editions"));
        }
        let mut validated = Vec::new();
        let mut slugs = std::collections::BTreeSet::new();
        for e in editions {
            e.slug
                .parse::<aor_router::Slug>()
                .map_err(|_| io::Error::other("invalid edition slug"))?;
            if !slugs.insert(e.slug.clone())
                || e.title.is_empty()
                || e.title.len() > 256
                || e.body.len() > 256 * 1024
            {
                return Err(io::Error::other("duplicate slug or invalid edition size"));
            }
            let date = DateTime::parse_from_rfc3339(&e.published_at)
                .map_err(io::Error::other)?
                .with_timezone(&Utc);
            validated.push((e, date));
        }
        let mut conn = self.0.acquire().await.map_err(io::Error::other)?;
        let mut tx = conn.begin().await.map_err(io::Error::other)?;
        let count = validated.len();
        for (e, date) in validated {
            let id = Uuid::new_v4();
            match self.0.dialect() {
                Dialect::Postgres => {
                    SavePg::execute(&mut tx, id, e.slug, e.title, e.body, date)
                        .await
                        .map_err(io::Error::other)?;
                }
                Dialect::Sqlite => {
                    SaveLocal::execute(
                        &mut tx,
                        id.to_string(),
                        e.slug,
                        e.title,
                        e.body,
                        date.format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string(),
                    )
                    .await
                    .map_err(io::Error::other)?;
                }
            }
        }
        tx.commit().await.map_err(io::Error::other)?;
        Ok(count)
    }
}
