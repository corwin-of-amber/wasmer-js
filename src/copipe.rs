use std::io;
use std::io::{Read, Seek, Write};
use js_sys::{Function, Uint8Array};
use wasm_bindgen::JsValue;
use wasmer_vfs::{FsError, VirtualFile};

#[derive(Debug, Clone, Default)]
pub struct Copipe {
    pub(crate) hook: JsValue
}

unsafe impl Send for Copipe { }
unsafe impl Sync for Copipe { }

impl Read for Copipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let emit: Function =
            js_sys::Reflect::get(&self.hook, &"read".into()).unwrap().into();
        let rbuf: Uint8Array = emit.call1(&self.hook, &buf.len().into()).unwrap().into();
        let sz: usize = rbuf.length().try_into().unwrap();
        rbuf.copy_to(&mut buf[..sz]);
        Ok(sz)
    }
}
impl Write for Copipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let emit: Function =
            js_sys::Reflect::get(&self.hook, &"write".into()).unwrap().into();
        let ui8a: Uint8Array = Uint8Array::from(buf);
        emit.call1(&self.hook, &ui8a).unwrap();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Seek for Copipe {
    fn seek(&mut self, _pos: io::SeekFrom) -> io::Result<u64> { Ok(0) }
}

impl VirtualFile for Copipe {
    fn last_accessed(&self) -> u64 { 0 }
    fn last_modified(&self) -> u64 { 0 }
    fn created_time(&self) -> u64 { 0 }
    fn size(&self) -> u64 { 0 }
    fn set_len(&mut self, len: u64) -> Result<(), FsError> { Ok(()) }
    fn unlink(&mut self) -> Result<(), FsError> { Ok(()) }
    fn bytes_available_read(&self) -> Result<Option<usize>, FsError> {
        Ok(Some(0))
    }
}
