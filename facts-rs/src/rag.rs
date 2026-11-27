//! RAG over the user's documents in <drive>/data: find the embedding model, keep an index of
//! chunks + vectors in .system/rag/index.bin (the one thing facts saves, with consent), and pull
//! the best-matching chunks into each question. Questions, prompts and retrieved chunks stay in
//! memory only.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::chat::{self, Server};
use crate::gguf;

const FORMAT_VERSION: u32 = 1;
const CHUNKER_VERSION: u32 = 1;
const PREFERRED_MODEL: &str = "bge-small-en-v1.5";
pub const MIN_RAM_GB: f64 = 7.0; // "8 GB" machines report ~7.7 GB
const TOP_K: usize = 5;
// Cosine; below this a chunk is treated as unrelated. Tuned on bge-small-en-v1.5 (with its query
// prefix): unrelated questions scored up to 0.48, real matches 0.56 and up.
const MIN_SCORE: f32 = 0.52;
const CHARS_PER_TOKEN: usize = 4; // English estimate; chunks are checked against the real tokenizer
const BATCH: usize = 16;

pub struct EmbedModel {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub ctx: usize,
    pub dim: usize,
}

impl EmbedModel {
    /// GB the embedding server takes from the chat model's budget: weights + small buffers at 512 ctx.
    pub fn need_gb(&self) -> f64 {
        self.bytes as f64 / 1_073_741_824.0 + 0.3
    }
    /// BAAI's English bge models retrieve better when short questions carry this instruction
    /// (documents are embedded as-is). Other models get the question unchanged.
    fn query_prefix(&self) -> &'static str {
        let n = self.name.to_lowercase();
        if n.contains("bge") && n.contains("-en") {
            "Represent this sentence for searching relevant passages: "
        } else {
            ""
        }
    }
    /// Half the input limit (256 tokens for bge-small): with ~400-token chunks a single fact inside a
    /// long, repetitive file scored below unrelated questions in testing; at 256 it scored above them.
    fn target_tokens(&self) -> usize {
        self.ctx / 2
    }
    fn settings(&self) -> String {
        let t = self.target_tokens();
        format!("chunker={CHUNKER_VERSION};target={t};overlap={};cpt={CHARS_PER_TOKEN}", t / 10)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub path: String, // relative to data/, '/' separators so the index works on every OS
    pub size: u64,
    pub mtime: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Chunk {
    pub file: u32,
    pub offset: u32,
    pub text: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Index {
    version: u32,
    model: String,
    model_bytes: u64,
    dim: u32,
    settings: String,
    pub files: Vec<FileEntry>,
    pub chunks: Vec<Chunk>,
    vectors: Vec<f32>, // chunks.len() rows of `dim`
}

impl Index {
    fn new(m: &EmbedModel) -> Index {
        Index {
            version: FORMAT_VERSION,
            model: m.name.clone(),
            model_bytes: m.bytes,
            dim: m.dim as u32,
            settings: m.settings(),
            ..Default::default()
        }
    }
    fn matches(&self, m: &EmbedModel) -> bool {
        self.version == FORMAT_VERSION
            && self.model == m.name
            && self.model_bytes == m.bytes
            && self.dim as usize == m.dim
            && self.settings == m.settings()
    }
    fn vector(&self, i: usize) -> &[f32] {
        let d = self.dim as usize;
        &self.vectors[i * d..(i + 1) * d]
    }
}

pub enum Plan {
    Off(String),
    On { model: EmbedModel, files: Vec<FileEntry> },
}

pub fn data_dir(root: &Path) -> PathBuf {
    root.join("data")
}
fn rag_dir(system_dir: &Path) -> PathBuf {
    system_dir.join("rag")
}
fn index_path(system_dir: &Path) -> PathBuf {
    rag_dir(system_dir).join("index.bin")
}

/// `facts --forget`: deletes the saved index.
pub fn forget(system_dir: &Path) -> io::Result<bool> {
    let dir = rag_dir(system_dir);
    if !dir.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(dir)?;
    Ok(true)
}

/// Every .txt / .md under data/ (recursive, hidden files skipped), sorted by path.
pub fn scan(data: &Path) -> Vec<FileEntry> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<FileEntry>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue; // includes macOS ._ files on exFAT
            }
            let rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(meta) = e.metadata() else { continue };
            let lower = name.to_lowercase();
            if meta.is_dir() {
                walk(&e.path(), &rel, out);
            } else if lower.ends_with(".txt") || lower.ends_with(".md") {
                let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                out.push(FileEntry { path: rel, size: meta.len(), mtime });
            }
        }
    }
    let mut out = vec![];
    walk(data, "", &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The embedding model in .system/embed/: one valid file is used; with several, bge-small-en-v1.5
/// if present, else the smallest. Files without a pooling type aren't embedding models.
pub fn find_model(system_dir: &Path) -> Result<EmbedModel, String> {
    let dir = system_dir.join("embed");
    let mut found = vec![];
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().ends_with(".gguf") || name.starts_with('.') {
            continue;
        }
        let h = match gguf::read_header(&e.path()) {
            Ok(h) => h,
            Err(err) => {
                println!("  [WARNING] Skipping {name} in .system/embed: unreadable ({err}).");
                continue;
            }
        };
        let (Some(_), Some(ctx), Some(dim)) =
            (h.arch_int("pooling_type"), h.arch_int("context_length"), h.arch_int("embedding_length"))
        else {
            println!("  [WARNING] Skipping {name} in .system/embed: not an embedding model.");
            continue;
        };
        let bytes = e.metadata().map_or(0, |m| m.len());
        found.push(EmbedModel { path: e.path(), name, bytes, ctx: ctx as usize, dim: dim as usize });
    }
    if found.is_empty() {
        return Err(format!("no embedding model in .system/embed (add {PREFERRED_MODEL} in GGUF format)"));
    }
    let many = found.len() > 1;
    let pick = match found.iter().position(|m| m.name.to_lowercase().contains(PREFERRED_MODEL)) {
        Some(i) => found.swap_remove(i),
        None => {
            found.sort_by_key(|m| m.bytes);
            found.swap_remove(0)
        }
    };
    if many {
        println!("  Several embedding models found; using {}.", pick.name);
    }
    Ok(pick)
}

fn ask_consent() -> bool {
    println!();
    println!("  facts. will save searchable copies of the files in data/ on this drive.");
    println!("  They stay on this drive and never leave this computer. 'facts --forget' deletes them.");
    print!("  Type yes to continue (anything else: plain chat, nothing saved): ");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    let _ = io::stdin().lock().read_line(&mut answer);
    println!();
    answer.trim().eq_ignore_ascii_case("yes")
}

/// Decides whether RAG runs this launch. Asks for consent before the first index.
pub fn plan(root: &Path, system_dir: &Path, ram_gb: f64) -> Plan {
    let data = data_dir(root);
    let _ = fs::create_dir_all(&data); // so users can see where documents go
    let files = scan(&data);
    if files.is_empty() {
        // Every document was removed: drop the saved copies too.
        let _ = forget(system_dir);
        return Plan::Off("no .txt or .md files in data/".into());
    }
    if ram_gb < MIN_RAM_GB {
        return Plan::Off(format!("needs 8 GB RAM, this PC has {ram_gb:.0} GB"));
    }
    let model = match find_model(system_dir) {
        Ok(m) => m,
        Err(e) => return Plan::Off(e),
    };
    if !index_path(system_dir).exists() && !ask_consent() {
        return Plan::Off("not allowed this launch; you'll be asked again next time".into());
    }
    Plan::On { model, files }
}

/// Byte positions where a chunk may start or end: paragraph and heading starts, plus word breaks
/// inside paragraphs longer than `max_gap` bytes. Includes 0 and text.len().
fn cuts(text: &str, max_gap: usize) -> Vec<usize> {
    let mut paras = vec![0];
    let (mut pos, mut prev_blank) = (0, false);
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if pos > 0 && ((prev_blank && !t.is_empty()) || t.starts_with('#')) {
            paras.push(pos);
        }
        prev_blank = t.is_empty();
        pos += line.len();
    }
    paras.push(text.len());
    paras.dedup();

    let mut out = vec![];
    for w in paras.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        out.push(a);
        while b - a > max_gap {
            let mut limit = a + max_gap;
            while !text.is_char_boundary(limit) {
                limit -= 1;
            }
            let c = match text[a..limit].rfind(|c: char| c.is_ascii_whitespace()) {
                Some(i) if i > 0 => a + i + 1,
                _ if limit > a => limit, // one long word: cut mid-word
                _ => a + text[a..].chars().next().map_or(1, char::len_utf8),
            };
            out.push(c);
            a = c;
        }
    }
    out.push(text.len());
    out.dedup();
    out
}

/// Splits text into chunks of at most `target` bytes, breaking at paragraphs and headings where
/// possible, each chunk repeating about `overlap` bytes of the previous one. Returns (offset, text).
pub fn chunk_text(text: &str, target: usize, overlap: usize) -> Vec<(usize, &str)> {
    let overlap = overlap.min(target / 2);
    let cuts = cuts(text, target - overlap);
    let mut out = vec![];
    let mut start = 0;
    loop {
        let end = *cuts.iter().rev().find(|&&c| c > start && c - start <= target).expect("cuts are at most target apart");
        let raw = &text[start..end];
        let piece = raw.trim();
        if !piece.is_empty() {
            out.push((start + raw.len() - raw.trim_start().len(), piece));
        }
        if end == text.len() {
            return out;
        }
        if text[end..].starts_with('#') {
            start = end; // a new section: no overlap with the one before
            continue;
        }
        let mut from = end.saturating_sub(overlap).max(start + 1);
        while !text.is_char_boundary(from) {
            from += 1;
        }
        start = text[from..end].find(|c: char| c.is_ascii_whitespace()).map_or(from, |i| from + i + 1);
    }
}

/// Splits a chunk in half (at a word) until each part has at most `limit` tokens.
fn fit_tokens(
    offset: usize,
    text: &str,
    limit: usize,
    count: &mut impl FnMut(&str) -> Result<usize, String>,
    out: &mut Vec<(usize, String)>,
) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let mut mid = text.len() / 2;
    while !text.is_char_boundary(mid) {
        mid += 1;
    }
    let mid = text[mid..].find(char::is_whitespace).map_or(mid, |i| mid + i);
    if mid == 0 || mid == text.len() || count(text)? <= limit {
        out.push((offset, text.to_string())); // fits, or a single character
        return Ok(());
    }
    let (a, b) = text.split_at(mid);
    fit_tokens(offset, a.trim_end(), limit, count, out)?;
    let lead = b.len() - b.trim_start().len();
    fit_tokens(offset + mid + lead, b.trim_start(), limit, count, out)
}

/// Unit-length copy (cosine similarity becomes a dot product).
pub fn normalise(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Indexes of the best `k` rows (dot product with `q`) scoring at least `min`, best first.
pub fn top_k(vectors: &[f32], dim: usize, q: &[f32], k: usize, min: f32) -> Vec<(usize, f32)> {
    let mut scored: Vec<(usize, f32)> = vectors
        .chunks_exact(dim)
        .map(|row| row.iter().zip(q).map(|(a, b)| a * b).sum())
        .enumerate()
        .filter(|&(_, s)| s >= min)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(k);
    scored
}

/// Manifest diff: (old file ids to keep, current file positions to (re)index). A file is kept only
/// if path, size and modified time are all unchanged; files missing from `now` are dropped.
pub fn diff(old: &[FileEntry], now: &[FileEntry]) -> (Vec<usize>, Vec<usize>) {
    let mut keep = vec![];
    let mut fresh = vec![];
    for (i, f) in now.iter().enumerate() {
        match old.iter().position(|o| o == f) {
            Some(j) => keep.push(j),
            None => fresh.push(i),
        }
    }
    (keep, fresh)
}

/// Tag numbers like [2] that appear in the answer, in order, 1..=n only.
pub fn cited(answer: &str, n: usize) -> Vec<usize> {
    let mut out = vec![];
    for part in answer.split('[').skip(1) {
        if let Some((num, _)) = part.split_once(']') {
            if let Ok(i) = num.trim().parse::<usize>() {
                if (1..=n).contains(&i) && !out.contains(&i) {
                    out.push(i);
                }
            }
        }
    }
    out
}

fn embed(server: &Server, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let v = chat::post(server, "/v1/embeddings", &json!({ "input": texts, "model": "embed" }))?;
    let data = v["data"].as_array().ok_or("embedding server gave no data")?;
    let mut rows = vec![Vec::new(); texts.len()];
    for (pos, d) in data.iter().enumerate() {
        let i = d["index"].as_u64().map_or(pos, |i| i as usize);
        let mut vec: Vec<f32> = d["embedding"].as_array().ok_or("bad embedding")?.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
        normalise(&mut vec);
        *rows.get_mut(i).ok_or("bad embedding index")? = vec;
    }
    Ok(rows)
}

fn count_tokens(server: &Server, text: &str) -> Result<usize, String> {
    let v = chat::post(server, "/tokenize", &json!({ "content": text }))?;
    v["tokens"].as_array().map(Vec::len).ok_or_else(|| "tokenizer gave no tokens".into())
}

fn load(system_dir: &Path) -> Option<Index> {
    bincode::deserialize(&fs::read(index_path(system_dir)).ok()?).ok()
}

fn save(system_dir: &Path, index: &Index) -> io::Result<()> {
    let dir = rag_dir(system_dir);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("index.bin.tmp");
    fs::write(&tmp, bincode::serialize(index).map_err(io::Error::other)?)?;
    fs::rename(tmp, index_path(system_dir)) // replaces the old index in one step
}

pub struct Rag {
    pub server: Server,
    pub model: EmbedModel,
    pub index: Index,
}

/// Brings the saved index up to date with data/ (only changed files are re-embedded).
pub fn update(system_dir: &Path, root: &Path, model: &EmbedModel, files: Vec<FileEntry>, server: &Server) -> Result<Index, String> {
    let old = load(system_dir).filter(|i| i.matches(model)).unwrap_or_else(|| Index::new(model));
    let (keep, fresh) = diff(&old.files, &files);
    if fresh.is_empty() && keep.len() == old.files.len() {
        return Ok(old);
    }

    let mut index = Index::new(model);
    let dim = model.dim;
    for (new_id, f) in files.iter().enumerate() {
        if let Some(&old_id) = keep.iter().find(|&&j| old.files[j] == *f) {
            for (c, chunk) in old.chunks.iter().enumerate().filter(|(_, c)| c.file as usize == old_id) {
                index.chunks.push(Chunk { file: new_id as u32, ..chunk.clone() });
                index.vectors.extend_from_slice(old.vector(c));
            }
        }
    }

    // Chunk new and changed files, checking each chunk against the model's real input limit.
    let target = model.target_tokens();
    let mut pending: Vec<Chunk> = vec![];
    let limit = model.ctx.saturating_sub(4); // room for the model's start/end tokens
    for &i in &fresh {
        let f = &files[i];
        let path = data_dir(root).join(&f.path);
        let text = match fs::read(&path) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                println!("  [WARNING] Skipping data/{}: {e}", f.path);
                continue;
            }
        };
        let mut parts = vec![];
        for (off, piece) in chunk_text(&text, target * CHARS_PER_TOKEN, target / 10 * CHARS_PER_TOKEN) {
            fit_tokens(off, piece, limit, &mut |t| count_tokens(server, t), &mut parts)?;
        }
        pending.extend(parts.into_iter().map(|(off, text)| Chunk { file: i as u32, offset: off as u32, text }));
    }

    let total = pending.len();
    for (n, batch) in pending.chunks(BATCH).enumerate() {
        print!("\r  Indexing data/: {}/{total} new chunks", (n * BATCH + batch.len()).min(total));
        let _ = io::stdout().flush();
        let texts: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
        for (c, row) in batch.iter().zip(embed(server, &texts)?) {
            if row.len() != dim {
                return Err(format!("embedding has {} values, expected {dim}", row.len()));
            }
            index.chunks.push(c.clone());
            index.vectors.extend(row);
        }
    }
    if total > 0 {
        println!();
    }
    index.files = files;
    if let Err(e) = save(system_dir, &index) {
        println!("  [WARNING] Could not save the RAG index ({e}); it will be rebuilt next launch.");
    }
    Ok(index)
}

impl Rag {
    pub fn status(&self) -> String {
        let files = self.index.chunks.iter().map(|c| c.file).collect::<std::collections::HashSet<_>>().len();
        format!("RAG index: {} chunks from {files} files (stored on this drive)", self.index.chunks.len())
    }

    /// Best chunks for a question: (chunk index, score).
    pub fn search(&self, question: &str) -> Result<Vec<(usize, f32)>, String> {
        // Keep the question within the model's input limit.
        let mut end = question.len().min(self.model.target_tokens() * CHARS_PER_TOKEN);
        while !question.is_char_boundary(end) {
            end -= 1;
        }
        let q = embed(&self.server, &[format!("{}{}", self.model.query_prefix(), &question[..end])])?.pop().unwrap_or_default();
        Ok(top_k(&self.index.vectors, self.model.dim, &q, TOP_K, MIN_SCORE))
    }

    pub fn file_of(&self, chunk: usize) -> &str {
        &self.index.files[self.index.chunks[chunk].file as usize].path
    }

    /// The user message for this turn: tagged excerpts, then the question.
    pub fn prompt(&self, question: &str, hits: &[(usize, f32)]) -> String {
        let mut s = String::from(
            "Answer the question using the excerpts from the user's documents below. Cite the excerpts you use by their tags, like [1], at the end of each sentence that uses them. If the excerpts don't contain the answer, say so, then answer from general knowledge.\n\n",
        );
        for (n, &(c, _)) in hits.iter().enumerate() {
            s.push_str(&format!("[{}] {}\n{}\n\n", n + 1, self.file_of(c), self.index.chunks[c].text));
        }
        s.push_str(&format!("Question: {question}"));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize) -> String {
        (0..n).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(chunk_text("  hello world \n", 100, 10), vec![(2, "hello world")]);
        assert!(chunk_text("   \n\n ", 100, 10).is_empty());
    }

    #[test]
    fn chunks_fit_overlap_and_cover_everything() {
        let text = format!("# Intro\n{}\n\n{}\n\n# Next\n{}\n", words(80), words(300), words(40));
        let chunks = chunk_text(&text, 400, 40);
        assert!(chunks.len() > 3);
        for (off, c) in &chunks {
            assert!(c.len() <= 400, "{} bytes", c.len());
            assert_eq!(&text[*off..off + c.len()], *c);
        }
        for pair in chunks.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(b.0 > a.0, "chunks move forward");
            let a_end = a.0 + a.1.len();
            // Overlap, except where a chunk ends on a paragraph/heading break.
            assert!(b.0 < a_end || text[a_end..b.0].trim().is_empty());
        }
        for w in text.split_whitespace() {
            assert!(chunks.iter().any(|(_, c)| c.split_whitespace().any(|x| x == w)), "{w} lost");
        }
        assert!(chunks.iter().any(|(_, c)| c.starts_with("# Next")), "heading starts a chunk");
    }

    #[test]
    fn long_word_and_unicode_split_safely() {
        let text = "Ã©".repeat(300) + " " + &"x".repeat(500);
        let chunks = chunk_text(&text, 101, 10);
        assert!(chunks.iter().all(|(_, c)| c.len() <= 101 && !c.is_empty()));
        assert_eq!(chunks.iter().map(|(_, c)| c.matches('x').count()).max(), Some(101));
    }

    #[test]
    fn oversized_chunks_split_by_token_count() {
        let text = words(100);
        let mut out = vec![];
        let mut count = |t: &str| Ok(t.split_whitespace().count());
        fit_tokens(7, &text, 30, &mut count, &mut out).unwrap();
        assert!(out.len() >= 4);
        assert!(out.iter().all(|(_, t)| t.split_whitespace().count() <= 30));
        assert_eq!(out.iter().map(|(_, t)| t.split_whitespace().count()).sum::<usize>(), 100);
        assert_eq!(&text[out[1].0 - 7..out[1].0 - 7 + out[1].1.len()], out[1].1);
    }

    #[test]
    fn scoring_picks_best_above_minimum() {
        let mut rows = vec![1.0, 0.0, 0.6, 0.8, 0.0, 1.0, -1.0, 0.0];
        rows.as_chunks_mut::<2>().0.iter_mut().for_each(|r| normalise(r));
        let hits = top_k(&rows, 2, &[1.0, 0.0], 2, 0.5);
        assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), vec![0, 1]);
        assert!((hits[1].1 - 0.6).abs() < 1e-6);
        assert!(top_k(&rows, 2, &[0.0, -1.0], 5, 0.5).is_empty());
    }

    #[test]
    fn manifest_diff() {
        let f = |p: &str, size, mtime| FileEntry { path: p.into(), size, mtime };
        let old = [f("a.md", 10, 1), f("b.txt", 20, 2), f("gone.md", 5, 5)];
        let now = [f("a.md", 10, 1), f("b.txt", 21, 3), f("new/c.md", 7, 7)];
        assert_eq!(diff(&old, &now), (vec![0], vec![1, 2]));
        assert_eq!(diff(&[], &now), (vec![], vec![0, 1, 2]));
    }

    #[test]
    fn finds_cited_tags() {
        assert_eq!(cited("Paris [2] is big [1][2]. See [7] and [x].", 5), vec![2, 1]);
        assert!(cited("no tags", 5).is_empty());
    }

    #[test]
    fn index_round_trips_and_detects_model_change() {
        let m = EmbedModel { path: "x".into(), name: "bge.gguf".into(), bytes: 9, ctx: 512, dim: 2 };
        let mut i = Index::new(&m);
        i.vectors = vec![0.5, 0.5];
        let back: Index = bincode::deserialize(&bincode::serialize(&i).unwrap()).unwrap();
        assert!(back.matches(&m) && back.vector(0) == [0.5, 0.5]);
        assert!(!back.matches(&EmbedModel { bytes: 10, ..m }));
    }

    #[test]
    fn rejects_files_that_are_not_embedding_models() {
        let dir = std::env::temp_dir().join(format!("facts-test-{}", std::process::id()));
        let embed = dir.join("embed");
        fs::create_dir_all(&embed).unwrap();
        fs::write(embed.join("junk.gguf"), b"not a model").unwrap();
        assert!(find_model(&dir).is_err());
        fs::write(embed.join("bge-small-en-v1.5-q8_0.gguf"), crate::gguf::tests::fake_embed_header()).unwrap();
        let m = find_model(&dir).unwrap();
        assert_eq!((m.ctx, m.dim), (512, 384));
        let _ = fs::remove_dir_all(dir);
    }
}
