fn main() {
    use sha2::{Digest, Sha256};
    println!("cargo:rerun-if-changed=public/archive.css");
    let bytes = std::fs::read("public/archive.css").expect("archive stylesheet");
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let path = format!("/assets/archive.{}.css", &hash[..16]);
    std::fs::write(
        std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("archive_css_path.txt"),
        path,
    )
    .unwrap();
    aor_migrate::build("migrations/postgres", aor_migrate::Dialect::Postgres)
        .expect("committed migrations must derive a schema");
    println!("cargo:rerun-if-changed=migrations/sqlite");
}
