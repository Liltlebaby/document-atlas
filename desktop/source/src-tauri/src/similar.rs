use super::*;
use std::collections::HashMap;

// Document-level similarity on top of the RAG chunk vectors (rag_vec).
//
// forward(A→B)  = mean over A's chunks of the best cosine similarity to any chunk of B
//                 ("how much of A is already said somewhere in B")
// backward(B→A) = the same measured from B's side
// Chunks that closely match many documents (page headers, the document-control notice printed on
// every page) say nothing about content and are left out, unless a document has nothing else.

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
#[serde(default)]
pub(crate) struct Thresholds { pub duplicate: f32, pub contained: f32, pub related: f32, pub common: f32 }
impl Default for Thresholds {
    fn default() -> Self { Self { duplicate: 0.93, contained: 0.85, related: 0.78, common: 0.90 } }
}
const TOP_K: usize = 3;
const EVIDENCE: usize = 2;
const SNIPPET: usize = 420;

pub(crate) fn init_similar_db(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS iso_similar (
        node_id TEXT NOT NULL, other_id TEXT NOT NULL, rank INTEGER NOT NULL, forward REAL NOT NULL, backward REAL NOT NULL,
        evidence TEXT NOT NULL DEFAULT '[]', computed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, PRIMARY KEY(node_id,other_id));").map_err(err)
}
pub(crate) fn indexed_ids(db: &Connection) -> Result<HashSet<String>> {
    let mut stmt = db.prepare("SELECT node_id FROM rag_documents WHERE status IN ('ready','partial_text')").map_err(err)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
    rows.collect::<std::result::Result<HashSet<_>, _>>().map_err(err)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Evidence { score: f32, source_page: i64, source_text: String, other_page: i64, other_text: String }
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SimilarDoc { node_id: String, name: String, path: String, rank: i64, forward: f32, backward: f32, verdict: String, evidence: Vec<Evidence> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SimilarityRun { targets: usize, indexed_targets: usize, pool_documents: usize, pool_chunks: usize, pairs: usize, seconds: f64 }

pub(crate) fn verdict(forward: f32, backward: f32, th: &Thresholds) -> &'static str {
    if forward >= th.duplicate && backward >= th.duplicate { "duplicate" }
    else if forward >= th.contained { "contained" }
    else if backward >= th.contained { "contains" }
    else if forward.max(backward) >= th.related { "related" }
    else { "different" }
}

struct Pool { docs: Vec<String>, index: HashMap<String, usize>, doc_chunks: Vec<Vec<usize>>, chunk_doc: Vec<usize>, chunk_ids: Vec<i64>, vectors: Vec<f32>, dim: usize }

fn normalised(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 { v.iter_mut().for_each(|x| *x /= n); }
    v
}
fn from_blob(blob: &[u8]) -> Vec<f32> { normalised(blob.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()) }
fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Eight independent accumulators let the compiler vectorise the loop.
    let mut acc = [0f32; 8];
    let (ha, ta) = a.split_at(a.len() / 8 * 8);
    let (hb, tb) = b.split_at(ha.len());
    for (x, y) in ha.chunks_exact(8).zip(hb.chunks_exact(8)) { for k in 0..8 { acc[k] += x[k] * y[k]; } }
    acc.iter().sum::<f32>() + ta.iter().zip(tb).map(|(x, y)| x * y).sum::<f32>()
}

impl Pool {
    fn load(db: &Connection, folders: &[String]) -> Result<Pool> {
        if db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='rag_vec'", [], |r| r.get::<_, i64>(0)).map_err(err)? == 0 {
            return Err("ยังไม่มีดัชนี RAG กรุณาทำดัชนีเอกสารก่อน".into());
        }
        let mut pool = Pool { docs: Vec::new(), index: HashMap::new(), doc_chunks: Vec::new(), chunk_doc: Vec::new(), chunk_ids: Vec::new(), vectors: Vec::new(), dim: 0 };
        let mut seen = HashSet::new();
        let mut stmt = db.prepare("SELECT c.id,c.node_id,v.embedding FROM rag_chunks c JOIN rag_vec v ON v.rowid=c.id JOIN nodes n ON n.id=c.node_id
            WHERE n.active=1 AND n.path LIKE ?1 ESCAPE '\\' ORDER BY c.node_id,c.id").map_err(err)?;
        for folder in folders {
            let rows = stmt.query_map([iso::like_prefix(folder)], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?))).map_err(err)?;
            for row in rows {
                let (id, node_id, blob) = row.map_err(err)?;
                if !seen.insert(id) { continue; }
                pool.push(id, node_id, from_blob(&blob))?;
            }
        }
        Ok(pool)
    }
    fn push(&mut self, chunk_id: i64, node_id: String, vector: Vec<f32>) -> Result<()> {
        if self.dim == 0 { self.dim = vector.len(); }
        if vector.len() != self.dim || self.dim == 0 { return Err("มิติเวกเตอร์ในดัชนีไม่ตรงกัน กรุณาทำดัชนีใหม่ด้วยโมเดลเดียวกัน".into()); }
        let doc = *self.index.entry(node_id.clone()).or_insert_with(|| { self.docs.push(node_id); self.doc_chunks.push(Vec::new()); self.docs.len() - 1 });
        self.doc_chunks[doc].push(self.chunk_ids.len());
        self.chunk_doc.push(doc);
        self.chunk_ids.push(chunk_id);
        self.vectors.extend(vector);
        Ok(())
    }
    fn vector(&self, chunk: usize) -> &[f32] { &self.vectors[chunk * self.dim..(chunk + 1) * self.dim] }
    /// Best similarity of `q` to each document, with the chunk that achieved it.
    fn best_per_doc(&self, q: &[f32]) -> Vec<(f32, usize)> {
        let mut best = vec![(f32::MIN, usize::MAX); self.docs.len()];
        for (chunk, &doc) in self.chunk_doc.iter().enumerate() {
            let s = dot(q, self.vector(chunk));
            if s > best[doc].0 { best[doc] = (s, chunk); }
        }
        best
    }
    /// A chunk is boilerplate when it nearly matches more than this many other documents.
    fn common_limit(&self) -> usize { (self.docs.len() / 10).max(5) }
    fn is_common(&self, best: &[(f32, usize)], own: Option<usize>, th: &Thresholds) -> bool {
        best.iter().enumerate().filter(|(d, (s, _))| Some(*d) != own && *s >= th.common).count() > self.common_limit()
    }
}

struct Ranked { doc: usize, forward: f32, backward: f32, evidence: Vec<(usize, usize, f32)> } // (query index, pool chunk, score)

/// Top-k pool documents covering `queries`. `own` is the pool index of the source document, if it is in the pool.
fn rank(pool: &Pool, queries: &[&[f32]], own: Option<usize>, th: &Thresholds) -> Vec<Ranked> {
    let bests: Vec<Vec<(f32, usize)>> = queries.iter().map(|q| pool.best_per_doc(q)).collect();
    let common: Vec<bool> = bests.iter().map(|b| pool.is_common(b, own, th)).collect();
    let used: Vec<usize> = if common.iter().all(|c| *c) { (0..queries.len()).collect() } else { (0..queries.len()).filter(|&i| !common[i]).collect() };
    if used.is_empty() { return Vec::new(); }
    let mut ranked: Vec<Ranked> = (0..pool.docs.len()).filter(|&d| Some(d) != own).map(|d| Ranked {
        doc: d, forward: used.iter().map(|&i| bests[i][d].0).sum::<f32>() / used.len() as f32, backward: 0.0, evidence: Vec::new(),
    }).collect();
    ranked.sort_by(|a, b| b.forward.total_cmp(&a.forward));
    ranked.truncate(TOP_K);
    for r in &mut ranked {
        let mut ev: Vec<(usize, usize, f32)> = used.iter().map(|&i| (i, bests[i][r.doc].1, bests[i][r.doc].0)).collect();
        ev.sort_by(|a, b| b.2.total_cmp(&a.2));
        ev.truncate(EVIDENCE);
        r.evidence = ev;
        r.backward = backward(pool, r.doc, queries, th);
    }
    ranked
}
/// How much of pool document `other` is covered by `targets` (boilerplate chunks of `other` ignored).
fn backward(pool: &Pool, other: usize, targets: &[&[f32]], th: &Thresholds) -> f32 {
    let mut kept = Vec::new();
    let mut all = Vec::new();
    for &chunk in &pool.doc_chunks[other] {
        let q = pool.vector(chunk);
        let score = targets.iter().map(|t| dot(q, t)).fold(f32::MIN, f32::max);
        all.push(score);
        if !pool.is_common(&pool.best_per_doc(q), Some(other), th) { kept.push(score); }
    }
    let scores = if kept.is_empty() { all } else { kept };
    if scores.is_empty() { 0.0 } else { scores.iter().sum::<f32>() / scores.len() as f32 }
}

/// Top-k similar documents for each target that is in the pool, computed on all CPU cores.
fn compute(pool: &Pool, targets: &[String], th: &Thresholds) -> Vec<(String, Vec<Ranked>)> {
    let indexed: Vec<(String, usize)> = targets.iter().filter_map(|t| pool.index.get(t).map(|&d| (t.clone(), d))).collect();
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8);
    let size = indexed.len().div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = indexed.chunks(size).map(|part| scope.spawn(move || part.iter().map(|(id, d)| {
            let queries: Vec<&[f32]> = pool.doc_chunks[*d].iter().map(|&c| pool.vector(c)).collect();
            (id.clone(), rank(pool, &queries, Some(*d), th))
        }).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

fn save(db: &mut Connection, pool: &Pool, results: &[(String, Vec<Ranked>)]) -> Result<usize> {
    let tx = db.transaction().map_err(err)?;
    let mut pairs = 0;
    for (node_id, ranked) in results {
        tx.execute("DELETE FROM iso_similar WHERE node_id=?1", [node_id]).map_err(err)?;
        let own = pool.index[node_id];
        for (rank, r) in ranked.iter().enumerate() {
            let evidence: Vec<(i64, i64, f32)> = r.evidence.iter().map(|(q, chunk, s)| (pool.chunk_ids[pool.doc_chunks[own][*q]], pool.chunk_ids[*chunk], *s)).collect();
            tx.execute("INSERT INTO iso_similar(node_id,other_id,rank,forward,backward,evidence,computed_at) VALUES(?1,?2,?3,?4,?5,?6,CURRENT_TIMESTAMP)",
                params![node_id, pool.docs[r.doc], rank as i64 + 1, r.forward as f64, r.backward as f64, serde_json::to_string(&evidence).map_err(err)?]).map_err(err)?;
            pairs += 1;
        }
    }
    tx.commit().map_err(err)?;
    Ok(pairs)
}

fn chunk_text(db: &Connection, id: i64) -> Result<(i64, String)> {
    db.query_row("SELECT page,text FROM rag_chunks WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get::<_, String>(1)?))).map_err(err)
}
fn snippet(text: &str) -> String { let s: String = text.chars().take(SNIPPET).collect(); if text.chars().count() > SNIPPET { format!("{s}…") } else { s } }
fn node_label(db: &Connection, id: &str) -> Result<(String, String)> { db.query_row("SELECT name,path FROM nodes WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err) }

fn similar_for(db: &Connection, node_id: &str, th: &Thresholds) -> Result<Vec<SimilarDoc>> {
    let mut stmt = db.prepare("SELECT s.other_id,n.name,n.path,s.rank,s.forward,s.backward,s.evidence FROM iso_similar s JOIN nodes n ON n.id=s.other_id WHERE s.node_id=?1 ORDER BY s.rank").map_err(err)?;
    let rows = stmt.query_map([node_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, f64>(4)?, r.get::<_, f64>(5)?, r.get::<_, String>(6)?))).map_err(err)?;
    let mut out = Vec::new();
    for row in rows {
        let (other, name, path, rank, forward, backward, evidence) = row.map_err(err)?;
        let mut items = Vec::new();
        for (source, target, score) in serde_json::from_str::<Vec<(i64, i64, f32)>>(&evidence).unwrap_or_default() {
            let (Ok((sp, st)), Ok((op, ot))) = (chunk_text(db, source), chunk_text(db, target)) else { continue };
            items.push(Evidence { score, source_page: sp, source_text: snippet(&st), other_page: op, other_text: snippet(&ot) });
        }
        let (forward, backward) = (forward as f32, backward as f32);
        out.push(SimilarDoc { node_id: other, name, path, rank, forward, backward, verdict: verdict(forward, backward, th).into(), evidence: items });
    }
    Ok(out)
}

fn group_nodes(db: &Connection, cfg: &iso::IsoConfig, group: i64) -> Result<Vec<Node>> {
    let mut seen = HashSet::new();
    let mut nodes = Vec::new();
    for d in cfg.departments.iter().filter(|d| group == 0 || d.group == group) {
        for n in iso::department_nodes(db, d)? { if seen.insert(n.id.clone()) { nodes.push(n); } }
    }
    Ok(nodes)
}

#[tauri::command]
pub(crate) fn iso_start_index(app: AppHandle, state: State<'_, AppState>, group: i64) -> Result<rag::RagJob> {
    let items = {
        let db = state.db.lock().map_err(err)?;
        let cfg = iso::config(&db)?.ok_or("กรุณานำเข้าไฟล์ตั้งค่าแผนกก่อน")?;
        let mut stmt = db.prepare("SELECT node_id FROM rag_documents WHERE status IN ('ready','partial_text','needs_ocr')").map_err(err)?;
        let done = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?.collect::<std::result::Result<HashSet<_>, _>>().map_err(err)?;
        group_nodes(&db, &cfg, group)?.into_iter().filter(|n| !done.contains(&n.id)).collect::<Vec<_>>()
    };
    if items.is_empty() { return Err("เอกสารในขอบเขตนี้ทำดัชนีครบแล้ว".into()); }
    rag::launch_index(app, &state, items)
}

#[tauri::command]
pub(crate) async fn iso_compute_similarity(state: State<'_, AppState>, group: i64) -> Result<SimilarityRun> {
    let started = Instant::now();
    let (pool, targets, th) = {
        let db = state.db.lock().map_err(err)?;
        let cfg = iso::config(&db)?.ok_or("กรุณานำเข้าไฟล์ตั้งค่าแผนกก่อน")?;
        let targets: Vec<String> = group_nodes(&db, &cfg, group)?.into_iter().map(|n| n.id).collect();
        (Pool::load(&db, &cfg.similarity_folders())?, targets, cfg.similarity)
    };
    if pool.docs.len() < 2 { return Err("ดัชนี RAG มีเอกสารน้อยเกินไป กรุณาทำดัชนีเอกสารในขอบเขตเปรียบเทียบก่อน".into()); }
    let (pool, results) = tauri::async_runtime::spawn_blocking(move || { let results = compute(&pool, &targets, &th); (pool, results) }).await.map_err(err)?;
    let pairs = { let mut db = state.db.lock().map_err(err)?; save(&mut db, &pool, &results)? };
    let targets = { let db = state.db.lock().map_err(err)?; let cfg = iso::config(&db)?.ok_or("ไม่พบไฟล์ตั้งค่าแผนก")?; group_nodes(&db, &cfg, group)?.len() };
    Ok(SimilarityRun { targets, indexed_targets: results.len(), pool_documents: pool.docs.len(), pool_chunks: pool.chunk_ids.len(), pairs, seconds: started.elapsed().as_secs_f64() })
}

#[tauri::command]
pub(crate) fn iso_similar(state: State<'_, AppState>, node_id: String) -> Result<Vec<SimilarDoc>> {
    let db = state.db.lock().map_err(err)?;
    let th = iso::config(&db)?.map(|c| c.similarity).unwrap_or_default();
    similar_for(&db, &node_id, &th)
}

fn read_pdf(path: &str) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).map_err(err)?;
    if meta.len() > 100 * 1024 * 1024 { return Err("ไฟล์ใหญ่เกิน 100 MB".into()); }
    let bytes = fs::read(path).map_err(err)?;
    if !bytes.starts_with(b"%PDF-") { return Err("ไฟล์ที่เลือกไม่ใช่ PDF".into()); }
    Ok(bytes)
}

/// Compare a PDF that is not on the Portal yet (a new or revised document) against the indexed documents.
#[tauri::command]
pub(crate) async fn iso_check_new_document(state: State<'_, AppState>, path: String) -> Result<Vec<SimilarDoc>> {
    let bytes = read_pdf(&path)?;
    let (engine_config, folders, th) = {
        let db = state.db.lock().map_err(err)?;
        let cfg = iso::config(&db)?.ok_or("กรุณานำเข้าไฟล์ตั้งค่าแผนกก่อน")?;
        (rag::selected_engine(&db)?, cfg.similarity_folders(), cfg.similarity)
    };
    let (_, _, chunks) = tauri::async_runtime::spawn_blocking(move || rag::extract_pdf(&bytes)).await.map_err(err)??;
    if chunks.is_empty() { return Err("PDF นี้ไม่มีข้อความให้เทียบ (อาจเป็นภาพสแกน) ต้องให้ AI อ่านภาพแทน".into()); }
    let is_e5 = engine_config.e5();
    let inputs: Vec<String> = chunks.iter().map(|(_, t)| if is_e5 { format!("passage: {t}") } else { t.clone() }).collect();
    let mut engine = rag::start_engine(engine_config).await?;
    let vectors: Vec<Vec<f32>> = engine.embed(&inputs).await?.into_iter().map(normalised).collect();
    let pool = { let db = state.db.lock().map_err(err)?; Pool::load(&db, &folders)? };
    if pool.docs.is_empty() { return Err("ยังไม่มีเอกสารในดัชนี RAG ให้เทียบ".into()); }
    let ranked = tauri::async_runtime::spawn_blocking(move || {
        let queries: Vec<&[f32]> = vectors.iter().map(|v| v.as_slice()).collect();
        let ranked = rank(&pool, &queries, None, &th);
        ranked.into_iter().map(|r| (pool.docs[r.doc].clone(), r.forward, r.backward, r.evidence.into_iter().map(|(q, c, s)| (q, pool.chunk_ids[c], s)).collect::<Vec<_>>())).collect::<Vec<_>>()
    }).await.map_err(err)?;
    let db = state.db.lock().map_err(err)?;
    let mut out = Vec::new();
    for (rank, (node_id, forward, backward, evidence)) in ranked.into_iter().enumerate() {
        let (name, path) = node_label(&db, &node_id)?;
        let items = evidence.into_iter().filter_map(|(q, other, score)| {
            let (op, ot) = chunk_text(&db, other).ok()?;
            let (sp, st) = &chunks[q];
            Some(Evidence { score, source_page: *sp, source_text: snippet(st), other_page: op, other_text: snippet(&ot) })
        }).collect();
        out.push(SimilarDoc { node_id, name, path, rank: rank as i64 + 1, forward, backward, verdict: verdict(forward, backward, &th).into(), evidence: items });
    }
    Ok(out)
}

/// Write a comparison package (new document text + full text of the matched documents) for the AI reviewer.
#[tauri::command]
pub(crate) fn iso_write_case(state: State<'_, AppState>, dir: String, pdf_path: String, matches: Vec<SimilarDoc>) -> Result<String> {
    let dir = PathBuf::from(dir.trim());
    if !dir.is_dir() { return Err("กรุณาเลือกโฟลเดอร์ที่มีอยู่จริง".into()); }
    let (pages, _, chunks) = rag::extract_pdf(&read_pdf(&pdf_path)?)?;
    let db = state.db.lock().map_err(err)?;
    let mut candidates = Vec::new();
    for m in &matches {
        let mut stmt = db.prepare("SELECT page,text FROM rag_chunks WHERE node_id=?1 ORDER BY page,id").map_err(err)?;
        let text = stmt.query_map([&m.node_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
        candidates.push(serde_json::json!({ "node_id": m.node_id, "name": m.name, "path": m.path, "rank": m.rank, "forward": m.forward, "backward": m.backward,
            "verdict": m.verdict, "chunks": text.into_iter().map(|(p, t)| serde_json::json!({"page": p, "text": t})).collect::<Vec<_>>() }));
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    let stem = Path::new(&pdf_path).file_stem().map(|s| s.to_string_lossy().chars().filter(|c| !"<>:\"/\\|?*".contains(*c)).take(60).collect::<String>()).unwrap_or_default();
    let target = dir.join(format!("case-{stamp}-{stem}.json"));
    let payload = serde_json::json!({
        "kind": "iso-new-document-check",
        "question": "เอกสารใหม่นี้ควร (1) ออกเป็นฉบับใหม่ (2) แก้ Revision ฉบับเดิม (3) รวมเข้ากับฉบับเดิม หรือ (4) ซ้ำ ไม่ต้องออก เทียบกับเอกสารที่คล้ายที่สุด",
        "new_document": { "file": pdf_path, "pages": pages, "chunks": chunks.into_iter().map(|(p, t)| serde_json::json!({"page": p, "text": t})).collect::<Vec<_>>() },
        "candidates": candidates,
    });
    fs::write(&target, serde_json::to_string_pretty(&payload).map_err(err)?).map_err(err)?;
    Ok(target.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(docs: &[(&str, &[[f32; 3]])]) -> Pool {
        let mut p = Pool { docs: Vec::new(), index: HashMap::new(), doc_chunks: Vec::new(), chunk_doc: Vec::new(), chunk_ids: Vec::new(), vectors: Vec::new(), dim: 0 };
        let mut id = 0;
        for (name, chunks) in docs { for c in *chunks { id += 1; p.push(id, name.to_string(), normalised(c.to_vec())).unwrap(); } }
        p
    }

    #[test]
    fn identical_documents_score_as_duplicates_and_subsets_as_contained() {
        let th = Thresholds::default();
        let p = pool(&[("a", &[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]), ("b", &[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]), ("small", &[[1.0, 0.02, 0.0]]), ("other", &[[0.0, 0.0, 1.0]])]);
        let a = p.index["a"];
        let queries: Vec<&[f32]> = p.doc_chunks[a].iter().map(|&c| p.vector(c)).collect();
        let ranked = rank(&p, &queries, Some(a), &th);
        assert_eq!(p.docs[ranked[0].doc], "b");
        assert_eq!(verdict(ranked[0].forward, ranked[0].backward, &th), "duplicate");
        let small = ranked.iter().find(|r| p.docs[r.doc] == "small").unwrap();
        assert_eq!(verdict(small.forward, small.backward, &th), "contains", "everything in 'small' is already in 'a'");
        assert!(ranked.iter().all(|r| p.docs[r.doc] != "a"), "a document is never similar to itself");
    }

    #[test]
    fn boilerplate_shared_by_many_documents_is_ignored() {
        let th = Thresholds::default();
        let header = [0.0, 0.0, 1.0];
        let mut docs: Vec<(String, Vec<[f32; 3]>)> = (0..12).map(|i| (format!("filler{i}"), vec![header, [0.3, -1.0, 0.0]])).collect();
        docs.push(("a".into(), vec![header, [1.0, 0.0, 0.0]]));
        docs.push(("b".into(), vec![header, [0.98, 0.2, 0.0]]));
        let refs: Vec<(&str, &[[f32; 3]])> = docs.iter().map(|(n, c)| (n.as_str(), c.as_slice())).collect();
        let p = pool(&refs);
        let a = p.index["a"];
        let queries: Vec<&[f32]> = p.doc_chunks[a].iter().map(|&c| p.vector(c)).collect();
        let ranked = rank(&p, &queries, Some(a), &th);
        assert_eq!(p.docs[ranked[0].doc], "b", "content chunk decides, not the shared header");
        assert!(ranked[0].forward > 0.9 && ranked[1].forward < 0.5);
        assert_eq!(ranked[0].evidence.len(), 1, "the header chunk is not offered as evidence");
    }

    #[test]
    fn dot_matches_naive_sum_for_odd_lengths() {
        let a: Vec<f32> = (0..1027).map(|i| (i as f32 * 0.37).sin()).collect();
        let b: Vec<f32> = (0..1027).map(|i| (i as f32 * 0.11).cos()).collect();
        let naive: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((dot(&a, &b) - naive).abs() < 1e-3);
    }

    #[test]
    fn stored_pairs_round_trip_with_evidence_text() {
        let mut db = Connection::open_in_memory().unwrap();
        init_db(&db).unwrap();
        for (id, name) in [("file:1", "A"), ("file:2", "B")] {
            db.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES(?1,?2,'file',?3,?4)", params![id, &id[5..], name, format!("Portal/VSX/{name}")]).unwrap();
        }
        db.execute("INSERT INTO rag_chunks(id,node_id,page,text) VALUES(1,'file:1',1,'ขั้นตอนการตรวจรับวัตถุดิบ'),(2,'file:2',3,'การตรวจรับวัตถุดิบยางแผ่น')", []).unwrap();
        let p = pool(&[("file:1", &[[1.0, 0.0, 0.0]]), ("file:2", &[[1.0, 0.0, 0.0]])]);
        let results = compute(&p, &["file:1".to_string(), "file:404".to_string()], &Thresholds::default());
        assert_eq!(results.len(), 1, "targets without vectors are skipped");
        assert_eq!(save(&mut db, &p, &results).unwrap(), 1);
        let hits = similar_for(&db, "file:1", &Thresholds::default()).unwrap();
        assert_eq!(hits[0].node_id, "file:2");
        assert_eq!(hits[0].verdict, "duplicate");
        assert_eq!(hits[0].evidence[0].other_page, 3);
        assert_eq!(hits[0].evidence[0].source_text, "ขั้นตอนการตรวจรับวัตถุดิบ");
    }
}
