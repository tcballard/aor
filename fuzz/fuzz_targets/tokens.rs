#![no_main]
libfuzzer_sys::fuzz_target!(|data:&[u8]| {
    if let Ok(tokens)=aor_http::tokens(data) { for t in tokens { assert!(!t.is_empty());assert!(t.iter().copied().all(aor_http::is_token)); } }
});
