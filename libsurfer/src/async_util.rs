/// Code related to asynchronous features.
///
/// As wasm32 and most other platforms behave differently, there are these wrappers.
use futures_core::Future;
use serde::Deserialize;
use tracing::info;

use crate::spawn;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub enum AsyncJob {
    SaveState,
}

// Platform-dependent trait alias for futures that can be spawned
#[cfg(target_arch = "wasm32")]
pub trait SpawnableFuture: Future<Output = ()> + 'static {}
#[cfg(target_arch = "wasm32")]
impl<F> SpawnableFuture for F where F: Future<Output = ()> + 'static {}

#[cfg(not(target_arch = "wasm32"))]
pub trait SpawnableFuture: Future<Output = ()> + Send + 'static {}
#[cfg(not(target_arch = "wasm32"))]
impl<F> SpawnableFuture for F where F: Future<Output = ()> + Send + 'static {}

// Wasm doesn't seem to support std::thread, so this spawns a thread where we can
// but runs the work sequentially where we can not.
pub fn perform_work<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    spawn! {async {
        info!("Starting async task");
        f();
    }}
    info!("Returning from perform work");
}

// Spawn an async task on the appropriate runtime.
// NOTE: wasm32 does not require a Send bound, but not-wasm32 does.
pub fn perform_async_work<F>(f: F)
where
    F: SpawnableFuture,
{
    spawn!(f);
}

#[cfg(target_arch = "wasm32")]
pub async fn sleep_ms(delay: u64) {
    use wasm_bindgen_futures::js_sys;

    let mut cb = |resolve: js_sys::Function, _reject: js_sys::Function| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, delay as i32)
            .unwrap();
    };

    let p = js_sys::Promise::new(&mut cb);

    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep_ms(delay_ms: u64) {
    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
}

#[macro_export]
macro_rules! spawn {
    ($task:expr) => {
        #[cfg(not(target_arch = "wasm32"))]
        tokio::spawn($task);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local($task);
    };
}
