# Transactional mail

Replaces detached mail callbacks and queue integrations such as Underway's mail
jobs. AoR owns the outbox, transactional enqueue, replay policy and fenced worker.
RustCrypto HMAC-SHA256 is bought cryptography; the HTTPS/TLS transport runs in the
system `/usr/bin/curl` process. No HTTP framework, HTTP client crate or queue crate
is introduced into the dependency tree.

`Outbox::enqueue_in` requires the caller's `&mut Tx`. The account/token row, outbox
row and delivery job commit or roll back together. `Auth::with_outbox` enables it
for registration and password reset. The legacy development spool remains separate
and is forbidden in production.

## Secrets and key management

Job payloads contain only a random delivery UUID. They contain neither recipient
addresses nor delivery tokens. Outbox rows contain recipient/sender, purpose,
expiry, state and a token digest. Tokens are reconstructed using domain-separated
HMAC-SHA256 over the random UUID, sender, recipient, purpose and expiry. The account
store still holds only a SHA-256 digest. No custom cryptographic primitive is used.

The signing key is 32 random bytes in a private regular file. It is not the Resend
API key. Keep it outside the database and retain it across serve/worker restarts.
The loader rejects public/group permissions, final-component symlinks, special
files and incorrect length. Missing/wrong keys fail closed before network activity.
Rotating this initial single key requires draining pending messages or deliberately
abandoning them; a multi-key rotation mechanism is not implemented.

`IssuedToken`, `Key` and `Message` have no wire serialization or Debug derives.
Only trusted auth/transport code can explicitly expose the token or message text.
Mail errors use fixed codes. Queue inspection omits payloads and outbox contents.

## Delivery and replay

The worker records its first attempt before calling the provider, renews the lease
while waiting, and commits delivery state plus acknowledgement in a fenced
transaction. If it loses ownership, it cancels the in-flight future and cannot
acknowledge. The curl child is killed when its future is dropped. An already
accepted external request cannot be undone; retries use the same provider key and
exact same body.

Resend documents a 24-hour deduplication window. AoR retries only within 23 hours
of the first recorded attempt and before token expiry. Beyond that window an
unconfirmed delivery becomes `ambiguous` and the job fails permanently without a
new request. Explicit job requeue cannot bypass that guard. Expired mail is
marked `expired` and is not sent. A new verification/reset request creates a new
token/job, rather than reusing an old provider key.

The adapter sends only to `https://api.resend.com/emails`, with no redirects or
curl configuration inheritance. API credentials and the message are supplied on
stdin rather than process arguments or environment. Connection/total/process
bounds are 5/20/25 seconds; the provider body and stderr are discarded. Provider
acceptance is recorded as `delivered`; it is not evidence of inbox delivery.

Sources inspected 21 October 2026:
- [Resend send API](https://resend.com/docs/api-reference/emails/send-email)
- [Resend idempotency](https://resend.com/docs/dashboard/emails/idempotency-keys)
- [RustCrypto HMAC](https://docs.rs/hmac/0.12.1/hmac/)
- [curl configuration](https://curl.se/docs/manpage.html)

Tests use fictional addresses, a deterministic provider double and real curl over
a loopback HTTP fixture. They do not send through a real Resend account. Live
credentials, a verified sender and delivery acceptance are deployment work.
