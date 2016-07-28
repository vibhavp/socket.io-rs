extern crate engine_io;
extern crate iron;
extern crate serde;
extern crate serde_json;

pub mod server;
pub mod socket;
pub mod packet;
pub mod data;

pub const PROTOCOL_VERSION: usize = 4;
