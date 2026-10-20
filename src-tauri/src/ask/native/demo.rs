//! Trying and measuring the native quick ask window without the rest of
//! Typr (no Tauri, no microphone, no model):
//!
//! * `typr --ask-demo [chat] [bottom] [ru] [long|code]` plays sample answers
//!   in it, as the old page's browser demo did (`long`: a first answer
//!   taller than the card; `code`: code blocks inside list items).
//! * `typr --ask-bench` measures it: memory, time to the first frame (cold,
//!   warm and as a process of its own), CPU while it unfolds and streams.
//! * `typr --ask-child` is the process the bench starts for that last one.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use super::{send, start};
use crate::ask::window::{Command, Event, Geometry, Layout, Recording, Stats};
use crate::overlay::ScreenRect;
use crate::settings::IndicatorPosition;

pub const DEMO_ARG: &str = "--ask-demo";
pub const BENCH_ARG: &str = "--ask-bench";
pub const CHILD_ARG: &str = "--ask-child";

fn layout(chat: bool, bottom: bool, language: &'static str) -> Layout {
    let (area, scale) = crate::overlay::primary_screen().unwrap_or((
        ScreenRect {
            x: 0,
            y: 0,
            width: 1440,
            height: 900,
        },
        1.0,
    ));
    let position = if bottom {
        IndicatorPosition::BottomCenter
    } else {
        IndicatorPosition::TopRight
    };
    Layout {
        geometry: Geometry::new(position, area, scale),
        bottom,
        language,
        chat,
        hotkey: "Ctrl+Shift+Space".to_string(),
        push_to_talk: false,
    }
}

/// Starts the window's thread; its events come out of the receiver.
fn start_window() -> Receiver<(Event, Instant)> {
    let (events, received) = channel();
    start(move |event| {
        let _ = events.send((event, Instant::now()));
    })
    .expect("the window's thread starts");
    received
}

fn wait_shown(events: &Receiver<(Event, Instant)>) -> Instant {
    loop {
        match events.recv_timeout(Duration::from_secs(10)) {
            Ok((Event::Shown(at), _)) => return at,
            Ok(_) => {}
            Err(_) => panic!("the window never showed"),
        }
    }
}

fn wait_closed(events: &Receiver<(Event, Instant)>) {
    while let Ok((event, _)) = events.recv_timeout(Duration::from_secs(10)) {
        if matches!(event, Event::Closed) {
            send(Command::Destroy);
            return;
        }
    }
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Streams `text` as the backend does: pieces at most every 40 ms
fn stream(id: u64, text: &str, piece: usize, every: u64) {
    let chars: Vec<char> = text.chars().collect();
    for chunk in chars.chunks(piece) {
        send(Command::Delta {
            id,
            text: chunk.iter().collect(),
        });
        sleep(every);
    }
    send(Command::Done {
        id,
        text: text.to_string(),
        stopped: false,
    });
}

const FIRST_EN: &str = "This **regular expression** checks an email address:\n\n- `^[\\w.+-]+` is the name before `@`\n- `@[\\w-]+` is the domain\n- `\\.[a-z]{2,}$` is the zone, like `.com`\n\nIt accepts `anna.k+news@mail.com`, but not `anna@mail`.";
const FIRST_RU: &str = "Это **регулярное выражение** проверяет адрес почты:\n\n- `^[\\w.+-]+` — имя до `@`\n- `@[\\w-]+` — домен\n- `\\.[a-z]{2,}$` — зона, например `.com`\n\nОно пропустит `anna.k+news@mail.ru`, но не `anna@mail`.";
const SECOND_EN: &str = "Yes, here is a stricter one:\n\n```js\nconst email = /^[\\w.+-]+@[\\w-]+(\\.[\\w-]+)*\\.[a-z]{2,}$/i;\n```";
const SECOND_RU: &str = "Да, вот вариант построже:\n\n```js\nconst email = /^[\\w.+-]+@[\\w-]+(\\.[\\w-]+)*\\.[a-z]{2,}$/i;\n```";
const THIRD_EN: &str = "According to [Exa](https://exa.ai), the latest version is **2.1**, released last week.";
const THIRD_RU: &str = "По данным [Exa](https://exa.ai), последняя версия — **2.1**, вышла на прошлой неделе.";
const CHAT_EN: &str = "`const` can't be **reassigned**, `let` can. Both live only inside their block `{}`.\n\n- **`let`** is for values that change: counters, flags, intermediate results.\n- **`const`** is for everything else: it shows the value won't be swapped.\n- **`var`** is best avoided: it is seen in the whole function and hoisted to its top.\n\nAn object in a `const` can still change: only giving the variable another value is forbidden.\n\n```js\nconst user = { name: \"Anna\" };\nuser.name = \"Maria\"; // fine\nuser = {}; // error\n```";
const CHAT_RU: &str = "`const` нельзя **переназначить**, а `let` можно. Оба видны только внутри блока `{}`.\n\n- **`let`** — для значений, которые меняются: счётчики, флаги, промежуточные результаты.\n- **`const`** — для всего остального: сразу видно, что значение не подменят.\n- **`var`** лучше не использовать: он виден во всей функции и поднимается наверх.\n\nОбъект в `const` при этом менять можно: запрещено только присвоить переменной другое значение.\n\n```js\nconst user = { name: \"Анна\" };\nuser.name = \"Мария\"; // можно\nuser = {}; // ошибка\n```";

/// `typr --ask-demo [chat] [bottom] [ru] [long|code]`
pub fn run_demo(args: &[String]) {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let ru = has("ru");
    let chat = has("chat");
    let events = start_window();
    send(Command::Open(layout(chat, has("bottom"), if ru { "ru" } else { "en" })));
    sleep(300);
    send(Command::Reveal);
    wait_shown(&events);
    snapshot(if chat { "c01-shown" } else { "01-dot" });

    if chat {
        let ask = |id: u64, question: &str, selection: Option<&str>| {
            send(Command::Recording(Recording::Recording));
            sleep(1600);
            send(Command::Recording(Recording::Transcribing));
            sleep(500);
            send(Command::Question {
                id,
                text: question.to_string(),
                selection: selection.map(str::to_string),
            });
            send(Command::Answer { id });
            send(Command::Recording(Recording::Ready));
            sleep(700);
        };
        sleep(120);
        snapshot("c02-arriving");
        sleep(250);
        snapshot("c03-unfolding");
        sleep(1400);
        snapshot("c04-empty");
        send(Command::Recording(Recording::Recording));
        sleep(700);
        snapshot("c05-listening");
        ask(
            1,
            if ru { "Чем let отличается от const в JavaScript?" } else { "How is let different from const in JavaScript?" },
            None,
        );
        stream(1, if ru { CHAT_RU } else { CHAT_EN }, 6, 28);
        sleep(600);
        snapshot("c06-answer");
        sleep(900);
        ask(
            2,
            if ru { "Переведи на английский" } else { "Translate this into Russian" },
            Some(if ru {
                "Встреча переносится на четверг, 15:00. Пожалуйста, подтвердите участие."
            } else {
                "The meeting moves to Thursday at 3 pm. Please confirm you can make it."
            }),
        );
        stream(
            2,
            if ru {
                "The meeting has been moved to Thursday at 3 pm. Please confirm your attendance."
            } else {
                "Встреча переносится на четверг, 15:00. Пожалуйста, подтвердите, что сможете прийти."
            },
            6,
            28,
        );
        sleep(600);
        snapshot("c07-translation");
    } else {
        send(Command::Answer { id: 1 });
        sleep(110);
        snapshot("02-bar");
        sleep(220);
        snapshot("03-growing");
        sleep(370);
        snapshot("04-typing");
        let first = if has("long") {
            LONG_ANSWER
        } else if has("code") {
            CODE_IN_LISTS
        } else if ru {
            FIRST_RU
        } else {
            FIRST_EN
        };
        stream(1, first, 6, 28);
        sleep(600);
        snapshot("05-answer");
        sleep(1200);
        send(Command::Recording(Recording::Recording));
        sleep(700);
        snapshot("06-listening");
        sleep(1300);
        send(Command::Recording(Recording::Transcribing));
        send(Command::Answer { id: 2 });
        send(Command::Recording(Recording::Ready));
        sleep(800);
        stream(2, if ru { SECOND_RU } else { SECOND_EN }, 6, 28);
        sleep(600);
        snapshot("07-code");
        sleep(900);
        send(Command::Recording(Recording::Recording));
        sleep(1500);
        send(Command::Recording(Recording::Transcribing));
        send(Command::Answer { id: 3 });
        send(Command::Recording(Recording::Ready));
        sleep(500);
        send(Command::Search { id: 3 });
        sleep(900);
        snapshot("08-search");
        sleep(1700);
        stream(3, if ru { THIRD_RU } else { THIRD_EN }, 6, 28);
        sleep(600);
        snapshot("09-final");
    }
    if std::env::var_os("TYPR_ASK_SNAPSHOTS").is_some() {
        send(Command::Fold);
        sleep(200);
        snapshot("10-folding");
        wait_closed(&events);
        sleep(200);
        return;
    }
    // Time to look, click, copy, scroll; the cross closes it
    println!("Demo playing; close the card with its cross (or wait 60 s)");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((Event::Closed, _)) => break,
            Ok(_) => {}
            Err(_) => {
                send(Command::Fold);
                wait_closed(&events);
                break;
            }
        }
    }
    send(Command::Destroy);
    sleep(200);
}

// ── Measurements ────────────────────────────────────────

#[derive(Clone, Copy)]
struct Memory {
    /// Commit charge: what the process alone holds
    private: f64,
    working_set: f64,
}

fn memory() -> Memory {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        );
    }
    let mb = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    Memory {
        private: mb(counters.PrivateUsage),
        working_set: mb(counters.WorkingSetSize),
    }
}

/// CPU time the process used so far (all threads)
fn cpu_time() -> Duration {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user);
    }
    let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

fn cpu_share(cpu: Duration, wall: Duration) -> f64 {
    cpu.as_secs_f64() / wall.as_secs_f64() * 100.0
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
}

/// Code blocks indented under list items, and a closing fence with a
/// language (a model's slip)
const CODE_IN_LISTS: &str = "Для большинства дистрибутивов Linux достаточно установить пакет из репозиториев:\n\n- **Debian / Ubuntu / Mint**  \n  ```bash\n  sudo apt update\n  sudo apt install neofetch\n  ```\n\n- **Fedora / RHEL / CentOS**  \n  ```bash\n  sudo dnf install neofetch      # (или yum на старых версиях)\n  ```\n\n- **Arch Linux**  \n  ```bash\n  sudo pacman -S neofetch\n  ```\n\nЕсли в репозитории нет neofetch, можно собрать его из исходников:\n\n```bash\ngit clone https://github.com/dylanaraps/neofetch\ncd neofetch\nmake install   # возможно понадобится sudo\n```bash\n\nПосле установки запустите `neofetch` в терминале.";

const LONG_ANSWER: &str = "## How the cache works\n\nThe cache keeps **recent results** in memory, so the second request for the same key is *much* faster. Entries expire after `ttl` seconds.\n\n1. A request comes in with a key.\n2. If the key is in the cache and fresh, the value is returned at once.\n3. Otherwise the value is computed, stored and returned.\n\n> Stale entries are removed lazily, on the next read.\n\n```rust\nfn get(&mut self, key: &str) -> Option<&Value> {\n    let entry = self.map.get(key)?;\n    if entry.expires < Instant::now() {\n        return None;\n    }\n    Some(&entry.value)\n}\n```\n\n| Setting | Default |\n|---|---|\n| ttl | 60 s |\n| size | 1000 |\n\nA few more notes:\n\n- Keys are case-sensitive.\n- Values larger than **1 MB** are never cached.\n- See [the docs](https://example.com/docs) for details.\n\nThat is all there is to it: a map, a clock and a limit. Most of the time you won't need to tune anything, but when you do, start with `ttl` and watch the hit rate.";

/// What the window's thread has done so far
fn thread_stats() -> Stats {
    let (done, wait) = channel();
    send(Command::Stats(Box::new(move |stats| {
        let _ = done.send(stats);
    })));
    wait.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
}

/// CPU cycles a busy thread runs per second (the TSC's rate), to turn the
/// window thread's cycle count into time
fn cycles_per_second() -> f64 {
    let cycles = super::platform::thread_cycles;
    let (start, at) = (cycles(), Instant::now());
    let mut x = 0u64;
    while at.elapsed() < Duration::from_millis(300) {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
    std::hint::black_box(x);
    (cycles() - start) as f64 / at.elapsed().as_secs_f64()
}

/// One phase of the bench: frames, drawing time and the window thread's CPU
struct Phase {
    name: String,
    frames: u64,
    render_ms: f64,
    present_ms: f64,
    slowest_ms: f64,
    /// Average share of the window redrawn per frame
    area: f64,
    cpu: f64,
    process_cpu: f64,
}

fn measure(name: &str, rate: f64, act: impl FnOnce()) -> Phase {
    let (before, cpu0, wall0) = (thread_stats(), cpu_time(), Instant::now());
    act();
    let (after, cpu1, wall) = (thread_stats(), cpu_time(), wall0.elapsed());
    let frames = after.frames - before.frames;
    Phase {
        name: name.to_string(),
        frames,
        render_ms: (after.render - before.render).as_secs_f64() * 1000.0,
        present_ms: (after.present - before.present).as_secs_f64() * 1000.0,
        slowest_ms: after.slowest.as_secs_f64() * 1000.0,
        area: if frames > 0 { (after.area - before.area) as f64 / frames as f64 } else { 0.0 },
        cpu: (after.cycles - before.cycles) as f64 / rate / wall.as_secs_f64() * 100.0,
        process_cpu: cpu_share(cpu1 - cpu0, wall),
    }
}

/// `typr --ask-bench`: prints the measurements.
pub fn run_bench() {
    let exe = std::env::current_exe().expect("own path");
    let exe_size = std::fs::metadata(&exe).map(|m| m.len() as f64 / (1024.0 * 1024.0)).unwrap_or(0.0);
    println!("Executable: {} ({:.1} MB)", exe.display(), exe_size);
    let rate = cycles_per_second();
    sleep(300);
    let before = memory();

    // Cold: the thread, Slint and the fonts start with the first window
    let sent = Instant::now();
    let events = start_window();
    send(Command::Open(layout(false, false, "en")));
    send(Command::Reveal);
    let cold = wait_shown(&events) - sent;

    let mut phases = Vec::new();
    // The card unfolds: dot, bar, card, the typing dots
    phases.push(measure("Unfolding, then typing dots (0.7 s)", rate, || {
        send(Command::Answer { id: 1 });
        sleep(700);
    }));
    let open = memory();

    // A long answer streams in, 6 characters every 40 ms (the backend's
    // fastest), growing the card to its tallest and scrolling after that
    let static_layout = std::env::var("TYPR_ASK_LAYOUT").as_deref() == Ok("static");
    phases.push(measure(
        &format!("Streaming {} characters, 6 every 40 ms", LONG_ANSWER.chars().count()),
        rate,
        || {
            if static_layout {
                send(Command::Done {
                    id: 1,
                    text: LONG_ANSWER.to_string(),
                    stopped: false,
                });
            } else {
                stream(1, LONG_ANSWER, 6, 40);
            }
            sleep(300);
        },
    ));
    let streamed = memory();
    snapshot("bench-long");
    if std::env::var_os("TYPR_ASK_LAYOUT").is_some() {
        send(Command::Fold);
        wait_closed(&events);
        return;
    }

    // Still: nothing moves, nothing should be drawn
    phases.push(measure("Still, card open (2 s)", rate, || sleep(2000)));

    // Listening: the ring around the card and the light pulse
    send(Command::Recording(Recording::Recording));
    sleep(300);
    phases.push(measure("Listening: ring + pulse (2 s)", rate, || sleep(2000)));
    send(Command::Recording(Recording::Transcribing));
    send(Command::Recording(Recording::Ready));

    phases.push(measure("Folding away (0.6 s)", rate, || {
        send(Command::Fold);
        wait_closed(&events);
    }));
    sleep(1500);
    let idle = memory();

    // Warm: the thread and the fonts are there
    let mut warm = Vec::new();
    for _ in 0..10 {
        let sent = Instant::now();
        send(Command::Open(layout(false, false, "en")));
        send(Command::Reveal);
        warm.push((wait_shown(&events) - sent).as_secs_f64() * 1000.0);
        sleep(250);
        send(Command::Destroy);
        sleep(150);
    }
    sleep(1000);
    let idle_after = memory();

    // A process of its own for each window, as local recognition does
    let mut spawned = Vec::new();
    let mut child_memory = Vec::new();
    for _ in 0..5 {
        let sent = Instant::now();
        let mut child = std::process::Command::new(&exe)
            .arg(CHILD_ARG)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("the child starts");
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        for line in lines.by_ref() {
            let line = line.unwrap_or_default();
            if line == "shown" {
                spawned.push(sent.elapsed().as_secs_f64() * 1000.0);
            } else if let Some(mb) = line.strip_prefix("memory ") {
                child_memory.push(mb.to_string());
                break;
            }
        }
        let _ = child.wait();
        sleep(200);
    }

    println!();
    println!("| Measure | Value |");
    println!("|---|---|");
    println!("| Process before the window (private / working set) | {:.1} / {:.1} MB |", before.private, before.working_set);
    println!("| Card open, unfolded (private / working set) | {:.1} / {:.1} MB |", open.private, open.working_set);
    println!("| After a long answer streamed (private / working set) | {:.1} / {:.1} MB |", streamed.private, streamed.working_set);
    println!("| Idle after closing (private / working set) | {:.1} / {:.1} MB |", idle.private, idle.working_set);
    println!("| Idle after 11 windows (private / working set) | {:.1} / {:.1} MB |", idle_after.private, idle_after.working_set);
    println!("| First frame on screen, cold (thread + Slint + fonts + window) | {:.0} ms |", cold.as_secs_f64() * 1000.0);
    println!("| First frame on screen, warm (median of 10, max) | {:.1} ms (max {:.1}) |", median(&mut warm.clone()), warm.iter().cloned().fold(0.0, f64::max));
    println!("| First frame on screen, own process (median of 5) | {:.0} ms |", median(&mut spawned.clone()));
    println!("| Own process memory with the card open | {} |", child_memory.first().cloned().unwrap_or_default());
    println!();
    println!("| Phase | Frames | Pixels per frame | Drawing | Presenting | Slowest frame | Window thread CPU | Process CPU (15.6 ms ticks) |");
    println!("|---|---|---|---|---|---|---|---|");
    for p in &phases {
        println!(
            "| {} | {} | {:.0} | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} % | {:.1} % |",
            p.name, p.frames, p.area, p.render_ms, p.present_ms, p.slowest_ms, p.cpu, p.process_cpu
        );
    }
    let _ = std::io::stdout().flush();
}

/// `typr --ask-child`: opens one window, says when it shows, how much
/// memory the process holds once it unfolded, and exits.
pub fn run_child() {
    let events = start_window();
    send(Command::Open(layout(false, false, "en")));
    send(Command::Reveal);
    wait_shown(&events);
    println!("shown");
    let _ = std::io::stdout().flush();
    send(Command::Answer { id: 1 });
    sleep(700);
    let m = memory();
    println!("memory {:.1} / {:.1} MB", m.private, m.working_set);
    let _ = std::io::stdout().flush();
    send(Command::Destroy);
    sleep(100);
}

// ── Snapshots ───────────────────────────────────────────

/// With `TYPR_ASK_SNAPSHOTS` set to a folder, the demo saves the window's
/// frame there as `<name>.png`, over a grey checkerboard (or a dark desktop
/// with `TYPR_ASK_DARK`)
fn snapshot(name: &str) {
    let Some(dir) = std::env::var_os("TYPR_ASK_SNAPSHOTS") else { return };
    let path = std::path::PathBuf::from(dir).join(format!("{}.png", name));
    let (done, wait) = channel();
    send(Command::Snapshot(Box::new(move |pixels| {
        if let Some((width, height, rgba)) = pixels {
            let mut over = Vec::with_capacity(rgba.len());
            for (i, pixel) in rgba.chunks(4).enumerate() {
                let (x, y) = (i as u32 % width, i as u32 / width);
                let back: u16 = if std::env::var_os("TYPR_ASK_DARK").is_some() {
                    0x12
                } else if (x / 12 + y / 12) % 2 == 0 {
                    0x3a
                } else {
                    0x44
                };
                let a = pixel[3] as u16;
                for c in &pixel[..3] {
                    over.push(((*c as u16 * a + back * (255 - a)) / 255) as u8);
                }
                over.push(255);
            }
            if let Err(e) = write_png(&path, width, height, &over) {
                eprintln!("Snapshot {}: {}", path.display(), e);
            }
        }
        let _ = done.send(());
    })));
    let _ = wait.recv_timeout(Duration::from_secs(5));
}

/// A PNG without compression: enough for a few snapshots
fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let crc_table: Vec<u32> = (0..256u32)
        .map(|n| (0..8).fold(n, |c, _| if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 }))
        .collect();
    let crc = |bytes: &[u8]| !bytes.iter().fold(!0u32, |c, b| crc_table[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8));
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in rgba.chunks(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push((i + 1 == blocks.len()) as u8);
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut chunk = |kind: &[u8], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        png.extend_from_slice(&body);
        png.extend_from_slice(&crc(&body).to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    std::fs::write(path, png)
}
