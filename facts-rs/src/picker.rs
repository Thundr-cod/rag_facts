//! Model picker (port of pick-model.ps1 + bench-decide.ps1).
//! Keeps every .gguf in .system that fits this PC's memory, benchmarks a few (biggest first,
//! GPU vs CPU) and picks the biggest one that runs at MIN_TPS or faster. The pick is cached per
//! computer and per set of model files in .system/.model-choice-<PC>.txt (same format as the
//! PowerShell picker, so existing caches keep working).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MIN_TPS: f64 = 5.0; // below this, chat is slower than reading speed
const MAX_CANDIDATES: usize = 3; // each benchmark takes ~30-90 s

pub struct Model {
    pub name: String,
    pub path: PathBuf,
    pub gb: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Device {
    Gpu,
    Cpu,
}

impl Device {
    pub fn label(self) -> &'static str {
        match self {
            Device::Gpu => "GPU",
            Device::Cpu => "CPU",
        }
    }
    /// -ngl value for llama-server: "auto" fits as many layers as VRAM holds.
    pub fn ngl(self) -> &'static str {
        match self {
            Device::Gpu => "auto",
            Device::Cpu => "0",
        }
    }
}

pub struct Pick {
    pub path: PathBuf,
    pub device: Device,
    pub tps: f64, // 0 = not measured this launch
}

/// GB a model needs: weights + KV cache for the context + ~0.5 GB runtime.
// shortcut: KV is scaled from Qwen3.5-4B (~0.14 GB per 1K context per 4.5 GB of weights); real KV
// depends on the architecture, so the benchmark is the real judge. Upgrade: read the GGUF header.
pub fn need_gb(file_gb: f64, ctx: u32) -> f64 {
    file_gb + (ctx as f64 / 1024.0) * 0.14 * (file_gb / 4.5) + 0.5
}

/// Models that fit, biggest first. If more fit than we bench, keep the biggest few plus the
/// smallest, so a "too slow" failure means even the smallest one is too slow.
pub fn select_candidates(models: &[Model], budget_gb: f64, ctx: u32, max: usize) -> Vec<&Model> {
    let mut fit: Vec<&Model> = models.iter().filter(|m| need_gb(m.gb, ctx) <= budget_gb).collect();
    fit.sort_by(|a, b| b.gb.total_cmp(&a.gb));
    if fit.len() > max {
        let smallest = fit[fit.len() - 1];
        fit.truncate(max - 1);
        fit.push(smallest);
    }
    fit
}

/// Token-generation speed from llama-bench's table: the row with a `tgNN` cell, last number on it.
pub fn parse_tps(out: &str) -> Option<f64> {
    let line = out.lines().find(|l| {
        l.split('|').any(|f| {
            let f = f.trim();
            f.len() > 2 && f.starts_with("tg") && f[2..].bytes().all(|b| b.is_ascii_digit())
        })
    })?;
    line.split('|').rev().find_map(|f| {
        let num: String = f.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        num.parse().ok()
    })
}

/// GPU vs CPU benchmark. GPU must beat CPU by 10% to win. One side failing (e.g. the model only
/// fits with GPU help) lets the other win with 0 for the loser; both failing returns None.
pub fn decide(gpu: Option<f64>, cpu: Option<f64>) -> Option<(Device, f64, f64)> {
    match (gpu, cpu) {
        (None, None) => None,
        (Some(g), None) => Some((Device::Gpu, g, 0.0)),
        (None, Some(c)) => Some((Device::Cpu, c, 0.0)),
        (Some(g), Some(c)) if g > c * 1.1 => Some((Device::Gpu, g, c)),
        (Some(g), Some(c)) => Some((Device::Cpu, c, g)),
    }
}

fn bench(bench_exe: &Path, model: &Path, ngl: &str) -> Option<f64> {
    let out = Command::new(bench_exe)
        .arg("-m").arg(model)
        .args(["-ngl", ngl, "-p", "0", "-n", "32", "--no-warmup"])
        .output()
        .ok()?;
    parse_tps(&String::from_utf8_lossy(&out.stdout))
}

pub fn list_models(system_dir: &Path) -> Vec<Model> {
    // shortcut: split models (name-00001-of-00003.gguf) are sized per part. Upgrade if anyone ships them.
    let Ok(entries) = fs::read_dir(system_dir) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            let meta = e.metadata().ok()?;
            (meta.is_file() && lower.ends_with(".gguf") && !lower.contains("mmproj")).then(|| Model {
                gb: meta.len() as f64 / 1_073_741_824.0,
                path: e.path(),
                name,
            })
        })
        .collect()
}

/// Cache key: every model's name and byte size, plus the context size.
pub fn signature(models: &[Model], ctx: u32) -> String {
    let mut parts: Vec<String> = models
        .iter()
        .map(|m| format!("{}:{}", m.name, (m.gb * 1_073_741_824.0).round() as u64))
        .collect();
    parts.sort();
    format!("{}|ctx={ctx}", parts.join(";"))
}

/// Parses a cached line "OK|<path>|GPU|<tps>|<other>".
pub fn parse_pick(line: &str) -> Option<Pick> {
    let f: Vec<&str> = line.split('|').collect();
    if f.len() < 4 || f[0] != "OK" {
        return None;
    }
    let device = match f[2] {
        "GPU" => Device::Gpu,
        "CPU" => Device::Cpu,
        _ => return None,
    };
    Some(Pick { path: PathBuf::from(f[1]), device, tps: f[3].parse().ok()? })
}

/// NVIDIA VRAM in GB, 0 when there is none (-ngl auto spills what doesn't fit into RAM).
// shortcut: AMD/Intel VRAM isn't read, so those count as RAM-only (on Apple Silicon RAM is the VRAM).
pub fn nvidia_vram_gb() -> f64 {
    Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next()?.trim().parse::<f64>().ok())
        .map_or(0.0, |mib| mib / 1024.0)
}

/// `budget` = usable GB for model + context. `prefer_small` benches smallest first (Intel Macs:
/// CPU-only, where Q8 pushes a 16 GB Mac into swap, so the old launcher always used Q4).
pub fn pick(system_dir: &Path, engine_dir: &Path, budget: f64, ctx: u32, host: &str, prefer_small: bool) -> Result<Pick, String> {
    let models = list_models(system_dir);
    if models.is_empty() {
        return Err("No .gguf model files found in the .system folder.".into());
    }

    let cache_file = system_dir.join(format!(".model-choice-{host}.txt"));
    let sig = signature(&models, ctx);
    if let Ok(text) = fs::read_to_string(&cache_file) {
        let mut lines = text.lines();
        if lines.next() == Some(sig.as_str()) {
            if let Some(p) = lines.next().and_then(parse_pick).filter(|p| p.path.exists()) {
                return Ok(p);
            }
        }
    }

    let mut cands = select_candidates(&models, budget, ctx, MAX_CANDIDATES);
    if prefer_small {
        cands.reverse();
    }
    if cands.is_empty() {
        let small = models.iter().min_by(|a, b| a.gb.total_cmp(&b.gb)).unwrap();
        return Err(format!(
            "Every model is too big for this PC. The smallest ({}) needs about {:.1} GB, this PC has about {budget:.1} GB usable. Add a smaller model (for example a Q4_K_M of a 1-4B model).",
            small.name, need_gb(small.gb, ctx)
        ));
    }

    let bench_exe = engine_dir.join(format!("llama-bench{}", std::env::consts::EXE_SUFFIX));
    if !bench_exe.exists() {
        // No benchmark tool on this engine (some Mac builds): trust the fit check, let -ngl auto place layers.
        return Ok(Pick { path: cands[0].path.clone(), device: Device::Gpu, tps: 0.0 });
    }

    let mut fastest: Option<(&str, f64)> = None;
    for m in &cands {
        println!("  Testing {} ({:.1} GB)...", m.name, m.gb);
        let Some((device, tps, other)) = decide(bench(&bench_exe, &m.path, "99"), bench(&bench_exe, &m.path, "0")) else {
            continue;
        };
        if tps >= MIN_TPS {
            let line = format!("OK|{}|{}|{tps}|{other}", m.path.display(), device.label());
            // Only cache when both devices were measured; a one-sided result may be a transient failure.
            if other > 0.0 {
                let _ = fs::write(&cache_file, format!("{sig}\n{line}\n"));
            }
            return Ok(Pick { path: m.path.clone(), device, tps });
        }
        if fastest.is_none_or(|(_, f)| tps > f) {
            fastest = Some((&m.name, tps));
        }
    }

    match fastest {
        // Every benchmark broke (often antivirus holding llama-bench on first run): don't lock anything
        // in, run the smallest model that fits on CPU this session and test again next launch.
        None => {
            let smallest = cands.iter().min_by(|a, b| a.gb.total_cmp(&b.gb)).unwrap();
            Ok(Pick { path: smallest.path.clone(), device: Device::Cpu, tps: 0.0 })
        }
        Some((name, tps)) => Err(format!(
            "Every model that fits runs too slowly on this PC (best: {name} at {tps} tokens/sec, need {MIN_TPS}+). Add a smaller model (for example a Q4_K_M of a 1-4B model)."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, gb: f64) -> Model {
        Model { name: name.into(), path: PathBuf::from(name), gb }
    }
    fn names(v: Vec<&Model>) -> String {
        v.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(",")
    }

    #[test]
    fn need_matches_measured_4b_model() {
        // Stock 4.5 GB model at 32K: ~9.5 GB (measured ~9.9 GB on an RTX 5070).
        assert_eq!((need_gb(4.5, 32768) * 10.0).round() / 10.0, 9.5);
    }

    #[test]
    fn candidates() {
        let ms = [m("tiny", 1.0), m("small", 2.5), m("mid", 4.5), m("big", 8.0), m("huge", 40.0)];
        assert_eq!(names(select_candidates(&ms, 16.0, 8192, 3)), "big,mid,tiny"); // 4 fit: 2 biggest + smallest
        assert_eq!(names(select_candidates(&ms, 7.0, 8192, 3)), "mid,small,tiny"); // 3 fit: all
        assert_eq!(names(select_candidates(&ms, 16.0, 8192, 2)), "big,tiny");
        assert!(select_candidates(&ms, 1.0, 8192, 3).is_empty()); // nothing fits -> fail out
    }

    #[test]
    fn parses_llama_bench_table() {
        let out = "| model | size | params | backend | ngl | test | t/s |\n\
                   | --- | ---: | ---: | --- | --: | ---: | ---: |\n\
                   | qwen35 4B Q8_0 | 4.17 GiB | 4.21 B | Vulkan | 99 | tg32 | 94.52 ± 1.03 |";
        assert_eq!(parse_tps(out), Some(94.52));
        assert_eq!(parse_tps("error: failed to load model"), None);
    }

    #[test]
    fn gpu_needs_ten_percent_margin() {
        assert_eq!(decide(Some(94.5), Some(9.3)).unwrap().0, Device::Gpu);
        assert_eq!(decide(Some(10.5), Some(10.0)).unwrap().0, Device::Cpu);
        assert_eq!(decide(None, Some(8.0)).unwrap(), (Device::Cpu, 8.0, 0.0));
        assert!(decide(None, None).is_none());
    }

    #[test]
    fn reads_powershell_cache_line() {
        let p = parse_pick(r"OK|D:\facts\.system\Qwen3.5-4B-abliterated.Q8_0.gguf|GPU|94.52|9.33").unwrap();
        assert_eq!((p.device, p.tps), (Device::Gpu, 94.52));
        assert!(parse_pick("FAIL|nope").is_none());
    }
}
