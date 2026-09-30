//! Strict, bounded HTTP/1.1 for a trusted reverse-proxy hop.
//! Experimental: public-use fuzzing and independent review gates are not yet met.
mod parser;
mod server;
pub use parser::*;
pub use server::*;
