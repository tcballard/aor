//! A service transaction revalidates its capability before any repository effects.
use aor_db::{Lease, Tx};
use aor_policy::{Action, Authorized, Resource};
pub async fn begin<'a, R: Resource, A: Action>(
    lease: &'a mut Lease,
    auth: &aor_session::Auth,
    scope: &Authorized<R, A>,
) -> aor_session::Result<Tx<'a>> {
    let mut tx = lease.begin().await?;
    auth.validate_in(&mut tx, scope.principal()).await?;
    Ok(tx)
}
