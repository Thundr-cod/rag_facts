//! facts. launcher: wipe history, pick the best model for this PC, run an offline chat, wipe again.
//! Lives at <drive>/.system/facts-<os>; the per-OS launcher scripts only start it.

mod chat;
mod picker;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

use sysinfo::System;

const HELP: &str = "  Need help? Visit opensourceeverything.io and use the support chat.";

#[derive(Clone, Copy, PartialEq)]
enum Platform {
    Windows,
    Linux,
    MacArm,
    MacIntel,
}

fn platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        // Ask the hardware, not the binary: under Rosetta a universal binary runs its x86_64 half
        // on Apple Silicon and would silently fall back to the slow CPU engine.
        let arm = Command::new("sysctl").args(["-n", "hw.optional.arm64"]).output();
        if arm.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1") {
            Platform::MacArm
        } else {
            Platform::MacIntel
        }
    }
}

fn engine_dir(system_dir: &Path, p: Platform) -> PathBuf {
    match p {
        Platform::Windows => system_dir.join("windows"),
        Platform::Linux => system_dir.join("linux"),
        Platform::MacIntel => system_dir.join("x86_64"), // CPU build
        Platform::MacArm => system_dir.to_path_buf(),   // Metal build sits in .system/
    }
}

/// Zero-log: remove any history/session files llama.cpp tools may have written.
fn wipe(root: &Path, system_dir: &Path) {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let mut files = vec![
        root.join("llama.chat.history"),
        root.join("main.session"),
        system_dir.join("llama.chat.history"),
        system_dir.join("main.session"),
    ];
    files.extend(home.map(|h| h.join(".llama_history")));
    for f in files {
        let _ = fs::remove_file(f);
    }
}

/// Clears the screen and scrollback so the conversation isn't left on screen after exit.
fn clear_screen() {
    if cfg!(windows) {
        let _ = Command::new("cmd").args(["/c", "cls"]).status(); // clears scrollback too
    } else {
        let _ = Command::new("clear").status();
        print!("\x1b[3J"); // scrollback; old Windows consoles would print this as garbage
    }
}

fn fail(msg: &str) -> ! {
    println!("\n  [ERROR] {msg}\n{HELP}\n");
    exit(1);
}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("facts {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // FACTS_ROOT lets a developer run the binary from target/; on the drive it sits in .system/.
    let root = std::env::var_os("FACTS_ROOT").map(PathBuf::from).unwrap_or_else(|| {
        let exe = std::env::current_exe().expect("cannot locate facts executable");
        exe.parent().and_then(Path::parent).expect("facts must live in <drive>/.system/").to_path_buf()
    });
    let system_dir = root.join(".system");
    let plat = platform();
    let engine = engine_dir(&system_dir, plat);

    let logo = if cfg!(windows) { "ose-logo-win.txt" } else { "ose-logo.txt" };
    if let Ok(text) = fs::read_to_string(system_dir.join(logo)) {
        print!("{text}");
    }
    println!("----------------------------------------------------------------");
    println!("  facts. {} [{}]", env!("CARGO_PKG_VERSION"), std::env::consts::OS);
    println!("----------------------------------------------------------------");

    wipe(&root, &system_dir);
    {
        let (root, system_dir) = (root.clone(), system_dir.clone());
        // Ctrl+C reaches llama-server too (same console), so only the wipe is left to do here.
        let _ = ctrlc::set_handler(move || {
            wipe(&root, &system_dir);
            clear_screen();
            exit(0);
        });
    }
    println!("  Cache Status: Wiped Clean [Zero-Log Mode]");

    let mut sys = System::new();
    sys.refresh_memory();
    let gib = 1_073_741_824.0;
    let ram_gb = sys.total_memory() as f64 / gib;
    let avail_gb = sys.available_memory() as f64 / gib;
    println!("  Hardware Detected: {ram_gb:.0} GB RAM ({avail_gb:.0} GB available)");
    if avail_gb < 4.0 {
        println!("\n  [WARNING] Low available RAM. Close browsers and other heavy apps for best performance.\n");
    }

    let server_exe = engine.join(format!("llama-server{}", std::env::consts::EXE_SUFFIX));
    if !server_exe.exists() {
        fail(&format!(
            "The AI engine (llama-server) is missing from {}. Your antivirus may have removed it: check its quarantine or history, restore the file and add an exclusion.",
            engine.display()
        ));
    }

    // Context sized to RAM; tiers are nominal-minus-1 because a 16 GB iGPU box reports ~15 GB.
    let ctx = if ram_gb >= 30.0 { 32768 } else if ram_gb >= 15.0 { 16384 } else { 8192 };

    println!("  Choosing the best model for this PC...");
    println!("  [First launch on a new PC benchmarks each model, ~30-90 sec each. It is not frozen.]");
    let host = System::host_name().unwrap_or_else(|| "this-pc".into());
    // Usable memory: RAM minus ~2 GB for the OS, plus NVIDIA VRAM. Macs share RAM with the GPU and
    // the old launcher sized to FREE memory there, so a busy Mac picks a smaller model on purpose.
    // shortcut: that Mac choice is cached like any other; delete .model-choice-*.txt to re-pick.
    let budget = match plat {
        Platform::MacArm | Platform::MacIntel => (ram_gb - 2.0).min(avail_gb),
        _ => ram_gb - 2.0 + picker::nvidia_vram_gb(),
    };
    let pick = picker::pick(&system_dir, &engine, budget, ctx, &host, plat == Platform::MacIntel)
        .unwrap_or_else(|e| fail(&e));
    let name = pick.path.file_name().unwrap_or_default().to_string_lossy();
    if pick.tps > 0.0 {
        println!("  Device: {} [{} tokens/sec]", pick.device.label(), pick.tps);
    } else {
        println!("  Device: {} [not benchmarked this launch]", pick.device.label());
    }
    println!("  Loading: {name} ({}K context)\n", ctx / 1024);
    println!("  LOADING MODEL INTO MEMORY... Do NOT close this window.");

    // The Intel Mac engine is a different llama.cpp build: there --reasoning-budget 0 is ignored and
    // only --reasoning off stops Qwen3.5's thinking output (verified 2026-06-28 in MacLaunch.command).
    let reasoning: [&str; 2] = if plat == Platform::MacIntel { ["--reasoning", "off"] } else { ["--reasoning-budget", "0"] };
    let mut server = chat::start_server(&engine, &pick.path, ctx, pick.device.ngl(), &reasoning)
        .unwrap_or_else(|e| fail(&format!("Could not start the AI engine: {e}")));
    if let Err(e) = chat::wait_ready(&mut server) {
        let _ = server.child.kill();
        fail(&e);
    }
    println!("----------------------------------------------------------------");
    println!("  AI READY. Type below to chat. /clear starts over, /exit quits.");
    println!("----------------------------------------------------------------\n");

    chat::chat_loop(&server);

    let _ = server.child.kill();
    let _ = server.child.wait();
    wipe(&root, &system_dir);
    clear_screen();
    println!("----------------------------------------------------------------");
    println!("  The AI has stopped. Nothing from this chat was saved.");
    println!("  If it stopped unexpectedly: check your antivirus, close other apps to free RAM, then relaunch.");
    println!("{HELP}");
    println!("----------------------------------------------------------------");
}
