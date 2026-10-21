use aor_db::Pool;
use aor_jobs::{Context, Handler, Worker};
use aor_mail::{DeliverMailV1, Key, Resend};

struct Manifest;
impl Handler<crate::manifests::ValidateManifestV1> for Manifest {
    async fn handle(
        &self,
        context: Context<'_>,
        _job: crate::manifests::ValidateManifestV1,
    ) -> aor_jobs::Result<()> {
        crate::manifests::process(context.pool, context.claim, context.now()).await
    }
}
struct Mail(aor_mail::Worker<Resend>);
impl Handler<DeliverMailV1> for Mail {
    async fn handle(&self, context: Context<'_>, _job: DeliverMailV1) -> aor_jobs::Result<()> {
        self.0.run_claim(context.claim).await.map_err(|e| match e {
            aor_mail::Error::Queue(e) => e,
            aor_mail::Error::Database(e) => aor_jobs::Error::Database(e),
            _ => aor_jobs::Error::Invalid,
        })
    }
}
pub fn runtime(pool: Pool, mail: Option<(Key, Resend)>) -> aor_jobs::Result<Worker> {
    let mut worker = Worker::new(pool.clone());
    worker.register::<crate::manifests::ValidateManifestV1>("registry", Manifest)?;
    if let Some((key, transport)) = mail {
        worker
            .register::<DeliverMailV1>("mail", Mail(aor_mail::Worker::new(pool, key, transport)))?;
    }
    Ok(worker)
}
