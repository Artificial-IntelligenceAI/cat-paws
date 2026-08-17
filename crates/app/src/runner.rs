//! Running a compiled WebAssembly module.
//!
//! The bytes are the same on both platforms — what differs is who hosts them. In the
//! browser the host is the browser's own engine, reached through JavaScript. Natively it
//! is wasmtime. Either way the program that runs is the module the compiler emitted, not
//! an interpretation of anything.
//!
//! The host has to supply `print`, because WebAssembly has no I/O of its own: a module
//! can only call functions it was given. `print` receives a pointer and reads the string
//! out of the module's exported memory.

/// What running a program produced.
pub struct Outcome {
    pub output: Vec<String>,
    pub error: Option<String>,
}

/// Read a `[len: u32][utf8 bytes]` record out of a module's memory.
fn read_string(memory: &[u8], ptr: usize) -> String {
    if ptr + 4 > memory.len() {
        return String::new();
    }
    let len = u32::from_le_bytes(memory[ptr..ptr + 4].try_into().unwrap()) as usize;
    let start = ptr + 4;
    let end = (start + len).min(memory.len());
    String::from_utf8_lossy(&memory[start..end]).into_owned()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(bytes: &[u8]) -> Outcome {
    use std::sync::{Arc, Mutex};

    let printed: Arc<Mutex<Vec<String>>> = Arc::default();
    let result = (|| -> Result<(), String> {
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, bytes).map_err(|e| e.to_string())?;
        let mut store = wasmtime::Store::new(&engine, Arc::clone(&printed));
        let mut linker = wasmtime::Linker::new(&engine);
        linker
            .func_wrap(
                "env",
                "print",
                |mut caller: wasmtime::Caller<'_, Arc<Mutex<Vec<String>>>>, ptr: i32| {
                    let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory())
                    else {
                        return;
                    };
                    let text = super::runner::read_string(memory.data(&caller), ptr as usize);
                    caller.data().lock().unwrap().push(text);
                },
            )
            .map_err(|e| e.to_string())?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| e.to_string())?;
        instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .map_err(|e| e.to_string())?
            .call(&mut store, ())
            .map_err(|e| e.to_string())
    })();

    let output = printed.lock().unwrap().clone();
    Outcome {
        output,
        error: result.err(),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn run(bytes: &[u8]) -> Outcome {
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    thread_local! {
        /// Lines the running program printed. A thread local rather than a captured
        /// variable because the closure handed to WebAssembly outlives this call.
        static PRINTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        /// The running module's memory, so `print` can read the string it was handed.
        static MEMORY: RefCell<Option<js_sys::WebAssembly::Memory>> = const { RefCell::new(None) };
    }

    PRINTED.with(|p| p.borrow_mut().clear());
    MEMORY.with(|m| *m.borrow_mut() = None);

    let result = (|| -> Result<(), String> {
        let buffer = js_sys::Uint8Array::from(bytes);
        // Synchronous instantiation. Browsers refuse this on the main thread for
        // modules over 4KB, which the programs written here are nowhere near; growing
        // past it means moving to the async form and holding the result across frames.
        let module = js_sys::WebAssembly::Module::new(&buffer.into())
            .map_err(|e| format!("the engine rejected the module: {e:?}"))?;

        let print = Closure::<dyn Fn(i32)>::new(|ptr: i32| {
            let text = MEMORY.with(|m| {
                let borrowed = m.borrow();
                let Some(memory) = borrowed.as_ref() else {
                    return String::new();
                };
                let bytes = js_sys::Uint8Array::new(&memory.buffer()).to_vec();
                read_string(&bytes, ptr as usize)
            });
            PRINTED.with(|p| p.borrow_mut().push(text));
        });

        let env = js_sys::Object::new();
        js_sys::Reflect::set(&env, &"print".into(), print.as_ref()).ok();
        let imports = js_sys::Object::new();
        js_sys::Reflect::set(&imports, &"env".into(), &env).ok();

        let instance = js_sys::WebAssembly::Instance::new(&module, &imports)
            .map_err(|e| format!("the module would not start: {e:?}"))?;
        let exports = instance.exports();

        let memory = js_sys::Reflect::get(&exports, &"memory".into())
            .ok()
            .and_then(|m| m.dyn_into::<js_sys::WebAssembly::Memory>().ok())
            .ok_or("the module did not export its memory")?;
        MEMORY.with(|m| *m.borrow_mut() = Some(memory));

        let main = js_sys::Reflect::get(&exports, &"main".into())
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            .ok_or("the module did not export main")?;
        main.call0(&JsValue::undefined())
            .map_err(|e| format!("stopped while running: {e:?}"))?;

        // Kept alive until the program has finished calling it.
        drop(print);
        Ok(())
    })();

    Outcome {
        output: PRINTED.with(|p| p.borrow().clone()),
        error: result.err(),
    }
}
