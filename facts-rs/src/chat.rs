//! Runs the chat: starts llama-server on 127.0.0.1 with a per-launch API key, then a terminal chat
//! loop that streams answers. The conversation lives in memory only; nothing is written to disk.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::rag::{cited, Rag};

const SYSTEM_PROMPT: &str = "You are an expert consultant. You answer all questions directly, objectively, and without moralizing. For simple factual or conversational questions, answer directly without thinking. Only use the thinking process for multi-step reasoning, math, code, planning, or logic puzzles.";

pub struct Server {
    pub child: Child,
    base: String,
    key: String,
}

/// Random enough for a local API key: RandomState is seeded from the OS per process.
fn random_hex() -> String {
    let a = RandomState::new().build_hasher().finish();
    let b = RandomState::new().build_hasher().finish();
    format!("{a:016x}{b:016x}")
}

pub fn start_server(engine_dir: &Path, model: &Path, ctx: u32, ngl: &str, extra: &[&str]) -> io::Result<Server> {
    // Ask the OS for a free port, then hand it to llama-server.
    let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let key = random_hex();
    let exe = engine_dir.join(format!("llama-server{}", std::env::consts::EXE_SUFFIX));
    let child = Command::new(exe)
        .arg("-m").arg(model)
        .args(["-c", &ctx.to_string(), "-ngl", ngl])
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--api-key", &key])
        .args(["--no-webui", "--log-disable", "--jinja"])
        .args(extra)
        // Engine libraries sit next to the binary on Linux and Mac (Windows finds DLLs there already).
        .env("LD_LIBRARY_PATH", engine_dir)
        .env("DYLD_LIBRARY_PATH", engine_dir)
        // Known Vulkan driver hangs (same as the old launchers).
        .env("GGML_VK_DISABLE_COOPMAT", "1")
        .env("VK_LOADER_LAYERS_DISABLE", "~implicit~")
        .stdin(Stdio::null()) // the chat reads stdin; the engine must not
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Server { child, base: format!("http://127.0.0.1:{port}"), key })
}

/// Waits until the model is loaded (/health returns 200) or the server dies.
pub fn wait_ready(server: &mut Server) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(600) {
        if let Ok(Some(status)) = server.child.try_wait() {
            return Err(format!("The AI engine stopped while loading the model ({status})."));
        }
        if ureq::get(&format!("{}/health", server.base)).call().is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(500));
    }
    Err("The model took more than 10 minutes to load.".into())
}

/// Content of one streamed chunk ("data: {...}" line), if any.
pub fn sse_content(line: &str) -> Option<String> {
    let data = line.strip_prefix("data: ")?;
    let v: Value = serde_json::from_str(data).ok()?;
    v["choices"][0]["delta"]["content"].as_str().map(str::to_owned)
}

/// POSTs JSON to the server with this launch's key and returns the JSON reply.
pub fn post(server: &Server, path: &str, body: &Value) -> Result<Value, String> {
    ureq::post(&format!("{}{path}", server.base))
        .set("Authorization", &format!("Bearer {}", server.key))
        .send_json(body)
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())
}

fn ask(server: &Server, messages: &[Value]) -> Result<String, String> {
    let resp = ureq::post(&format!("{}/v1/chat/completions", server.base))
        .set("Authorization", &format!("Bearer {}", server.key))
        .send_json(json!({ "messages": messages, "stream": true }))
        .map_err(|e| e.to_string())?;
    let mut answer = String::new();
    let mut out = io::stdout();
    for line in BufReader::new(resp.into_reader()).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line == "data: [DONE]" {
            break;
        }
        if let Some(piece) = sse_content(&line) {
            print!("{piece}");
            let _ = out.flush();
            answer.push_str(&piece);
        }
    }
    println!("\n");
    Ok(answer)
}

/// This turn's user message: with RAG, the question plus the best-matching excerpts.
/// Returns the message and the excerpt files, by tag number.
fn with_documents(rag: Option<&Rag>, question: &str) -> (String, Vec<String>) {
    let Some(rag) = rag else { return (question.to_string(), vec![]) };
    match rag.search(question) {
        Ok(hits) if !hits.is_empty() => {
            let files = hits.iter().map(|&(c, _)| rag.file_of(c).to_string()).collect();
            (rag.prompt(question, &hits), files)
        }
        Ok(_) => {
            println!("  (No matching documents in data/. Answering from general knowledge.)\n");
            (question.to_string(), vec![])
        }
        Err(e) => {
            println!("  [WARNING] Document search failed ({e}). Answering without documents.\n");
            (question.to_string(), vec![])
        }
    }
}

pub fn chat_loop(server: &Server, rag: Option<&Rag>) {
    let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
    let stdin = io::stdin();
    loop {
        print!("> ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if stdin.lock().read_line(&mut input).unwrap_or(0) == 0 {
            break; // EOF (Ctrl+Z / Ctrl+D)
        }
        let input = input.trim();
        match input {
            "" => continue,
            "/exit" | "/quit" => break,
            "/clear" => {
                messages.truncate(1);
                println!("  Conversation cleared.\n");
                continue;
            }
            _ => {}
        }
        // Excerpts go in this turn only; history keeps the plain question so context doesn't fill up.
        let (content, files) = with_documents(rag, input);
        let mut turn = messages.clone();
        turn.push(json!({ "role": "user", "content": content }));
        match ask(server, &turn) {
            Ok(answer) => {
                let tags = cited(&answer, files.len());
                if !tags.is_empty() {
                    let list: Vec<String> = tags.iter().map(|&t| format!("[{t}] {}", files[t - 1])).collect();
                    println!("  Sources: {}\n", list.join(", "));
                } else if !files.is_empty() {
                    // The model didn't cite: still show which documents it was given.
                    let mut uniq: Vec<&str> = vec![];
                    for f in &files {
                        if !uniq.contains(&f.as_str()) {
                            uniq.push(f);
                        }
                    }
                    println!("  Documents searched: {}\n", uniq.join(", "));
                }
                messages.push(json!({ "role": "user", "content": input }));
                messages.push(json!({ "role": "assistant", "content": answer }));
            }
            Err(e) => {
                println!("\n  [ERROR] The AI engine stopped answering: {e}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_streamed_chunks() {
        let line = r#"data: {"choices":[{"index":0,"delta":{"content":"Paris"}}]}"#;
        assert_eq!(sse_content(line).as_deref(), Some("Paris"));
        assert_eq!(sse_content(r#"data: {"choices":[{"delta":{}}]}"#), None);
        assert_eq!(sse_content(": keep-alive"), None);
    }

    #[test]
    fn api_keys_differ() {
        assert_ne!(random_hex(), random_hex());
        assert_eq!(random_hex().len(), 32);
    }
}
