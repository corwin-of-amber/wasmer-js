use std::sync::Arc;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use js_sys::{Reflect, Error};
use wasmer_wasix::WasiEnv;

#[derive(Debug, Default, Clone)]
#[wasm_bindgen]
pub struct Hooks {
    pub populate: Option<u32>
}


#[wasm_bindgen]
impl Hooks {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self { Default::default() }

    pub(crate) fn trigger_initiated(env: &WasiEnv) {
        let d = crate::fs::Directory::wrap(Arc::new(env.fs_root().clone()));
        Self::or_console_error(Self::call_method("initiated", d.into()));
    }

    pub(crate) fn trigger_populate(&mut self) -> Option<bool> {
        Self::or_console_error(Self::dispatch_take(&mut self.populate))
    }

    pub fn dispatch(handler: u32) -> Result<bool, Error> {
        Self::call_method("dispatch", handler.into())
    }

    pub(crate) fn dispatch_take(handler: &mut Option<u32>) -> Result<bool, Error> {
        if let Some(handler) = handler.take() { Self::dispatch(handler) }
        else { Ok(false) }
    }

    pub fn intercept(message: JsValue) -> Result<bool, Error> {
        Self::call_method("intercept", message)
    }

    fn call_method(name: &str, arg: JsValue) -> Result<bool, Error> {
        let js_fs_hook = Reflect::get(&js_sys::global(), &"fs_hook".into())?;
        if js_fs_hook.is_undefined() { return Ok(false); }
        let method = Reflect::get(&js_fs_hook, &name.into())?;
        if method.is_undefined() { return Ok(false); }
        let ret = js_sys::Function::call1(&method.into(), &js_fs_hook, &arg)?;
        Ok(ret.is_truthy())
    }

    pub(crate) fn or_console_error<T>(r: Result<T, Error>) -> Option<T> {
        r.map(Some).unwrap_or_else(|e: Error| {
            web_sys::console::error_2(&"in fs_hook:".into(), &e.into());
            None
        })
    }
}
