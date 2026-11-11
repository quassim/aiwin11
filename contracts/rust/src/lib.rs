pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/aiwin11.v1.rs"));
}

pub const SCHEMA_VERSION: u32 = 1;
pub use prost::Message;
