fn main() {
    aor_migrate::build("migrations/postgres", aor_migrate::Dialect::Postgres)
        .expect("committed migrations must derive a schema");
    println!("cargo:rerun-if-changed=migrations/sqlite");
}
