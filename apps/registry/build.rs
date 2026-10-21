fn main() {
    aor_migrate::build("migrations", aor_migrate::Dialect::Postgres)
        .expect("checked registry schema");
}
