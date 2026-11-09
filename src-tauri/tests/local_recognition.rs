//! Transcribes the 16 kHz mono WAV in TYPR_TEST_WAV with the runtime and
//! Parakeet Ultra already downloaded to the app folder, on the GPU and the CPU,
//! in the recognition process (this build's typr.exe), and checks that
//! Typr's own memory stays small and the process ends on unload:
//! `cargo test --test local_recognition -- --ignored --nocapture`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use typr_lib::local::{catalog, runtime::Paths, LocalEngine};

/// (working set, private bytes) of a process, in MB
fn memory(pid: u32) -> (u64, u64) {
    let script = format!("$p = Get-Process -Id {}; '{{0}} {{1}}' -f $p.WorkingSet64, $p.PrivateMemorySize64", pid);
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .unwrap();
    let numbers: Vec<u64> = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    (numbers[0] >> 20, numbers[1] >> 20)
}

/// Recognition processes this test started
fn children() -> usize {
    let script = format!(
        "@(Get-CimInstance Win32_Process -Filter 'ParentProcessId={}' | Where-Object Name -eq 'typr.exe').Count",
        std::process::id()
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn transcribes_in_the_recognition_process() {
    typr_lib::logger::init();
    let wav = std::env::var("TYPR_TEST_WAV").expect("set TYPR_TEST_WAV to a 16 kHz mono WAV");
    let mut reader = hound::WavReader::open(&wav).unwrap();
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();

    let engine = LocalEngine::with_worker_exe(
        Paths::new(Paths::default_root()),
        PathBuf::from(env!("CARGO_BIN_EXE_typr")),
    );
    let status = engine.status();
    assert!(status.runtime.installed, "download the components in Engine → Local first");
    assert!(
        status.models.iter().any(|m| m.id == catalog::DEFAULT_MODEL && m.installed),
        "download Parakeet Ultra in Engine → Local first"
    );

    let before = memory(std::process::id());
    println!("Typr before: working set {} MB, private {} MB", before.0, before.1);

    for backend in ["vulkan", "cpu"] {
        let cold = Instant::now();
        let text = engine
            .transcribe(catalog::DEFAULT_MODEL, backend, None, samples.clone())
            .await
            .unwrap();
        println!("{} (start + load + run): {} ms", backend, cold.elapsed().as_millis());
        let warm = Instant::now();
        engine
            .transcribe(catalog::DEFAULT_MODEL, backend, None, samples.clone())
            .await
            .unwrap();
        println!("{} (warm run): {} ms", backend, warm.elapsed().as_millis());
        println!("{}: {}", backend, text);
        println!("loaded: {:?}", engine.status().loaded);
        assert!(text.to_lowercase().contains("country"), "{}", text);

        let during = memory(std::process::id());
        println!("Typr with the model loaded: working set {} MB, private {} MB", during.0, during.1);
        assert!(during.1 < before.1 + 15, "the model must live in the recognition process");
    }
    assert_eq!(children(), 1);

    engine.unload("test");
    let deadline = Instant::now() + Duration::from_secs(10);
    while engine.status().loaded.is_some() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(engine.status().loaded.is_none());
    assert_eq!(children(), 0, "the recognition process must end on unload");
    let after = memory(std::process::id());
    println!("Typr after unload: working set {} MB, private {} MB", after.0, after.1);
}
