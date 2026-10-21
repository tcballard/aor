use aor_db::Pool;
use aor_jobs::{Claim, Job};
use aor_mail::{Key, Resend, Worker};
pub enum Task {
    Manifest(Claim),
    Mail(Claim),
}
pub struct Runtime {
    pool: Pool,
    mail: Option<Worker<Resend>>,
}
impl Runtime {
    pub fn new(pool: Pool, mail: Option<(Key, Resend)>) -> Self {
        let mail = mail.map(|(key, transport)| Worker::new(pool.clone(), key, transport));
        Self { pool, mail }
    }
    pub async fn claim(&self, mail_first: bool) -> aor_jobs::Result<Option<Task>> {
        for is_mail in [mail_first, !mail_first] {
            if is_mail && self.mail.is_none() {
                continue;
            }
            let queue = if is_mail { "mail" } else { "registry" };
            if let Some(claim) =
                aor_jobs::claim(&self.pool, queue, aor_db::Utc::now().timestamp(), 60).await?
            {
                return Ok(Some(if is_mail {
                    Task::Mail(claim)
                } else {
                    Task::Manifest(claim)
                }));
            }
        }
        Ok(None)
    }
    pub async fn process(&self, task: Task) -> Result<(), Box<dyn std::error::Error>> {
        match task {
            Task::Mail(claim) => {
                self.mail
                    .as_ref()
                    .ok_or("mail worker not configured")?
                    .run_claim(&claim)
                    .await?
            }
            Task::Manifest(claim) => {
                let at = aor_db::Utc::now().timestamp();
                if claim.job_type() != crate::manifests::ValidateManifestV1::NAME
                    || claim.payload_version() != crate::manifests::ValidateManifestV1::VERSION
                {
                    aor_jobs::incompatible(&self.pool, &claim, at).await?;
                } else if let Err(error) = crate::manifests::process(&self.pool, &claim, at).await {
                    if !matches!(error, aor_jobs::Error::Stale) {
                        aor_jobs::fail(
                            &self.pool,
                            &claim,
                            aor_db::Utc::now().timestamp(),
                            aor_jobs::Failure::Handler,
                        )
                        .await?;
                    }
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }
}
