//! Owner-scoped capabilities. Only this module's policy can construct a scope.
use aor_session::Principal;
use std::marker::PhantomData;
pub trait Resource {
    const NAME: &'static str;
    const PUBLIC_EXISTENCE: bool = false;
}
mod sealed {
    pub trait Action {}
}
pub trait Action: sealed::Action {
    const NAME: &'static str;
    const MUTATES: bool;
}
pub enum Read {}
pub enum Create {}
pub enum Update {}
pub enum Delete {}
macro_rules! action {
    ($t:ty,$n:literal,$m:literal) => {
        impl sealed::Action for $t {}
        impl Action for $t {
            const NAME: &'static str = $n;
            const MUTATES: bool = $m;
        }
    };
}
action!(Read, "read", false);
action!(Create, "create", true);
action!(Update, "update", true);
action!(Delete, "delete", true);
pub struct Authorized<R: Resource, A: Action> {
    principal: Principal,
    marker: PhantomData<fn() -> (R, A)>,
}
impl<R: Resource, A: Action> Authorized<R, A> {
    pub fn owner_id(&self) -> aor_db::Uuid {
        self.principal.user_id()
    }
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    Unauthenticated,
    NotFound,
    Forbidden,
}
pub fn owner<R: Resource, A: Action>(
    principal: Option<&Principal>,
) -> Result<Authorized<R, A>, Denied> {
    let principal = principal.ok_or(Denied::Unauthenticated)?;
    if !principal.permits(R::NAME, A::NAME) {
        return Err(if R::PUBLIC_EXISTENCE {
            Denied::Forbidden
        } else {
            Denied::NotFound
        });
    }
    Ok(Authorized {
        principal: principal.clone(),
        marker: PhantomData,
    })
}
/// Only explicit projections belong on the wire. Database entities stay unserializable.
#[derive(serde::Serialize)]
#[serde(transparent)]
pub struct View<T: serde::Serialize>(pub T);
