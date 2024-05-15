#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub mod client;
pub use workbook_icd as icd;
