use crate::{State, db_error, policy_error};
use aor_db::{Tx, Uuid};
use aor_policy::{Authorized, Create, Delete, Read, Resource, Update};
use aor_router::AppError;
use serde::{Deserialize, Serialize};
pub enum Version {}
impl Resource for Version {
    const NAME: &'static str = "versions";
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub name: String,
    pub plugin_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub name: String,
    pub version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Removal {
    pub version: i64,
}
#[derive(Serialize)]
pub struct Output {
    pub id: Uuid,
    pub name: String,
    pub version: i64,
    pub plugin_id: Uuid,
}
fn name(value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(AppError::BadJson)
    } else {
        Ok(())
    }
}
mod repository {
    use super::*;
    aor_db::sql!(
        List,
        portable,
        "migrations",
        "SELECT id, owner_id, plugin_id, name, version FROM versions WHERE owner_id = $1 ORDER BY id"
    );
    aor_db::sql!(
        Find,
        portable,
        "migrations",
        "SELECT id, owner_id, plugin_id, name, version FROM versions WHERE id = $1 AND owner_id = $2"
    );
    aor_db::sql!(
        Insert,
        portable,
        "migrations",
        "INSERT INTO versions (id, owner_id, plugin_id, name, version) VALUES ($1, $2, $3, $4, 1) RETURNING id, owner_id, plugin_id, name, version"
    );
    aor_db::sql!(
        UpdateRow,
        portable,
        "migrations",
        "UPDATE versions SET name = $1, version = version + 1 WHERE id = $2 AND owner_id = $3 AND version = $4 RETURNING id, owner_id, plugin_id, name, version"
    );
    aor_db::sql!(
        DeleteRow,
        portable,
        "migrations",
        "DELETE FROM versions WHERE id = $1 AND owner_id = $2 AND version = $3"
    );
    aor_db::sql!(
        Parent,
        portable,
        "migrations",
        "SELECT id FROM plugins WHERE id = $1 AND owner_id = $2"
    );
    impl TryFrom<ListRow> for Output {
        type Error = AppError;
        fn try_from(row: ListRow) -> Result<Self, AppError> {
            let _owner = row.owner_id;
            Ok(Self {
                id: row.id.parse().map_err(|_| AppError::Internal)?,
                name: row.name,
                version: row.version,
                plugin_id: row.plugin_id.parse().map_err(|_| AppError::Internal)?,
            })
        }
    }
    impl TryFrom<FindRow> for Output {
        type Error = AppError;
        fn try_from(row: FindRow) -> Result<Self, AppError> {
            let _owner = row.owner_id;
            Ok(Self {
                id: row.id.parse().map_err(|_| AppError::Internal)?,
                name: row.name,
                version: row.version,
                plugin_id: row.plugin_id.parse().map_err(|_| AppError::Internal)?,
            })
        }
    }
    impl TryFrom<InsertRow> for Output {
        type Error = AppError;
        fn try_from(row: InsertRow) -> Result<Self, AppError> {
            let _owner = row.owner_id;
            Ok(Self {
                id: row.id.parse().map_err(|_| AppError::Internal)?,
                name: row.name,
                version: row.version,
                plugin_id: row.plugin_id.parse().map_err(|_| AppError::Internal)?,
            })
        }
    }
    impl TryFrom<UpdateRowRow> for Output {
        type Error = AppError;
        fn try_from(row: UpdateRowRow) -> Result<Self, AppError> {
            let _owner = row.owner_id;
            Ok(Self {
                id: row.id.parse().map_err(|_| AppError::Internal)?,
                name: row.name,
                version: row.version,
                plugin_id: row.plugin_id.parse().map_err(|_| AppError::Internal)?,
            })
        }
    }
    pub async fn list(
        tx: &mut Tx<'_>,
        scope: &Authorized<Version, Read>,
    ) -> Result<Vec<Output>, AppError> {
        List::query(tx, scope.owner_id().to_string())
            .await
            .map_err(db_error)?
            .into_iter()
            .map(Output::try_from)
            .collect()
    }
    pub async fn get(
        tx: &mut Tx<'_>,
        scope: &Authorized<Version, Read>,
        id: Uuid,
    ) -> Result<Output, AppError> {
        Find::query(tx, id.to_string(), scope.owner_id().to_string())
            .await
            .map_err(db_error)?
            .pop()
            .ok_or(AppError::NotFound)?
            .try_into()
    }
    pub async fn create(
        tx: &mut Tx<'_>,
        scope: &Authorized<Version, Create>,
        input: Input,
    ) -> Result<Output, AppError> {
        if Parent::query(
            tx,
            input.plugin_id.to_string(),
            scope.owner_id().to_string(),
        )
        .await
        .map_err(db_error)?
        .is_empty()
        {
            return Err(AppError::NotFound);
        }
        Insert::query(
            tx,
            Uuid::new_v4().to_string(),
            scope.owner_id().to_string(),
            input.plugin_id.to_string(),
            input.name,
        )
        .await
        .map_err(db_error)?
        .pop()
        .ok_or(AppError::Internal)?
        .try_into()
    }
    pub async fn update(
        tx: &mut Tx<'_>,
        scope: &Authorized<Version, Update>,
        id: Uuid,
        input: Change,
    ) -> Result<Output, AppError> {
        if Find::query(tx, id.to_string(), scope.owner_id().to_string())
            .await
            .map_err(db_error)?
            .is_empty()
        {
            return Err(AppError::NotFound);
        }
        UpdateRow::query(
            tx,
            input.name,
            id.to_string(),
            scope.owner_id().to_string(),
            input.version,
        )
        .await
        .map_err(db_error)?
        .pop()
        .ok_or(AppError::Conflict)?
        .try_into()
    }
    pub async fn delete(
        tx: &mut Tx<'_>,
        scope: &Authorized<Version, Delete>,
        id: Uuid,
        input: Removal,
    ) -> Result<(), AppError> {
        if Find::query(tx, id.to_string(), scope.owner_id().to_string())
            .await
            .map_err(db_error)?
            .is_empty()
        {
            return Err(AppError::NotFound);
        }
        if DeleteRow::execute(
            tx,
            id.to_string(),
            scope.owner_id().to_string(),
            input.version,
        )
        .await
        .map_err(db_error)?
            != 1
        {
            return Err(AppError::Conflict);
        }
        Ok(())
    }
}
pub async fn list(
    state: &State,
    scope: Authorized<Version, Read>,
) -> Result<Vec<Output>, AppError> {
    let mut lease = state.pool.acquire().await.map_err(db_error)?;
    let mut tx = aor_tx::begin(&mut lease, &state.auth, &scope)
        .await
        .map_err(aor_router::auth_error)?;
    let result = repository::list(&mut tx, &scope).await;
    match result {
        Ok(out) => {
            tx.commit().await.map_err(db_error)?;
            Ok(out)
        }
        Err(e) => {
            tx.rollback().await.map_err(db_error)?;
            Err(e)
        }
    }
}
pub async fn get(
    state: &State,
    scope: Authorized<Version, Read>,
    id: Uuid,
) -> Result<Output, AppError> {
    let mut lease = state.pool.acquire().await.map_err(db_error)?;
    let mut tx = aor_tx::begin(&mut lease, &state.auth, &scope)
        .await
        .map_err(aor_router::auth_error)?;
    let result = repository::get(&mut tx, &scope, id).await;
    match result {
        Ok(out) => {
            tx.commit().await.map_err(db_error)?;
            Ok(out)
        }
        Err(e) => {
            tx.rollback().await.map_err(db_error)?;
            Err(e)
        }
    }
}
pub async fn create(
    state: &State,
    scope: Authorized<Version, Create>,
    input: Input,
) -> Result<Output, AppError> {
    name(&input.name)?;
    let mut lease = state.pool.acquire().await.map_err(db_error)?;
    let mut tx = aor_tx::begin(&mut lease, &state.auth, &scope)
        .await
        .map_err(aor_router::auth_error)?;
    let result = repository::create(&mut tx, &scope, input).await;
    match result {
        Ok(out) => {
            tx.commit().await.map_err(db_error)?;
            Ok(out)
        }
        Err(e) => {
            tx.rollback().await.map_err(db_error)?;
            Err(e)
        }
    }
}
pub async fn update(
    state: &State,
    scope: Authorized<Version, Update>,
    id: Uuid,
    input: Change,
) -> Result<Output, AppError> {
    name(&input.name)?;
    if input.version < 1 {
        return Err(AppError::BadJson);
    }
    let mut lease = state.pool.acquire().await.map_err(db_error)?;
    let mut tx = aor_tx::begin(&mut lease, &state.auth, &scope)
        .await
        .map_err(aor_router::auth_error)?;
    let result = repository::update(&mut tx, &scope, id, input).await;
    match result {
        Ok(out) => {
            tx.commit().await.map_err(db_error)?;
            Ok(out)
        }
        Err(e) => {
            tx.rollback().await.map_err(db_error)?;
            Err(e)
        }
    }
}
pub async fn delete(
    state: &State,
    scope: Authorized<Version, Delete>,
    id: Uuid,
    input: Removal,
) -> Result<(), AppError> {
    if input.version < 1 {
        return Err(AppError::BadJson);
    }
    let mut lease = state.pool.acquire().await.map_err(db_error)?;
    let mut tx = aor_tx::begin(&mut lease, &state.auth, &scope)
        .await
        .map_err(aor_router::auth_error)?;
    let result = repository::delete(&mut tx, &scope, id, input).await;
    match result {
        Ok(out) => {
            tx.commit().await.map_err(db_error)?;
            Ok(out)
        }
        Err(e) => {
            tx.rollback().await.map_err(db_error)?;
            Err(e)
        }
    }
}
pub fn routes(state: std::sync::Arc<State>) -> Vec<aor_router::Route> {
    let mut routes = Vec::new();
    {
        let state = state.clone();
        routes.push(aor_router::Route::protected(
            "GET",
            "/versions",
            "versions::list",
            "owner<Version,Read>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Version, Read>(ctx.principal())
                        .map_err(policy_error)?;
                    let out = list(&state, scope).await?;
                    crate::json(200, aor_policy::View(out))
                }
            },
        ));
    }
    {
        let state = state.clone();
        routes.push(aor_router::Route::protected(
            "POST",
            "/versions",
            "versions::create",
            "owner<Version,Create>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Version, Create>(ctx.principal())
                        .map_err(policy_error)?;
                    let input = ctx.json().await?;
                    let out = create(&state, scope, input).await?;
                    crate::json(201, aor_policy::View(out))
                }
            },
        ));
    }
    {
        let state = state.clone();
        routes.push(aor_router::Route::protected(
            "GET",
            "/versions/{id}",
            "versions::get",
            "owner<Version,Read>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Version, Read>(ctx.principal())
                        .map_err(policy_error)?;
                    let id = ctx.path.get::<Uuid>("id")?;
                    let out = get(&state, scope, id).await?;
                    crate::json(200, aor_policy::View(out))
                }
            },
        ));
    }
    {
        let state = state.clone();
        routes.push(aor_router::Route::protected(
            "PATCH",
            "/versions/{id}",
            "versions::update",
            "owner<Version,Update>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Version, Update>(ctx.principal())
                        .map_err(policy_error)?;
                    let id = ctx.path.get::<Uuid>("id")?;
                    let input = ctx.json().await?;
                    let out = update(&state, scope, id, input).await?;
                    crate::json(200, aor_policy::View(out))
                }
            },
        ));
    }
    {
        let state = state.clone();
        routes.push(aor_router::Route::protected(
            "DELETE",
            "/versions/{id}",
            "versions::delete",
            "owner<Version,Delete>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Version, Delete>(ctx.principal())
                        .map_err(policy_error)?;
                    let id = ctx.path.get::<Uuid>("id")?;
                    let input = ctx.json().await?;
                    delete(&state, scope, id, input).await?;
                    crate::json(200, aor_policy::View(()))
                }
            },
        ));
    }
    routes
}
