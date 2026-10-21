use crate::{Claim, Error, Failure, Idempotent, Job, Result};
use aor_db::Pool;
use std::{future::Future, marker::PhantomData, pin::Pin, sync::Arc, time::Duration};

/// Handlers own their fenced domain transaction and acknowledgement. Returning
/// success never causes the runtime to acknowledge on a separate connection.
pub trait Handler<J: Job>: Send + Sync {
    fn handle(&self, context: Context<'_>, job: J) -> impl Future<Output = Result<()>> + Send;
}

#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub pool: &'a Pool,
    pub claim: &'a Claim,
    clock: &'a (dyn Fn() -> i64 + Send + Sync),
}
impl Context<'_> {
    /// Read at each transaction boundary, rather than retaining claim time.
    pub fn now(&self) -> i64 {
        (self.clock)()
    }
}

#[derive(Clone, Copy)]
pub struct WorkerOptions {
    pub lease_seconds: i64,
    pub heartbeat: Duration,
    pub poll: Duration,
    pub drain: Duration,
}
impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            lease_seconds: 60,
            heartbeat: Duration::from_secs(10),
            poll: Duration::from_millis(100),
            drain: Duration::from_secs(30),
        }
    }
}
impl WorkerOptions {
    fn validate(self) -> Result<()> {
        if !(3..=3600).contains(&self.lease_seconds)
            || self.heartbeat.is_zero()
            || self.heartbeat > Duration::from_secs(self.lease_seconds as u64) / 3
            || self.poll.is_zero()
            || self.poll > Duration::from_secs(60)
            || self.drain > Duration::from_secs(3600)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

type Work<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
trait Dispatch: Send + Sync {
    fn run<'a>(&'a self, context: Context<'a>) -> Work<'a>;
}
struct Typed<J, H>(H, PhantomData<fn() -> J>);
impl<J: Job + Send + 'static, H: Handler<J>> Dispatch for Typed<J, H> {
    fn run<'a>(&'a self, context: Context<'a>) -> Work<'a> {
        Box::pin(async move {
            let job = context.claim.decode::<J>()?;
            self.0.handle(context, job).await
        })
    }
}
struct Registration {
    queue: String,
    name: &'static str,
    version: i64,
    handler: Box<dyn Dispatch>,
}

/// Sequential worker with fair queue polling and explicit accepted versions.
/// External effects still need provider idempotency: cancellation cannot undo
/// a request that a remote service has already accepted.
pub struct Worker {
    pool: Pool,
    options: WorkerOptions,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    registrations: Vec<Registration>,
    queues: Vec<String>,
    next_queue: usize,
}
impl Worker {
    pub fn new(pool: Pool) -> Self {
        Self {
            pool,
            options: WorkerOptions::default(),
            clock: Arc::new(|| aor_db::Utc::now().timestamp()),
            registrations: Vec::new(),
            queues: Vec::new(),
            next_queue: 0,
        }
    }
    pub fn with_options(mut self, options: WorkerOptions) -> Result<Self> {
        options.validate()?;
        self.options = options;
        Ok(self)
    }
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }
    pub fn register<J: Job + Send + 'static>(
        &mut self,
        queue: &str,
        handler: impl Handler<J> + 'static,
    ) -> Result<()> {
        if !crate::label(queue)
            || !crate::label(J::NAME)
            || J::VERSION < 1
            || matches!(J::IDEMPOTENCY, Idempotent::AtLeastOnceTolerated(reason) if reason.trim().is_empty())
            || self
                .registrations
                .iter()
                .any(|r| r.queue == queue && r.name == J::NAME && r.version == J::VERSION)
        {
            return Err(Error::Invalid);
        }
        if !self.queues.iter().any(|q| q == queue) {
            self.queues.push(queue.into());
        }
        self.registrations.push(Registration {
            queue: queue.into(),
            name: J::NAME,
            version: J::VERSION,
            handler: Box::new(Typed::<J, _>(handler, PhantomData)),
        });
        Ok(())
    }
    pub async fn claim(&mut self) -> Result<Option<Claim>> {
        if self.queues.is_empty() {
            return Err(Error::Invalid);
        }
        for _ in 0..self.queues.len() {
            let queue = &self.queues[self.next_queue];
            self.next_queue = (self.next_queue + 1) % self.queues.len();
            if let Some(claim) = crate::claim(
                &self.pool,
                queue,
                (self.clock)(),
                self.options.lease_seconds,
            )
            .await?
            {
                return Ok(Some(claim));
            }
        }
        Ok(None)
    }
    async fn supervised(&self, claim: &Claim, handler: &dyn Dispatch) -> Result<()> {
        let work = handler.run(Context {
            pool: &self.pool,
            claim,
            clock: self.clock.as_ref(),
        });
        tokio::pin!(work);
        loop {
            tokio::select! {
                biased;
                result = &mut work => return result,
                _ = tokio::time::sleep(self.options.heartbeat) => {}
            }
            // A fenced transaction may itself hold the job lock. Continue
            // polling it while renewal waits, or the worker would deadlock on
            // its own transaction. A hung renewal must not stall cancellation.
            let renewal = tokio::time::timeout(
                Duration::from_secs(self.options.lease_seconds as u64) - self.options.heartbeat,
                crate::renew(
                    &self.pool,
                    claim,
                    (self.clock)(),
                    self.options.lease_seconds,
                ),
            );
            tokio::select! {
                biased;
                result = &mut work => return result,
                renewed = renewal => renewed.map_err(|_| Error::Stale)??,
            }
        }
    }
    pub async fn process(&self, claim: &Claim) -> Result<()> {
        let Some(registration) = self.registrations.iter().find(|r| {
            r.queue == claim.queue()
                && r.name == claim.job_type()
                && r.version == claim.payload_version()
        }) else {
            return crate::incompatible(&self.pool, claim, (self.clock)()).await;
        };
        match self.supervised(claim, registration.handler.as_ref()).await {
            Ok(()) => Ok(()),
            Err(Error::Stale) => Err(Error::Stale),
            Err(error) => {
                let failure = if matches!(error, Error::Invalid) {
                    Failure::Payload
                } else {
                    Failure::Handler
                };
                match crate::fail(&self.pool, claim, (self.clock)(), failure).await {
                    Ok(()) | Err(Error::Stale) => Err(error),
                    Err(failure) => Err(failure),
                }
            }
        }
    }
    pub async fn work_once(&mut self) -> Result<bool> {
        let Some(claim) = self.claim().await? else {
            return Ok(false);
        };
        self.process(&claim).await?;
        Ok(true)
    }
    pub async fn run_until(&mut self, stop: impl Future<Output = ()>) -> Result<()> {
        if self.queues.is_empty() {
            return Err(Error::Invalid);
        }
        tokio::pin!(stop);
        loop {
            let claimed = tokio::select! {
                biased;
                _ = &mut stop => return Ok(()),
                result = self.claim() => result,
            };
            match claimed {
                Ok(Some(claim)) => {
                    let work = self.process(&claim);
                    tokio::pin!(work);
                    tokio::select! {
                        biased;
                        _ = &mut stop => {
                            if let Ok(Err(error)) = tokio::time::timeout(self.options.drain, &mut work).await {
                                eprintln!("{error}");
                            }
                            return Ok(());
                        }
                        result = &mut work => if let Err(error) = result { eprintln!("{error}"); },
                    }
                }
                Err(error) => eprintln!("{error}"),
                Ok(None) => {}
            }
            tokio::select! {
                biased;
                _ = &mut stop => return Ok(()),
                _ = tokio::time::sleep(self.options.poll) => {}
            }
        }
    }
}
