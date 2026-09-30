#![no_main]
use aor_http::{Chunk,ChunkDecoder,Limits};
fn decode(data:&[u8],fragment:usize)->Result<(Vec<u8>,usize),()> {
    let mut decoder=ChunkDecoder::default();let mut offset=0;let mut end=0;let mut bytes=Vec::new();
    loop {
        match decoder.step(&data[offset..end],&Limits::default()).map_err(|_|())? {
            Chunk::NeedMore=>{if end==data.len(){return Err(());}end=(end+fragment).min(data.len());},
            Chunk::Data{bytes:part,consumed}=>{bytes.extend_from_slice(part);offset+=consumed;},
            Chunk::Progress(n)=>offset+=n,
            Chunk::End(n)=>return Ok((bytes,offset+n)),
        }
        assert!(offset<=end);
    }
}
libfuzzer_sys::fuzz_target!(|data:&[u8]| { assert_eq!(decode(data,data.len().max(1)),decode(data,7)); });
