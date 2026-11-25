use super::*;
use std::collections::{BTreeMap, HashMap};

// ISO document review: departments come from iso-review/config/departments.json, proposals
// (ACTIVE / ARCHIVE / DUPLICATE / REVIEW) come from the review pipeline, decisions are made here.
// Nothing in this module changes or deletes files on the Portal or on disk.

const STATUSES: [&str; 4] = ["ACTIVE", "ARCHIVE", "DUPLICATE", "REVIEW"];
const DECISIONS: [&str; 3] = ["pending", "approved", "rejected"];

pub fn init_iso_db(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS iso_reviews (
        node_id TEXT PRIMARY KEY, department TEXT NOT NULL DEFAULT '', group_no INTEGER NOT NULL DEFAULT 0,
        code TEXT NOT NULL DEFAULT '', doc_type TEXT NOT NULL DEFAULT '', revision TEXT NOT NULL DEFAULT '', title TEXT NOT NULL DEFAULT '',
        status TEXT NOT NULL, reason TEXT NOT NULL DEFAULT '', related TEXT NOT NULL DEFAULT '', cluster TEXT NOT NULL DEFAULT '',
        merge_into TEXT NOT NULL DEFAULT '', iso_clauses TEXT NOT NULL DEFAULT '[]', source TEXT NOT NULL DEFAULT 'rule',
        decision TEXT NOT NULL DEFAULT 'pending', decision_note TEXT NOT NULL DEFAULT '', decided_by TEXT NOT NULL DEFAULT '',
        decided_at TEXT NOT NULL DEFAULT '', imported_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
    CREATE INDEX IF NOT EXISTS idx_iso_reviews_department ON iso_reviews(department);").map_err(err)?;
    // Added after the first ISO release: documents named in the ISO clause table that must never be merged away.
    let has_protected = conn.prepare("SELECT 1 FROM pragma_table_info('iso_reviews') WHERE name='protected'").map_err(err)?.exists([]).map_err(err)?;
    if !has_protected { conn.execute("ALTER TABLE iso_reviews ADD COLUMN protected INTEGER NOT NULL DEFAULT 0", []).map_err(err)?; }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct IsoConfig {
    branch: String, #[serde(default = "default_ratio")] target_ratio: f64, groups: BTreeMap<String, IsoGroupConfig>, pub(crate) departments: Vec<IsoDepartmentConfig>,
    /// Folders whose indexed documents are compared against (empty = all department folders).
    #[serde(default)] similarity_scope: Vec<String>,
    #[serde(default)] pub(crate) similarity: similar::Thresholds,
}
#[derive(Clone, Serialize, Deserialize)]
struct IsoGroupConfig { label: String, #[serde(default)] deadline: String }
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct IsoDepartmentConfig { id: String, pub(crate) group: i64, memo: IsoMemo, folders: Vec<String> }
impl IsoConfig {
    pub(crate) fn similarity_folders(&self) -> Vec<String> {
        if !self.similarity_scope.is_empty() { return self.similarity_scope.clone(); }
        self.departments.iter().flat_map(|d| d.folders.clone()).collect()
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct IsoMemo { #[serde(default)] total: i64 }
fn default_ratio() -> f64 { 0.2 }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IsoOverview { branch: String, target_ratio: f64, groups: Vec<IsoGroupSummary>, departments: Vec<IsoDepartmentSummary> }
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct IsoGroupSummary { id: i64, label: String, deadline: String, departments: i64, documents: i64, downloaded: i64, indexed: i64, reviewed: i64, projected: i64, target: i64 }
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct IsoDepartmentSummary { id: String, group: i64, memo_total: i64, found: i64, downloaded: i64, indexed: i64, active: i64, archive: i64, duplicate: i64, review: i64, approved: i64, rejected: i64, pending: i64, projected: i64, target: i64 }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IsoDocument { node_id: String, portal_id: String, name: String, path: String, code: String, doc_type: String, revision: String, title: String, status: String, reason: String, related: String, cluster: String, merge_into: String, iso_clauses: Vec<String>, source: String, protected: bool, decision: String, decision_note: String, decided_by: String, decided_at: String, downloaded: bool }

#[derive(Deserialize)]
struct IsoImportFile { documents: Vec<IsoImportDocument> }
#[derive(Deserialize)]
struct IsoImportDocument {
    node_id: String, status: String,
    #[serde(default)] department: String, #[serde(default)] group: i64, #[serde(default)] code: String, #[serde(default)] doc_type: String,
    #[serde(default)] revision: String, #[serde(default)] rev_num: Option<i64>, #[serde(default)] rev_letter: String, #[serde(default)] title: String,
    #[serde(default)] reason: String, #[serde(default)] related: String, #[serde(default)] cluster: String, #[serde(default)] merge_into: String,
    #[serde(default)] iso_clauses: Vec<String>, #[serde(default)] source: String, #[serde(default)] protected: bool,
}
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IsoImportResult { imported: i64, skipped: i64, reset_decisions: i64 }

pub(crate) fn config(db: &Connection) -> Result<Option<IsoConfig>> {
    let raw = setting(db, "iso_config")?;
    if raw.is_empty() { return Ok(None); }
    serde_json::from_str(&raw).map(Some).map_err(err)
}
pub(crate) fn like_prefix(folder: &str) -> String { format!("{}/%", folder.trim_end_matches('/').replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")) }
pub(crate) fn department_nodes(db: &Connection, department: &IsoDepartmentConfig) -> Result<Vec<Node>> {
    let mut seen = HashSet::new();
    let mut nodes = Vec::new();
    let mut stmt = db.prepare("SELECT id,portal_id,kind,name,path,active FROM nodes WHERE kind='file' AND active=1 AND path LIKE ?1 ESCAPE '\\' ORDER BY path").map_err(err)?;
    for folder in &department.folders {
        for n in stmt.query_map([like_prefix(folder)], row_node).map_err(err)? {
            let n = n.map_err(err)?;
            if seen.insert(n.id.clone()) { nodes.push(n); }
        }
    }
    Ok(nodes)
}
fn downloaded_ids(db: &Connection) -> Result<HashSet<String>> {
    let mut stmt = db.prepare("SELECT node_id,file_path FROM job_items WHERE status IN ('completed','skipped') AND file_path IS NOT NULL").map_err(err)?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(err)?;
    let mut ids = HashSet::new();
    for row in rows { let (id, path) = row.map_err(err)?; if Path::new(&path).is_file() { ids.insert(id); } }
    Ok(ids)
}
struct ReviewState { status: String, decision: String, merge_into: String }
fn review_states(db: &Connection) -> Result<HashMap<String, ReviewState>> {
    let mut stmt = db.prepare("SELECT node_id,status,decision,merge_into FROM iso_reviews").map_err(err)?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, ReviewState { status: r.get(1)?, decision: r.get(2)?, merge_into: r.get(3)? }))).map_err(err)?;
    rows.collect::<std::result::Result<HashMap<_, _>, _>>().map_err(err)
}

/// Documents expected to remain: ARCHIVE/DUPLICATE leave unless the owner rejected the proposal,
/// and documents proposed to merge into the same target count once (unless that merge was rejected).
fn projected(ids: &[String], reviews: &HashMap<String, ReviewState>) -> i64 {
    let mut individual = 0;
    let mut targets = HashSet::new();
    for id in ids {
        match reviews.get(id) {
            None => individual += 1,
            Some(r) if r.decision == "rejected" => individual += 1,
            Some(r) if r.status == "ARCHIVE" || r.status == "DUPLICATE" => {}
            Some(r) if !r.merge_into.is_empty() => { targets.insert(r.merge_into.clone()); }
            Some(_) => individual += 1,
        }
    }
    individual + targets.len() as i64
}

pub(crate) fn overview(db: &Connection) -> Result<Option<IsoOverview>> {
    let Some(cfg) = config(db)? else { return Ok(None) };
    let downloaded = downloaded_ids(db)?;
    let indexed = similar::indexed_ids(db)?;
    let reviews = review_states(db)?;
    let mut departments = Vec::new();
    let mut groups: BTreeMap<i64, IsoGroupSummary> = BTreeMap::new();
    for (key, g) in &cfg.groups {
        let id = key.parse::<i64>().map_err(|_| format!("รหัสกลุ่ม {key} ต้องเป็นตัวเลข"))?;
        groups.insert(id, IsoGroupSummary { id, label: g.label.clone(), deadline: g.deadline.clone(), ..Default::default() });
    }
    for d in &cfg.departments {
        let ids: Vec<String> = department_nodes(db, d)?.into_iter().map(|n| n.id).collect();
        let mut s = IsoDepartmentSummary { id: d.id.clone(), group: d.group, memo_total: d.memo.total, found: ids.len() as i64, ..Default::default() };
        for id in &ids {
            if downloaded.contains(id) { s.downloaded += 1; }
            if indexed.contains(id) { s.indexed += 1; }
            if let Some(r) = reviews.get(id) {
                match r.status.as_str() { "ACTIVE" => s.active += 1, "ARCHIVE" => s.archive += 1, "DUPLICATE" => s.duplicate += 1, _ => s.review += 1 }
                match r.decision.as_str() { "approved" => s.approved += 1, "rejected" => s.rejected += 1, _ => s.pending += 1 }
            }
        }
        s.projected = projected(&ids, &reviews);
        s.target = (d.memo.total.max(s.found) as f64 * cfg.target_ratio).ceil() as i64;
        let g = groups.entry(d.group).or_insert_with(|| IsoGroupSummary { id: d.group, label: format!("กลุ่ม {}", d.group), ..Default::default() });
        g.departments += 1; g.documents += s.found; g.downloaded += s.downloaded; g.indexed += s.indexed; g.projected += s.projected; g.target += s.target;
        g.reviewed += s.active + s.archive + s.duplicate + s.review;
        departments.push(s);
    }
    Ok(Some(IsoOverview { branch: cfg.branch, target_ratio: cfg.target_ratio, groups: groups.into_values().collect(), departments }))
}

pub(crate) fn import_results(db: &mut Connection, raw: &str) -> Result<IsoImportResult> {
    let file: IsoImportFile = serde_json::from_str(raw).map_err(|e| format!("ไฟล์ผลคัดแยกไม่ถูกต้อง: {e}"))?;
    let tx = db.transaction().map_err(err)?;
    let mut result = IsoImportResult { imported: 0, skipped: 0, reset_decisions: 0 };
    for d in file.documents {
        let status = d.status.trim().to_uppercase();
        let exists = tx.query_row("SELECT 1 FROM nodes WHERE id=?1", [&d.node_id], |_| Ok(())).optional().map_err(err)?.is_some();
        if !STATUSES.contains(&status.as_str()) || !exists { result.skipped += 1; continue; }
        let previous: Option<(String, String, String)> = tx.query_row("SELECT status,merge_into,decision FROM iso_reviews WHERE node_id=?1", [&d.node_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(err)?;
        // A decision only stays valid for the proposal it was made on.
        let keep = previous.as_ref().is_some_and(|(s, m, _)| *s == status && *m == d.merge_into);
        if previous.as_ref().is_some_and(|(_, _, decision)| decision != "pending") && !keep { result.reset_decisions += 1; }
        let revision = if d.revision.is_empty() { d.rev_num.map(|n| format!("{n}{}", d.rev_letter)).unwrap_or_default() } else { d.revision };
        let source = if d.source.is_empty() { "rule".to_string() } else { d.source };
        tx.execute("INSERT INTO iso_reviews(node_id,department,group_no,code,doc_type,revision,title,status,reason,related,cluster,merge_into,iso_clauses,source,protected,imported_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?16,CURRENT_TIMESTAMP)
            ON CONFLICT(node_id) DO UPDATE SET department=excluded.department,group_no=excluded.group_no,code=excluded.code,doc_type=excluded.doc_type,
            revision=excluded.revision,title=excluded.title,status=excluded.status,reason=excluded.reason,related=excluded.related,cluster=excluded.cluster,
            merge_into=excluded.merge_into,iso_clauses=excluded.iso_clauses,source=excluded.source,protected=excluded.protected,imported_at=CURRENT_TIMESTAMP,
            decision=CASE WHEN ?15 THEN decision ELSE 'pending' END,decision_note=CASE WHEN ?15 THEN decision_note ELSE '' END,
            decided_by=CASE WHEN ?15 THEN decided_by ELSE '' END,decided_at=CASE WHEN ?15 THEN decided_at ELSE '' END",
            params![d.node_id, d.department, d.group, d.code, d.doc_type, revision, d.title, status, d.reason, d.related, d.cluster, d.merge_into,
                serde_json::to_string(&d.iso_clauses).map_err(err)?, source, keep, d.protected]).map_err(err)?;
        result.imported += 1;
    }
    tx.commit().map_err(err)?;
    Ok(result)
}

pub(crate) fn documents(db: &Connection, department: &str) -> Result<Vec<IsoDocument>> {
    let cfg = config(db)?.ok_or("กรุณานำเข้าไฟล์ตั้งค่าแผนกก่อน")?;
    let dept = cfg.departments.iter().find(|d| d.id == department).ok_or("ไม่พบแผนกในไฟล์ตั้งค่า")?;
    let downloaded = downloaded_ids(db)?;
    let mut stmt = db.prepare("SELECT code,doc_type,revision,title,status,reason,related,cluster,merge_into,iso_clauses,source,protected,decision,decision_note,decided_by,decided_at FROM iso_reviews WHERE node_id=?1").map_err(err)?;
    let mut rows = Vec::new();
    for n in department_nodes(db, dept)? {
        let downloaded = downloaded.contains(&n.id);
        let reviewed = stmt.query_row([&n.id], |r| Ok(IsoDocument {
            node_id: n.id.clone(), portal_id: n.portal_id.clone(), name: n.name.clone(), path: n.path.clone(),
            code: r.get(0)?, doc_type: r.get(1)?, revision: r.get(2)?, title: r.get(3)?, status: r.get(4)?, reason: r.get(5)?, related: r.get(6)?, cluster: r.get(7)?, merge_into: r.get(8)?,
            iso_clauses: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(), source: r.get(10)?, protected: r.get(11)?,
            decision: r.get(12)?, decision_note: r.get(13)?, decided_by: r.get(14)?, decided_at: r.get(15)?, downloaded,
        })).optional().map_err(err)?;
        rows.push(reviewed.unwrap_or_else(|| IsoDocument {
            title: n.name.clone(), node_id: n.id, portal_id: n.portal_id, name: n.name, path: n.path, code: String::new(), doc_type: String::new(), revision: String::new(),
            status: String::new(), reason: String::new(), related: String::new(), cluster: String::new(), merge_into: String::new(), iso_clauses: Vec::new(), source: String::new(),
            protected: false, decision: "pending".into(), decision_note: String::new(), decided_by: String::new(), decided_at: String::new(), downloaded,
        }));
    }
    rows.sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.name.cmp(&b.name)));
    Ok(rows)
}

pub(crate) fn decide(db: &Connection, node_id: &str, decision: &str, note: &str, decided_by: &str) -> Result<()> {
    if !DECISIONS.contains(&decision) { return Err("ผลการพิจารณาไม่ถูกต้อง".into()); }
    if decision != "pending" && decided_by.trim().is_empty() { return Err("กรุณาระบุชื่อผู้อนุมัติ".into()); }
    let changed = db.execute("UPDATE iso_reviews SET decision=?2,decision_note=?3,decided_by=?4,decided_at=CASE WHEN ?2='pending' THEN '' ELSE CURRENT_TIMESTAMP END WHERE node_id=?1",
        params![node_id, decision, note.trim(), if decision == "pending" { "" } else { decided_by.trim() }]).map_err(err)?;
    if changed == 0 { Err("เอกสารนี้ยังไม่มีผลคัดแยก".into()) } else { Ok(()) }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) { format!("\"{}\"", value.replace('"', "\"\"")) } else { value.to_string() }
}
pub(crate) fn export_csv(db: &Connection) -> Result<String> {
    let mut out = String::from("\u{feff}department,group,code,doc_type,revision,title,status,reason,merge_into,iso_clauses,decision,decision_note,decided_by,decided_at,portal_id,path,protected\n");
    let mut stmt = db.prepare("SELECT r.department,r.group_no,r.code,r.doc_type,r.revision,r.title,r.status,r.reason,r.merge_into,r.iso_clauses,r.decision,r.decision_note,r.decided_by,r.decided_at,n.portal_id,n.path,CASE WHEN r.protected THEN 'ห้ามยุบ' ELSE '' END
        FROM iso_reviews r JOIN nodes n ON n.id=r.node_id ORDER BY r.group_no,r.department,r.code").map_err(err)?;
    let rows = stmt.query_map([], |r| (0..17).map(|i| r.get::<_, rusqlite::types::Value>(i).map(|v| match v { rusqlite::types::Value::Integer(n) => n.to_string(), rusqlite::types::Value::Text(t) => t, _ => String::new() })).collect::<rusqlite::Result<Vec<String>>>()).map_err(err)?;
    for row in rows {
        let mut row = row.map_err(err)?;
        row[9] = serde_json::from_str::<Vec<String>>(&row[9]).unwrap_or_default().join(" ");
        out.push_str(&row.iter().map(|v| csv_field(v)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    Ok(out)
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub fn iso_import_config(state: State<'_, AppState>, path: String) -> Result<Option<IsoOverview>> {
    let raw = fs::read_to_string(&path).map_err(err)?;
    let cfg: IsoConfig = serde_json::from_str(&raw).map_err(|e| format!("ไฟล์ตั้งค่าแผนกไม่ถูกต้อง: {e}"))?;
    if cfg.departments.is_empty() { return Err("ไฟล์ตั้งค่าไม่มีรายชื่อแผนก".into()); }
    let db = state.db.lock().map_err(err)?;
    db.execute("INSERT INTO settings(key,value) VALUES('iso_config',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&cfg).map_err(err)?]).map_err(err)?;
    db.execute("INSERT INTO settings(key,value) VALUES('iso_config_path',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&path]).map_err(err)?;
    overview(&db)
}

/// Live progress written by iso-review/scripts/review_department.py (output/agent_status.json next to the config folder).
fn agent_status_file(config_path: &str) -> Result<PathBuf> {
    let root = Path::new(config_path).parent().and_then(Path::parent).ok_or("ตำแหน่งไฟล์ตั้งค่าแผนกไม่ถูกต้อง")?;
    Ok(root.join("output").join("agent_status.json"))
}
fn read_agent_status(file: &Path) -> Result<Option<serde_json::Value>> {
    let Ok(meta) = fs::metadata(file) else { return Ok(None) };
    if meta.len() > 2 * 1024 * 1024 { return Err("ไฟล์สถานะ AI ใหญ่ผิดปกติ".into()); }
    // The script replaces the file atomically, but tolerate a half-written read by reporting "no status" for this poll.
    Ok(serde_json::from_str(&fs::read_to_string(file).map_err(err)?).ok())
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn iso_agent_status(state: State<'_, AppState>) -> Result<Option<serde_json::Value>> {
    let path = { let db = state.db.lock().map_err(err)?; setting(&db, "iso_config_path")? };
    if path.is_empty() { return Err("กรุณากด ตั้งค่าแผนก แล้วเลือก departments.json อีกครั้ง เพื่อให้แอปรู้ตำแหน่งโฟลเดอร์ iso-review".into()); }
    read_agent_status(&agent_status_file(&path)?)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn iso_overview(state: State<'_, AppState>) -> Result<Option<IsoOverview>> { let db = state.db.lock().map_err(err)?; overview(&db) }
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn iso_documents(state: State<'_, AppState>, department: String) -> Result<Vec<IsoDocument>> { let db = state.db.lock().map_err(err)?; documents(&db, &department) }
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn iso_import_results(state: State<'_, AppState>, path: String) -> Result<IsoImportResult> {
    let raw = fs::read_to_string(&path).map_err(err)?;
    let mut db = state.db.lock().map_err(err)?;
    import_results(&mut db, &raw)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn iso_decide(state: State<'_, AppState>, node_id: String, decision: String, note: String, decided_by: String) -> Result<()> {
    let db = state.db.lock().map_err(err)?; decide(&db, &node_id, &decision, &note, &decided_by)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn iso_export(state: State<'_, AppState>, path: String) -> Result<String> {
    let csv = { let db = state.db.lock().map_err(err)?; export_csv(&db)? };
    fs::write(&path, csv).map_err(err)?;
    Ok(path)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn iso_start_download(app: AppHandle, state: State<'_, AppState>, group: i64) -> Result<String> {
    let (p, items) = {
        let db = state.db.lock().map_err(err)?;
        let cfg = config(&db)?.ok_or("กรุณานำเข้าไฟล์ตั้งค่าแผนกก่อน")?;
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for d in cfg.departments.iter().filter(|d| group == 0 || d.group == group) {
            for n in department_nodes(&db, d)? { if seen.insert(n.id.clone()) { items.push(n); } }
        }
        (profile(&db)?, items)
    };
    launch_download(app, &state, p, items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        init_db(&db).unwrap();
        for (id, path) in [("file:1", "Portal/ISO/VSX/RM2/WI/RM2-W-1(11C) A"), ("file:2", "Portal/ISO/VSX/RM2/WI/RM2-W-2(8B) B"), ("file:3", "Portal/ISO/VSX/RM2/WI/RM2-W-3(8B) C"),
            ("file:4", "Portal/ISO/VSX/RM2/WI/RM2-W-4(16C) D"), ("file:5", "Portal/ISO/HQ/MK/MK-P-3(10D) E"), ("file:6", "Portal/ISO/VSX/RM2_OLD/X")] {
            db.execute("INSERT INTO nodes(id,portal_id,kind,name,path) VALUES(?1,?2,'file',?3,?4)", params![id, &id[5..], path.rsplit('/').next().unwrap(), path]).unwrap();
        }
        let cfg = r#"{"branch":"VSX","target_ratio":0.2,"groups":{"1":{"label":"เล็ก","deadline":"2026-10-21"}},
            "departments":[{"id":"RM2","group":1,"memo":{"total":4},"folders":["Portal/ISO/VSX/RM2"]},{"id":"MK","group":1,"memo":{"total":1},"folders":["Portal/ISO/HQ/MK"]}]}"#;
        let parsed: IsoConfig = serde_json::from_str(cfg).unwrap();
        db.execute("INSERT INTO settings(key,value) VALUES('iso_config',?1)", [serde_json::to_string(&parsed).unwrap()]).unwrap();
        db
    }
    fn import(db: &mut Connection, docs: &str) -> IsoImportResult { import_results(db, &format!(r#"{{"documents":[{docs}]}}"#)).unwrap() }

    #[test]
    fn department_folders_do_not_match_sibling_prefixes() {
        let db = fixture();
        let o = overview(&db).unwrap().unwrap();
        let rm2 = o.departments.iter().find(|d| d.id == "RM2").unwrap();
        assert_eq!(rm2.found, 4, "RM2_OLD must not be counted as RM2");
        assert_eq!(rm2.projected, 4);
        assert_eq!(rm2.target, 1);
        assert_eq!(o.groups[0].documents, 5);
    }

    #[test]
    fn projection_drops_archived_and_collapses_merges_unless_rejected() {
        let mut db = fixture();
        import(&mut db, r#"{"node_id":"file:1","status":"ACTIVE","merge_into":"WI รับและจัดเก็บ"},{"node_id":"file:2","status":"REVIEW","merge_into":"WI รับและจัดเก็บ"},
            {"node_id":"file:3","status":"ARCHIVE"},{"node_id":"file:4","status":"ACTIVE"}"#);
        let rm2 = |db: &Connection| overview(db).unwrap().unwrap().departments.into_iter().find(|d| d.id == "RM2").unwrap();
        assert_eq!(rm2(&db).projected, 2);
        decide(&db, "file:3", "rejected", "ยังใช้งานอยู่", "QMR").unwrap();
        assert_eq!(rm2(&db).projected, 3);
        assert_eq!(rm2(&db).rejected, 1);
    }

    #[test]
    fn reimport_keeps_decisions_only_for_unchanged_proposals() {
        let mut db = fixture();
        import(&mut db, r#"{"node_id":"file:1","status":"ARCHIVE","rev_num":11,"rev_letter":"C"},{"node_id":"file:2","status":"DUPLICATE"}"#);
        let docs = documents(&db, "RM2").unwrap();
        assert_eq!(docs.iter().find(|d| d.node_id == "file:1").unwrap().revision, "11C");
        assert_eq!(docs.iter().find(|d| d.node_id == "file:3").unwrap().status, "");
        decide(&db, "file:1", "approved", "", "QMR").unwrap();
        decide(&db, "file:2", "approved", "", "QMR").unwrap();
        let result = import(&mut db, r#"{"node_id":"file:1","status":"ARCHIVE"},{"node_id":"file:2","status":"REVIEW"},{"node_id":"file:404","status":"ACTIVE"},{"node_id":"file:4","status":"DELETE"}"#);
        assert_eq!(result, IsoImportResult { imported: 2, skipped: 2, reset_decisions: 1 });
        let decision = |id: &str| db.query_row("SELECT decision FROM iso_reviews WHERE node_id=?1", [id], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(decision("file:1"), "approved");
        assert_eq!(decision("file:2"), "pending");
    }

    #[test]
    fn agent_status_is_read_next_to_the_config_folder() {
        let root = std::env::temp_dir().join(format!("iso-agent-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join("output")).unwrap();
        let config = root.join("config").join("departments.json");
        let file = agent_status_file(&config.to_string_lossy()).unwrap();
        assert_eq!(file, root.join("output").join("agent_status.json"));
        assert!(read_agent_status(&file).unwrap().is_none(), "no file yet");
        fs::write(&file, r#"{"department":"RM1","stage":"reading","batch":2}"#).unwrap();
        assert_eq!(read_agent_status(&file).unwrap().unwrap()["department"], "RM1");
        fs::write(&file, r#"{"department":"RM1","sta"#).unwrap();
        assert!(read_agent_status(&file).unwrap().is_none(), "a half-written file is skipped, not an error");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn decisions_require_reviewer_and_known_document() {
        let mut db = fixture();
        import(&mut db, r#"{"node_id":"file:1","status":"ACTIVE"}"#);
        assert!(decide(&db, "file:1", "approved", "", " ").is_err());
        assert!(decide(&db, "file:1", "delete", "", "QMR").is_err());
        assert!(decide(&db, "file:2", "approved", "", "QMR").is_err());
        decide(&db, "file:1", "approved", "ok", "QMR").unwrap();
        decide(&db, "file:1", "pending", "", "QMR").unwrap();
        let (by, at): (String, String) = db.query_row("SELECT decided_by,decided_at FROM iso_reviews WHERE node_id='file:1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert!(by.is_empty() && at.is_empty());
    }

    #[test]
    fn export_escapes_thai_titles_and_joins_clauses() {
        let mut db = fixture();
        import(&mut db, r#"{"node_id":"file:5","department":"MK","group":1,"status":"DUPLICATE","title":"การทบทวน, \"SPEC\"","iso_clauses":["9001:8.2.3","9001:7.5"]}"#);
        import(&mut db, r#"{"node_id":"file:4","department":"RM2","status":"ACTIVE","protected":true}"#);
        assert!(documents(&db, "RM2").unwrap().iter().find(|d| d.node_id == "file:4").unwrap().protected);
        init_iso_db(&db).unwrap();
        let csv = export_csv(&db).unwrap();
        assert!(csv.contains(",ห้ามยุบ\n"));
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("\"การทบทวน, \"\"SPEC\"\"\""));
        assert!(csv.contains(",9001:8.2.3 9001:7.5,"));
    }
}
