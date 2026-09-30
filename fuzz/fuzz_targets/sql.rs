#![no_main]
use libfuzzer_sys::fuzz_target;
use aor_sql::{Dialect,Schema,check};
fuzz_target!(|data: &[u8]| {
    if let Ok(sql)=std::str::from_utf8(data) {
        for dialect in [Dialect::Postgres,Dialect::Sqlite] {
            let mut schema=Schema::default();
            schema.apply("CREATE TABLE editions (id TEXT NOT NULL PRIMARY KEY, title TEXT NOT NULL, version BIGINT NOT NULL);",dialect).unwrap();
            let _=check(&schema,sql,dialect);
            let _=schema.apply(sql,dialect);
        }
    }
});
