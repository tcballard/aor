use aor_db::sql;
sql!(pub Insert,portable,"migrations","INSERT INTO aor_jobs (id, job_type, payload_version, payload, dedup_key, queue, state, attempt, max_attempts, run_at, lease_expires_at, lease_generation, created_at, updated_at, last_error) VALUES ($1, $2, $3, $4, $5, $6, 'queued', 0, $7, $8, 0, 0, $9, $9, '') ON CONFLICT (job_type, dedup_key) DO NOTHING");
sql!(pub ByKey,portable,"migrations","SELECT id FROM aor_jobs WHERE job_type = $1 AND dedup_key = $2");
sql!(pub DueLocal,sqlite,"migrations","SELECT id, job_type, payload_version, payload, attempt, max_attempts, lease_generation FROM aor_jobs WHERE queue = $1 AND ((state = 'queued' AND run_at <= $2) OR (state = 'running' AND lease_expires_at <= $2)) ORDER BY run_at, id LIMIT 1");

sql!(pub ClaimRow,portable,"migrations","UPDATE aor_jobs SET state = 'running', attempt = attempt + 1, lease_generation = lease_generation + 1, lease_expires_at = $1, updated_at = $2 WHERE id = $3 AND lease_generation = $4 AND ((state = 'queued' AND run_at <= $2) OR (state = 'running' AND lease_expires_at <= $2))");

sql!(pub Fence,portable,"migrations","UPDATE aor_jobs SET updated_at = $1 WHERE id = $2 AND state = 'running' AND lease_generation = $3 AND lease_expires_at > $1");
sql!(pub Complete,portable,"migrations","UPDATE aor_jobs SET state = 'succeeded', updated_at = $1 WHERE id = $2 AND state = 'running' AND lease_generation = $3 AND lease_expires_at > $1");
sql!(pub Renew,portable,"migrations","UPDATE aor_jobs SET lease_expires_at = $1, updated_at = $2 WHERE id = $3 AND state = 'running' AND lease_generation = $4 AND lease_expires_at > $2");
sql!(pub Retry,portable,"migrations","UPDATE aor_jobs SET state = $1, run_at = $2, last_error = $3, updated_at = $4 WHERE id = $5 AND state = 'running' AND lease_generation = $6 AND lease_expires_at > $4");
sql!(pub Incompatible,portable,"migrations","UPDATE aor_jobs SET state = 'incompatible', attempt = attempt - 1, last_error = 'AOR-JOB-INCOMPATIBLE', updated_at = $1 WHERE id = $2 AND state = 'running' AND lease_generation = $3 AND lease_expires_at > $1");
sql!(pub Inspect,portable,"migrations","SELECT id, job_type, payload_version, queue, state, attempt, max_attempts, run_at, lease_expires_at, lease_generation, created_at, updated_at, last_error FROM aor_jobs WHERE id = $1");
sql!(pub List,portable,"migrations","SELECT id, job_type, state, attempt, max_attempts, run_at, last_error FROM aor_jobs WHERE queue = $1 ORDER BY created_at, id LIMIT $2");
sql!(pub Requeue,portable,"migrations","UPDATE aor_jobs SET state = 'queued', attempt = 0, run_at = $1, lease_generation = lease_generation + 1, last_error = '', updated_at = $1 WHERE id = $2 AND (state = 'failed' OR state = 'incompatible' OR state = 'parked')");
sql!(pub Park,portable,"migrations","UPDATE aor_jobs SET state = 'parked', lease_generation = lease_generation + 1, updated_at = $1 WHERE id = $2 AND (state = 'queued' OR state = 'running')");

sql!(pub ClaimPostgres,postgres,"migrations","UPDATE aor_jobs SET state = 'running', attempt = attempt + 1, lease_generation = lease_generation + 1, lease_expires_at = $1, updated_at = $2 WHERE id = (SELECT id FROM aor_jobs WHERE queue = $3 AND attempt < max_attempts AND ((state = 'queued' AND run_at <= $2) OR (state = 'running' AND lease_expires_at <= $2)) ORDER BY run_at, id LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING id, job_type, payload_version, payload, attempt, max_attempts, lease_generation");
sql!(pub ExhaustedDue,portable,"migrations","UPDATE aor_jobs SET state = 'failed', lease_generation = lease_generation + 1, last_error = 'AOR-JOB-EXHAUSTED', updated_at = $1 WHERE queue = $2 AND attempt >= max_attempts AND ((state = 'queued' AND run_at <= $1) OR (state = 'running' AND lease_expires_at <= $1))");
