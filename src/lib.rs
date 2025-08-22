#![cfg(all(target_family = "wasm", target_os = "unknown"))]

extern crate console_error_panic_hook;

use std::{
    future::Future,
    panic,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
    time::Duration,
};

use futures::FutureExt;
use tokio::{runtime, task_local, time::Instant};
use tokio_fs_ext::{
    create_dir_all,
    offload::{self, FsOffload},
    read_to_string, write,
};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    registry,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

use tracing_web::MakeWebConsoleWriter;
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

#[wasm_bindgen]
pub async fn mkdir(path: &str) {
    create_dir_all(path).await.unwrap()
}

#[wasm_bindgen(js_name = writeFile)]
pub async fn write_file(path: &str, content: &str) {
    write(path, content.as_bytes()).await.unwrap()
}

#[wasm_bindgen(js_name = readFile)]
pub async fn read_file(path: &str) -> String {
    read_to_string(path).await.unwrap()
}

task_local! {
    static TT: u8;
}

#[wasm_bindgen]
pub async fn run() -> Result<String, JsValue> {
    tracing::info!("start task");

    let path = "hello".to_string();

    tokio_fs_ext::write(&path, "world d d d d".as_bytes()).await;

    let (fs_actor, fs_actor_handle) = offload::FsActor::create();

    let path_cloned = path.clone();
    let path_cloned_cloned = path.clone();
    let fs_actor_handle_cloned = fs_actor_handle.clone();
    let wrap = TOKIO_RUNTIME.spawn(async move {
        let scoped_task = tokio::spawn(async move {
            // Ensure tokio::task::LocalKey::scope worked
            TT.scope(1, async move {
                tokio::task::spawn(async move {

                    let ret = unsafe {
                        String::from_utf8_unchecked(
                            fs_actor_handle_cloned.read(path_cloned_cloned.clone().into()).await.unwrap(),
                        )
                    };

                    let thread_id = std::thread::current().id();
                    tracing::info!("read_to_string {path_cloned_cloned} {ret} in tokio worker thread: {thread_id:?}");
                    Result::<String, String>::Ok(ret)
                })
                .await
                .unwrap()
            })
            .await
        });

        let path_cloned = path_cloned.clone();
        let fs_actor_handle = fs_actor_handle.clone();

        let normal_task = tokio::task::spawn( async move {
            let path_cloned = path_cloned.clone();
            let path_cloned_cloned = path_cloned.clone();

            let ret = unsafe {
                String::from_utf8_unchecked(
                    fs_actor_handle.read(path_cloned.into()).await.unwrap(),
                )
            };

            let thread_id = std::thread::current().id();
            tracing::info!("read_to_string {path_cloned_cloned} {ret} in tokio worker thread: {thread_id:?}");
            Result::<String, String>::Ok(ret)
        });

        // Ensure tokio::task::spawn blocking worked
        tokio::join!(
            tokio::task::spawn(blocking_task(4, 40)) ,
            tokio::task::spawn(blocking_task(3, 30)) ,
            tokio::task::spawn(blocking_task(2, 20)) ,
            tokio::task::spawn(blocking_task(1, 10)),
        );


        let (ret_1, ret_2) = tokio::join!(
            async { scoped_task.await.unwrap().unwrap() },
            async { normal_task.await.unwrap().unwrap() }
        );

        assert_eq!(ret_1,ret_2);

        ret_1
    });

    let (ret, _) = futures::future::join(
        wrap,
        // start the fs_actor, because calling the opfs api in tokio runtime will
        // cause hanging forever, the reason is that, calling betweean worker threads
        // on browser is scheduled in browser's event loop, but tokio runtime worker will
        // blocking or park the thread, so the browser's event loop will be blocked.
        // See: https://github.com/tokio-rs/tokio/blob/925c614c89d0a26777a334612e2ed6ad0e7935c3/tokio/src/runtime/scheduler/multi_thread/worker.rs#L524:L567
        // and https://github.com/chemicstry/wasm_thread/blob/7ec48686bb1a0d9bd42cf16e46622746e4d12ab3/README.md?plain=1#L22:L26
        fs_actor.run(),
    )
    .await;

    let ret = ret.map_err(|e| e.to_string())?;

    tracing::info!("read_to_string end {path} {ret}");

    Ok(ret)
}

#[wasm_bindgen(start)]
fn start() {
    panic::set_hook(Box::new(console_error_panic_hook::hook));

    let fmt_layer = fmt::layer()
        .without_time()
        .with_span_events(FmtSpan::ENTER | FmtSpan::CLOSE)
        .with_writer(MakeWebConsoleWriter::new())
        .with_filter(EnvFilter::new({
            ["tokio=debug", "tokio_fs_ext=debug", "tokio_browser=debug"].join(",")
        }));

    registry().with(fmt_layer).init();
}

pub static TOKIO_RUNTIME: LazyLock<runtime::Runtime> = LazyLock::new(|| {
    runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name_fn(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!("tokio-runtime-worker-{id}")
        })
        .wasm_bindgen_shim_url("http://localhost:9091/tokio_worker.js".to_string())
        .build()
        .unwrap()
});

async fn blocking_task(id: u32, n: u64) {
    let tokio_handle_id = tokio::runtime::Handle::current().id();
    tracing::info!("[Task {id}] Starting blocking task in {tokio_handle_id}...");

    let start = Instant::now();

    let result = fibonacci(n);

    let duration = start.elapsed();

    let thread_id = std::thread::current().id();

    tracing::info!(
        "[Task {id}] Fibonacci({n}) calculated to be {result} in {duration:?} in tokio worker thread: {thread_id:?}"
    );
}

fn fibonacci(n: u64) -> u64 {
    if n <= 1 {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}
