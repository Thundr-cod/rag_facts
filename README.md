# facts. v1.5

**Offline, uncensored, zero-log AI on a flash drive. Premium edition.**

No internet. No installation. No accounts. Plug in, double-click, ask anything.

Built by [Open Source Everything](https://opensourceeverything.io), *for the people, by the people.*

> **This is the v1.5 branch.** If you have the original facts. drive (model files named
> `Qwen3-4B-Instruct-2507-*.gguf`), use the [main branch](https://github.com/WEAREOSE/facts)
> instead. Not sure? Open the drive's hidden `.system` folder: v1.5 drives have
> `Qwen3.5-4B-abliterated` model files.

---

## What Is This?

This is the complete, open-source build for the **facts. v1.5** AI flash drive. Everything you need to build your own is right here: the launcher scripts, the guide files, the licenses, and the folder structure. Download the AI engine binaries and model files separately (links below), drop them in `.system/`, and you've got the same product we sell.

We charge for the hardware, the testing, and the convenience. The software is free.

## Quick Start

### Buy One (Pre-Built)
Visit [opensourceeverything.io](https://opensourceeverything.io), plug in and go.

### Build Your Own

1. Get a USB flash drive (16GB minimum, 32GB+ recommended)
2. Format it as exFAT (v1.5 retail drives use 128KB allocation units for faster model loads; on macOS: `newfs_exfat -b 131072`)
3. Clone or download this branch onto the drive
4. Copy `ose-logo.txt` into the `.system/` folder (the launchers print it at boot)
5. Download the required binaries into the `.system/` folder (see below)
6. Double-click the launcher for your OS:
   - **Windows:** `WindowsLaunch.bat`
   - **Mac:** `MacLaunch.command` (Apple Silicon and Intel both supported)
   - **Linux:** `Launch facts (Linux)` (double-click) or `LinuxLaunch.sh` (terminal)

### Required Downloads (Not Included — Too Large for Git)

The model files (~7.2 GB total) go in the `.system/` folder:

| File | Size | Source |
|------|------|--------|
| `Qwen3.5-4B-abliterated.Q8_0.gguf` | ~4.5GB | [HuggingFace](https://huggingface.co/mradermacher/Qwen3.5-4B-abliterated-GGUF) |
| `Qwen3.5-4B-abliterated.Q4_K_M.gguf` | ~2.7GB | [HuggingFace](https://huggingface.co/mradermacher/Qwen3.5-4B-abliterated-GGUF) |

Model credits: base model [Qwen3.5-4B](https://huggingface.co/Qwen/Qwen3.5-4B) by Alibaba Cloud (Qwen Team) · abliteration by [wangzhang](https://huggingface.co/wangzhang/Qwen3.5-4B-abliterated) (made with the Abliterix tool) · GGUF quantization by [mradermacher](https://huggingface.co/mradermacher/Qwen3.5-4B-abliterated-GGUF) · Apache 2.0.

Then the platform-specific AI engines from [llama.cpp releases](https://github.com/ggml-org/llama.cpp/releases) (build b8783 or newer):

| Platform | What you need | Where it goes |
|---|---|---|
| **Windows** | `llama-*-bin-win-vulkan-x64.zip` (extract) | `.system/windows/` (needs `llama-server.exe`, `llama-bench.exe`, `ggml-vulkan.dll`, all `ggml-cpu-*.dll`, `llama.dll`) |
| **Mac (Apple Silicon)** | `llama-*-bin-macos-arm64.tar.gz` (extract) | `.system/` (needs `llama-server`, `llama-bench`, all `lib*.dylib`) |
| **Mac (Intel)** | Build llama.cpp from source for x86_64 (CPU) | `.system/x86_64/` (needs `llama-server`; retail drives ship a custom AVX2 build) |
| **Linux** | `llama-*-bin-ubuntu-vulkan-x64.tar.gz` (extract) | `.system/linux/` (needs `llama-server`, `llama-bench`, all `lib*.so`) |

Then the `facts` launcher itself (Rust, source in `facts-rs/`). Until releases are published, build it with [Rust](https://rustup.rs) and copy it into `.system/`:

```
cd facts-rs
cargo build --release
# Windows: copy target\release\facts.exe ..\.system\facts-windows.exe
# Mac:     cp target/release/facts ../.system/facts-macos
# Linux:   cp target/release/facts ../.system/facts-linux
```

## Hardware Requirements

| | Minimum | Recommended |
|---|---------|-------------|
| **RAM** | 8GB | 16GB+ |
| **Windows** | 64-bit Windows 10/11 | Any GPU with a Vulkan driver (NVIDIA / AMD / Intel) |
| **Mac** | Apple Silicon (M1+) or Intel (2013+) | Apple Silicon for full GPU speed; Intel runs CPU mode (slower, fully functional) |
| **Linux** | Any modern x86_64 (glibc 2.32+) | Any GPU with a Vulkan driver (NVIDIA / AMD / Intel) |
| **Drive** | USB 2.0 works | USB 3.0 for faster load times |

## How It Works

1. Plug in the drive
2. Double-click the launcher for your OS
3. The OSE logo prints, ghost processes are killed, and all chat history is wiped (zero-log privacy — history is wiped at every launch AND at exit, and the exit clears the screen)
4. Your context window is sized to your RAM: 8K default, 16K on 16GB machines, 32K on 32GB+
5. The best model for this machine is picked. Any `.gguf` you drop in `.system/` is considered: `facts` keeps the models that fit your memory (RAM + NVIDIA VRAM; on Macs, free memory), benchmarks up to 3 on GPU vs CPU, and runs the biggest one that does at least 5 tokens/sec (Intel Macs: the smallest that does). The first launch on a new computer takes 30-90 seconds per model — it can look frozen, it is not. If every model is too big or too slow, it stops and tells you to add a smaller one. The pick is cached per computer in `.system/.model-choice-<PC>.txt`; delete it to force a re-test.
6. The engine (`llama-server`) loads the model on `127.0.0.1` only, with a fresh access key each launch (10-60 seconds)
7. `>` prompt appears — start asking questions. `/clear` starts over, `/exit` quits

Thinking mode is OFF by default for fast, direct answers. Enabling it (and raising the context size) is documented in `A GUIDE/READ_ME_FIRST.txt` under ADVANCED TWEAKS.

## What's In the Box

```
facts (v1.5)/
├── WindowsLaunch.bat              # Windows launcher (starts .system/facts-windows.exe)
├── MacLaunch.command              # Mac launcher (clears quarantine, starts facts-macos)
├── LinuxLaunch.sh                 # Linux launcher (starts facts-linux)
├── facts-rs/                      # Source of the facts launcher (Rust)
├── Launch facts (Linux)           # Linux launcher (double-click ELF)
├── ose-logo.txt                   # Boot logo — copy into .system/
├── A GUIDE/
│   ├── READ_ME_FIRST.txt          # Product guide, ADVANCED TWEAKS & legal
│   ├── TROUBLESHOOT_WIN.txt       # Windows troubleshooting
│   ├── TROUBLESHOOT_MAC.txt       # Mac troubleshooting
│   └── TROUBLESHOOT_LINUX.txt     # Linux troubleshooting
├── LICENSES/
│   ├── LLAMA_CPP_LICENSE.txt      # MIT License (llama.cpp)
│   └── MODEL LICENSES/
│       └── QWEN_LICENSE.txt       # Apache 2.0 (Qwen)
└── .system/                       # Hidden folder (engines + models)
    ├── facts-windows.exe, facts-macos, facts-linux   # The launcher, one per OS
    ├── llama-server, llama-bench  # Mac Apple Silicon engine (ARM64)
    ├── lib*.dylib                 # Mac shared libraries
    ├── ose-logo.txt               # Boot logo (printed by all launchers)
    ├── x86_64/                    # Intel Mac engine (CPU)
    ├── windows/                   # Windows engine (+ llama-bench.exe)
    ├── linux/                     # Linux engine (+ llama-bench)
    ├── Qwen3.5-4B-abliterated.Q8_0.gguf     # High performance model (~4.5GB)
    └── Qwen3.5-4B-abliterated.Q4_K_M.gguf   # Efficiency model (~2.7GB)
```

## Troubleshooting

Full per-platform guides live in `A GUIDE/`. Quick hits:

| Problem | Fix |
|---------|-----|
| First launch sits at "Testing ..." for a minute | Normal. One-time GPU-vs-CPU benchmark per model; the result is cached and future launches are instant. |
| Want to re-test the GPU after a driver update | Delete `.system/.model-choice-*.txt`, then relaunch. |
| "Every model is too big / too slow for this PC" | Add a smaller model to `.system/`, e.g. a Q4_K_M of a 1-4B model. |
| Terminal opens and closes instantly (Windows) | Antivirus quarantined `facts-windows.exe` or `llama-server.exe`. Windows Security → Protection History → restore + allow. |
| "Can't be opened" on double-click (Mac) | Right-click → Open → click "Open" in the dialog. |
| "Permission denied" (Mac/Linux) | `chmod -R +x` the drive's `.system` folder (see the platform guide). |
| Intel Mac | Fully supported: the launcher auto-detects and runs the CPU engine (slower than Apple Silicon, works fine). |
| Slow performance | Close other apps. The AI wants 4GB+ free RAM. |
| AI crashes mid-conversation | Context window full. Close and relaunch. |

## Tech Stack

| Component | Technology | License |
|-----------|-----------|---------|
| Launcher | `facts` (Rust, `facts-rs/`) | MIT |
| AI Engine (all platforms) | [llama.cpp](https://github.com/ggml-org/llama.cpp) `llama-server` (native binaries) | MIT |
| GPU acceleration | Vulkan (Windows / Linux), Metal (Mac Apple Silicon) | — |
| Model | [Qwen3.5-4B abliterated](https://huggingface.co/mradermacher/Qwen3.5-4B-abliterated-GGUF) (wangzhang abliteration of Qwen3.5-4B) | Apache 2.0 |
| Context Window | 8K / 16K / 32K, sized to your RAM | — |

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for the full version history.

## Support

- **Email:** support@opensourceeverything.io
- **Instagram:** [@open.source.everything](https://instagram.com/open.source.everything)
- **Website:** [opensourceeverything.io](https://opensourceeverything.io)
- **GitHub:** [github.com/WEAREOSE](https://github.com/WEAREOSE)

## License

The launcher scripts and guide files in this repo are released under the [MIT License](LICENSES/LLAMA_CPP_LICENSE.txt).

The AI model (Qwen3.5-4B) is licensed under [Apache 2.0](LICENSES/MODEL%20LICENSES/QWEN_LICENSE.txt).

llama.cpp is licensed under MIT by ggml-org.

---

*Privacy is urgency. It is more possible today than it will be tomorrow.*
