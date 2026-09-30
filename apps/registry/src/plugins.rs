use crate::{State, db_error, policy_error};
use aor_db::{Tx, Uuid};
use aor_policy::{Authorized, Create, Delete, Read, Resource, Update};
use aor_router::AppError;
use serde::{Deserialize, Serialize};
pub enum Plugin {}
impl Resource for Plugin {
    const NAME: &'static str = "plugins";
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub name: String,
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
        "SELECT id, owner_id, name, version FROM plugins WHERE owner_id = $1 ORDER BY id"
    );
    aor_db::sql!(
        Find,
        portable,
        "migrations",
        "SELECT id, owner_id, name, version FROM plugins WHERE id = $1 AND owner_id = $2"
    );
    aor_db::sql!(
        Insert,
        portable,
        "migrations",
        "INSERT INTO plugins (id, owner_id, name, version) VALUES ($1, $2, $3, 1) RETURNING id, owner_id, name, version"
    );
    aor_db::sql!(
        UpdateRow,
        portable,
        "migrations",
        "UPDATE plugins SET name = $1, version = version + 1 WHERE id = $2 AND owner_id = $3 AND version = $4 RETURNING id, owner_id, name, version"
    );
    aor_db::sql!(
        DeleteRow,
        portable,
        "migrations",
        "DELETE FROM plugins WHERE id = $1 AND owner_id = $2 AND version = $3"
    );
    impl TryFrom<ListRow> for Output {
        type Error = AppError;
        fn try_from(row: ListRow) -> Result<Self, AppError> {
            let _owner = row.owner_id;
            Ok(Self {
                id: row.id.parse().map_err(|_| AppError::Internal)?,
                name: row.name,
                version: row.version,
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
            })
        }
    }
    pub async fn list(
        tx: &mut Tx<'_>,
        scope: &Authorized<Plugin, Read>,
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
        scope: &Authorized<Plugin, Read>,
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
        scope: &Authorized<Plugin, Create>,
        input: Input,
    ) -> Result<Output, AppError> {
        Insert::query(
            tx,
            Uuid::new_v4().to_string(),
            scope.owner_id().to_string(),
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
        scope: &Authorized<Plugin, Update>,
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
        scope: &Authorized<Plugin, Delete>,
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
pub async fn list(state: &State, scope: Authorized<Plugin, Read>) -> Result<Vec<Output>, AppError> {
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
    scope: Authorized<Plugin, Read>,
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
    scope: Authorized<Plugin, Create>,
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
    scope: Authorized<Plugin, Update>,
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
    scope: Authorized<Plugin, Delete>,
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
            "/plugins",
            "plugins::list",
            "owner<Plugin,Read>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope =
                        aor_policy::owner::<Plugin, Read>(ctx.principal()).map_err(policy_error)?;
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
            "/plugins",
            "plugins::create",
            "owner<Plugin,Create>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Plugin, Create>(ctx.principal())
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
            "/plugins/{id}",
            "plugins::get",
            "owner<Plugin,Read>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope =
                        aor_policy::owner::<Plugin, Read>(ctx.principal()).map_err(policy_error)?;
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
            "/plugins/{id}",
            "plugins::update",
            "owner<Plugin,Update>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Plugin, Update>(ctx.principal())
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
            "/plugins/{id}",
            "plugins::delete",
            "owner<Plugin,Delete>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Plugin, Delete>(ctx.principal())
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
