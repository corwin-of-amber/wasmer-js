#![macro_use]

mod fs;
mod copipe;
mod wasi;

pub use crate::fs::{JSVirtualFile, MemFS};
pub use crate::wasi::WASI;

extern crate web_sys;

// A macro to provide `println!(..)`-style syntax for `console.log` logging.
#[macro_export]
macro_rules! log {
    ( $( $t:tt )* ) => {
        web_sys::console::log_1(&format!( $( $t )* ).into());
    }
}
