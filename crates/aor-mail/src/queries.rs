use aor_db::sql;
sql!(pub Insert,portable,"migrations","INSERT INTO aor_mail (id, sender, recipient, purpose, token_digest, expires_at, created_at, first_attempt_at, state) VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'pending')");
sql!(pub Load,portable,"migrations","SELECT id, sender, recipient, purpose, token_digest, expires_at, created_at, first_attempt_at, state FROM aor_mail WHERE id = $1");
sql!(pub Start,portable,"migrations","UPDATE aor_mail SET first_attempt_at = $1 WHERE id = $2 AND first_attempt_at = 0 AND state = 'pending'");
sql!(pub State,portable,"migrations","UPDATE aor_mail SET state = $1 WHERE id = $2 AND state = 'pending'");
