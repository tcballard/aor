#![no_main]
use aor_http::{parse_head,Limits};
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let limits=Limits::default();
    if let Ok(Some(head))=parse_head(data,&limits) {
        assert!(head.consumed <= data.len());
        let canonical=head.canonical();
        let other=parse_head(&canonical,&limits).unwrap().unwrap();
        assert_eq!(other.framing,head.framing);
        assert_eq!(other.method,head.method);
        assert_eq!(other.target,head.target);
        assert_eq!(other.close,head.close);
        assert_eq!(other.canonical(),canonical);
    }
});
