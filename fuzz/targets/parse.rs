#![no_main]
use libfuzzer_sys::fuzz_target;
use soppo::syntax::{FileId, Parser};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let mut parser = Parser::new(s, FileId(0));
        let _ = parser.parse_file();
    }
});
