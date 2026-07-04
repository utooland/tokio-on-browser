#![cfg(all(target_family = "wasm", target_os = "unknown"))]

extern crate console_error_panic_hook;

use std::{
    future::Future,
    panic,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
    time::Duration,
};

use futures::FutureExt;
use tokio::{runtime, task_local, time::Instant};
use tokio_fs_ext::{create_dir_all, offload, read_to_string, write};
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
pub async fn mkdir(path: &str) -> Result<(), JsValue> {
    create_dir_all(path).await.map_err(|e| e.to_string().into())
}

#[wasm_bindgen(js_name = writeFile)]
pub async fn write_file(path: &str, content: &str) -> Result<(), JsValue> {
    write(path, content.as_bytes())
        .await
        .map_err(|e| e.to_string().into())
}

#[wasm_bindgen(js_name = readFile)]
pub async fn read_file(path: &str) -> Result<String, JsValue> {
    read_to_string(path).await.map_err(|e| e.to_string().into())
}

task_local! {
    static TT: u8;
}

const HELLO_PATH: &str = "hello";

const SOMETHING: &str = " s o m e t h i n g ";

#[wasm_bindgen]
pub async fn run(stress_test_iterations: Option<usize>) -> Result<String, String> {
    let stress_test_iterations = stress_test_iterations.unwrap_or(0);
    tracing::info!("start task in thread {:?}", std::thread::current().id());

    tokio_fs_ext::write(HELLO_PATH, "world d d d d".as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let (mut fs_server, fs_client_0) = offload::split();

    let fs_client_1 = fs_client_0.clone();
    let fs_client_2 = fs_client_0.clone();
    let fs_client_3 = fs_client_0.clone();
    let fs_client_4 = fs_client_0.clone();

    // Clone once to pass owned copies to the main Tokio task
    let tokio_task = TOKIO_RUNTIME.spawn(async move {
        let fs_client_stress = fs_client_0.clone();
        fs_client_0
            .watch_dir("/", true, |event| {
                tokio_fs_ext::console::log!("fs_event: {event:?}")
            })
            .await
            .unwrap();

        let ret = run_basic_tests(
            fs_client_0,
            fs_client_1,
            fs_client_2,
            fs_client_3,
            fs_client_4,
        )
        .await?;

        if stress_test_iterations > 0 {
            run_stress_tests(fs_client_stress, stress_test_iterations).await?;
        }

        Result::<String, String>::Ok(ret)
    });

    let (ret, _) = futures::future::join(
        tokio_task,
        // start the fs_actor, because calling the opfs api in tokio runtime will
        // cause hanging forever, the reason is that, calling between worker threads
        // on browser is scheduled in browser's event loop, but tokio runtime worker will
        // blocking or park the thread, so the browser's event loop will be blocked.
        // See: https://github.com/tokio-rs/tokio/blob/925c614c89d0a26777a334612e2ed6ad0e7935c3/tokio/src/runtime/scheduler/multi_thread/worker.rs#L524:L567
        // and https://github.com/chemicstry/wasm_thread/blob/7ec48686bb1a0d9bd42cf16e46622746e4d12ab3/README.md?plain=1#L22:L26
        fs_server.serve(offload::FsOffloadDefault),
    )
    .await;

    let ret = ret.map_err(|e| e.to_string())??;

    tracing::info!("read_to_string end {:?} {}", HELLO_PATH, ret);

    Ok(ret)
}

async fn run_basic_tests(
    fs_client_0: offload::Client,
    fs_client_1: offload::Client,
    fs_client_2: offload::Client,
    fs_client_3: offload::Client,
    fs_client_4: offload::Client,
) -> Result<String, String> {
    // Clone for the scoped task
    let scoped_task = tokio::spawn(async move {
        // Ensure tokio::task::LocalKey::scope worked
        TT.scope(1, async move {
            let ret = fs_client_0
                .read(HELLO_PATH)
                .await
                .map(|buf| unsafe { String::from_utf8_unchecked(buf) })
                .map_err(|e| e.to_string())?;

            let thread_id = std::thread::current().id();
            tracing::info!(
                "read_to_string {:?} {} in tokio worker thread: {:?}",
                HELLO_PATH,
                ret,
                thread_id
            );
            Result::<String, String>::Ok(ret)
        })
        .await
    });

    tokio::join!(
        write_and_read_test(fs_client_1, "/1/1"),
        write_and_read_test(fs_client_2, "/2/2"),
        write_and_read_test(fs_client_3, "/3/3")
    );

    let normal_task = tokio::task::spawn(async move {
        let ret = fs_client_4
            .read(HELLO_PATH)
            .await
            .map(|buf| unsafe { String::from_utf8_unchecked(buf) })
            .map_err(|e| e.to_string())?;

        let thread_id = std::thread::current().id();
        tracing::info!(
            "read_to_string {:?} {} in tokio worker thread: {:?}",
            HELLO_PATH,
            ret,
            thread_id
        );
        Result::<String, String>::Ok(ret)
    });

    // Ensure tokio::task::spawn blocking worked
    let _ = tokio::join!(
        tokio::task::spawn_blocking(|| blocking_task(40, 40)),
        tokio::task::spawn_blocking(|| blocking_task(20, 20)),
        tokio::task::spawn_blocking(|| blocking_task(10, 10)),
        tokio::task::spawn_blocking(|| blocking_task(30, 30)),
    );

    let (ret_1, ret_2) = tokio::join!(
        async { scoped_task.await.map_err(|e| e.to_string())? },
        async { normal_task.await.map_err(|e| e.to_string())? }
    );

    let ret_1 = ret_1.map_err(|e| e.to_string())?;
    let ret_2 = ret_2.map_err(|e| e.to_string())?;

    assert_eq!(ret_1, ret_2);

    Ok(ret_1)
}

async fn run_stress_tests(
    fs_client_stress: offload::Client,
    stress_test_iterations: usize,
) -> Result<(), String> {
    const CONCURRENCY: usize = 50;

    // Independent files stress test
    run_concurrent_stress_test(
        "Stress test (independent files)",
        fs_client_stress.clone(),
        CONCURRENCY,
        stress_test_iterations,
        stress_test_task,
    )
    .await?;

    // Shared file stress test
    // Ensure directory exists
    fs_client_stress
        .create_dir_all("/stress")
        .await
        .map_err(|e| e.to_string())?;

    run_concurrent_stress_test(
        "Shared file stress test",
        fs_client_stress.clone(),
        CONCURRENCY,
        stress_test_iterations,
        stress_test_shared_file_task,
    )
    .await?;

    // FS ops stress test
    run_concurrent_stress_test(
        "FS ops stress test (create/move/delete)",
        fs_client_stress.clone(),
        CONCURRENCY,
        stress_test_iterations,
        stress_test_fs_ops_task,
    )
    .await?;

    // Race condition stress test
    run_concurrent_stress_test(
        "Race condition stress test (same file create/delete/write)",
        fs_client_stress.clone(),
        CONCURRENCY,
        stress_test_iterations,
        stress_test_race_condition_task,
    )
    .await?;

    Ok(())
}

async fn run_concurrent_stress_test<F, Fut>(
    test_name: &str,
    fs_client: offload::Client,
    concurrency: usize,
    iterations: usize,
    task_fn: F,
) -> Result<(), String>
where
    F: Fn(offload::Client, usize, usize) -> Fut + Send + Sync + Copy + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    tracing::info!("Starting {}...", test_name);
    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..concurrency {
        let client = fs_client.clone();
        handles.push(tokio::spawn(async move {
            task_fn(client, i, iterations).await
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| e.to_string())??;
    }
    tracing::info!("{} completed in {:?}", test_name, start.elapsed());
    Ok(())
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
            format!(r#"tokio-runtime-worker-{id}"#)
        })
        .wasm_bindgen_shim_url("http://localhost:9091/tokio_worker.js".to_string())
        .build()
        .expect("Failed to build Tokio runtime")
});

async fn write_and_read_test(fs_client: offload::Client, dir: &str) -> Result<String, String> {
    fs_client
        .create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;

    let path = format!("{dir}/rw");

    fs_client
        .write(&path, SOMETHING.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let ret = fs_client
        .read(&path)
        .await
        .map(|buf| unsafe { String::from_utf8_unchecked(buf) })
        .map_err(|e| e.to_string())?;

    assert_eq!(ret, SOMETHING);

    let thread_id = std::thread::current().id();
    tracing::info!(
        "read_to_string {:?} {} in tokio worker thread: {:?}",
        &path,
        ret,
        thread_id
    );

    Ok(ret)
}

fn blocking_task(id: u32, n: u64) {
    let tokio_handle_id = tokio::runtime::Handle::current().id();
    tracing::info!(
        "[Task {}] Starting blocking task in tokio worker {}...",
        id,
        tokio_handle_id
    );

    let start = Instant::now();

    let result = fibonacci(n);

    let duration = start.elapsed();

    let thread_id = std::thread::current().id();

    tracing::info!(
        "[Task {}] Fibonacci({}) calculated to be {} in {:?} in tokio worker thread: {:?}",
        id,
        n,
        result,
        duration,
        thread_id
    );
}

fn fibonacci(n: u64) -> u64 {
    if n <= 1 {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}

async fn stress_test_task(
    fs_client: offload::Client,
    id: usize,
    iterations: usize,
) -> Result<(), String> {
    let base_path = format!("/stress/task_{}", id);
    fs_client
        .create_dir_all(&base_path)
        .await
        .map_err(|e| e.to_string())?;

    for i in 0..iterations {
        let file_path = format!("{}/file_{}.txt", base_path, i);
        let content = format!("content_task_{}_iter_{}", id, i);

        // Write
        fs_client
            .write(&file_path, content.as_bytes())
            .await
            .map_err(|e| e.to_string())?;

        // Read and verify
        let read_content = fs_client
            .read(&file_path)
            .await
            .map_err(|e| e.to_string())?;
        let read_string = unsafe { String::from_utf8_unchecked(read_content) };

        if read_string != content {
            return Err(format!(
                "Stress test mismatch in {}: expected '{}', got '{}'",
                file_path, content, read_string
            ));
        }
    }
    
    if id % 10 == 0 {
        tracing::info!("Stress task {} completed {} iterations", id, iterations);
    }
    
    Ok(())
}

async fn stress_test_shared_file_task(
    fs_client: offload::Client,
    id: usize,
    iterations: usize,
) -> Result<(), String> {
    let file_path = "/stress/shared_file.txt";

    for i in 0..iterations {
        let content = format!("shared_content_task_{}_iter_{}", id, i);

        // Write
        fs_client
            .write(file_path, content.as_bytes())
            .await
            .map_err(|e| e.to_string())?;

        // Read
        let read_content = fs_client
            .read(file_path)
            .await
            .map_err(|e| e.to_string())?;
        
        // Verify valid UTF-8 and basic structure
        let read_string = String::from_utf8(read_content)
            .map_err(|e| format!("Invalid UTF-8 read in shared test: {}", e))?;

        if !read_string.starts_with("shared_content_task_") {
             return Err(format!(
                "Shared file stress test corruption in {}: got '{}'",
                file_path, read_string
            ));
        }
    }
    
    if id % 10 == 0 {
        tracing::info!("Shared file stress task {} completed {} iterations", id, iterations);
    }
    
    Ok(())
}

async fn stress_test_fs_ops_task(
    fs_client: offload::Client,
    id: usize,
    iterations: usize,
) -> Result<(), String> {
    let base_path = format!("/stress/ops_task_{}", id);
    fs_client
        .create_dir_all(&base_path)
        .await
        .map_err(|e| e.to_string())?;

    for i in 0..iterations {
        let dir_path = format!("{}/dir_{}", base_path, i);
        let file_path = format!("{}/file.txt", dir_path);
        let renamed_file_path = format!("{}/renamed_file.txt", dir_path);
        let renamed_dir_path = format!("{}/renamed_dir_{}", base_path, i);

        // Create dir
        fs_client
            .create_dir_all(&dir_path)
            .await
            .map_err(|e| e.to_string())?;

        // Create file
        fs_client
            .write(&file_path, b"content")
            .await
            .map_err(|e| e.to_string())?;

        // Rename file - Not supported by Client yet
        // fs_client
        //     .rename(&file_path, &renamed_file_path)
        //     .await
        //     .map_err(|e| e.to_string())?;

        // Rename dir - Not supported by Client yet
        // fs_client
        //     .rename(&dir_path, &renamed_dir_path)
        //     .await
        //     .map_err(|e| e.to_string())?;

        // Remove file
        fs_client
            .remove_file(&file_path)
            .await
            .map_err(|e| e.to_string())?;

        // Remove dir
        fs_client
            .remove_dir(&dir_path)
            .await
            .map_err(|e| e.to_string())?;
    }

    if id % 10 == 0 {
        tracing::info!("FS ops task {} completed {} iterations", id, iterations);
    }

    Ok(())
}

async fn stress_test_race_condition_task(
    fs_client: offload::Client,
    id: usize,
    iterations: usize,
) -> Result<(), String> {
    let file_path = "/stress/race_file.txt";
    // Increase content size to hold the lock longer (10KB)
    let large_content = "a".repeat(1024 * 10);

    for i in 0..iterations {
        let content = format!("race_content_{}_{}_{}", id, i, large_content);

        let client_write = fs_client.clone();
        let client_read = fs_client.clone();

        // Try to write and read concurrently to trigger lock contention (Permission Denied)
        // This simulates the scenario where a handle is not yet released when another operation starts.
        let (write_res, read_res) = tokio::join!(
            client_write.write(file_path, content.as_bytes()),
            client_read.read(file_path)
        );

        if let Err(e) = write_res {
            // Log error but continue to keep the pressure on
            tracing::warn!("Race write error task {}: {}", id, e);
        }

        if let Err(e) = read_res {
             tracing::warn!("Race read error task {}: {}", id, e);
        }

        // Try to remove
        if let Err(e) = fs_client.remove_file(file_path).await {
             tracing::warn!("Race remove error task {}: {}", id, e);
        }
    }
    
    if id % 10 == 0 {
        tracing::info!("Race condition task {} completed {} iterations", id, iterations);
    }
    
    Ok(())
}
