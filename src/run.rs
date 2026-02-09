use futures::channel::oneshot;
use std::sync::Arc;
use wasm_bindgen::{prelude::wasm_bindgen, JsCast};
use wasmer::RuntimeError;
use wasmer_wasix::{WasiEnv, WasiError, WasiRuntimeError};
use wasmer_wasix::runtime::module_cache::HashedModuleData;
use crate::{instance::ExitCondition, utils::Error, Instance, RunOptions};

const DEFAULT_PROGRAM_NAME: &str = "wasm";

/// Run a WASIX program.
///
/// # WASI Compatibility
///
/// The [WASIX standard][wasix] is a superset of [WASI preview 1][preview-1], so programs
/// compiled to WASI will run without any problems.
///
/// WASI Preview 2 is a backwards incompatible rewrite of WASI.
/// This means programs compiled for WASI Preview 2 will fail to load.
///
/// [wasix]: https://wasix.dev/
/// [preview-1]: https://github.com/WebAssembly/WASI/blob/main/legacy/README.md
#[wasm_bindgen(js_name = "runWasix")]
pub async fn run_wasix(wasm_module: WasmModule, config: RunOptions) -> Result<Instance, Error> {
    run_wasix_inner(wasm_module, config).await
}

#[tracing::instrument(level = "debug", skip_all)]
async fn run_wasix_inner(wasm_module: WasmModule, config: RunOptions) -> Result<Instance, Error> {
    let mut runtime = config.runtime().resolve()?.into_inner();
    // We set it up with the default pool
    runtime = Arc::new(runtime.with_default_pool());

    let program_name = config
        .program()
        .as_string()
        .unwrap_or_else(|| DEFAULT_PROGRAM_NAME.to_string());

    let mut builder = WasiEnv::builder(program_name).runtime(runtime.clone());
    let (stdin, stdout, stderr) = config.configure_builder(&mut builder, runtime.clone())?;

    let (exit_code_tx, exit_code_rx) = oneshot::channel();

    let module: wasmer::Module = wasm_module.to_module(&*runtime).await?;

    let env = builder.build()?;
    let runtime = env.runtime.clone();

    let mut join_handle = wasmer_wasix::bin_factory::spawn_exec_module(module, env, &runtime)?;

    wasm_bindgen_futures::spawn_local(async move {
        let result = join_handle.wait_finished().await
            .and_then(|ec| if ec.is_success() { Ok(()) }
                           else { Err(Arc::new(WasiRuntimeError::Wasi(WasiError::Exit(ec)))) })
            .map_err(anyhow::Error::new);
        _ = exit_code_tx.send(ExitCondition::from_result(result));
    });

    Ok(Instance {
        stdin,
        stdout,
        stderr,
        exit: exit_code_rx,
    })
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "WebAssembly.Module | Uint8Array")]
    pub type WasmModule;
}

impl WasmModule {
    pub(crate) async fn to_module(
        &self,
        runtime: &dyn wasmer_wasix::Runtime,
    ) -> Result<wasmer::Module, Error> {
        if let Some(module) = self.dyn_ref::<js_sys::WebAssembly::Module>() {
            Ok(module.clone().into())
        } else {
            let buffer = self.dyn_ref::<js_sys::Uint8Array>().map(|o| o.clone())
                /* coerce to Uint8Array (this is sometimes needed, esp. when multiple JS contexts exist) */
                .unwrap_or_else(|| js_sys::Uint8Array::new(self))
                .to_vec();
            if buffer.len() == 0 {
                Err(RuntimeError::new("invalid WASM binary data").into())
            } else {
                let module = runtime.load_hashed_module(HashedModuleData::new(buffer), None).await?;
                Ok(module)
            }
        }
    }
}
