//! Генератор index.yaml сканом (specs/Passport.md, разделы 1/5/6; I1:
//! "Индекс сборки строится сканом, список элементов руками не ведётся").
//!
//! Сканирует `elements/*/passport.yaml` (не корневой passport.yaml —
//! это сборка, kind: assembly, не член), для каждого считает
//! `element_hash` по канонической форме (ключи отсортированы, без
//! `note`/`open_questions`/`sources` — section 5), рекурсивно включая
//! хеши зависимостей по `depends_on[].id` (id — имя директории под
//! `elements/`, подтверждено по всем текущим passport.yaml). Пишет
//! `index.yaml` в корень репозитория с `assembly_hash =
//! hash(отсортированные element_hash)`.
//!
//! Запуск: `cargo run --release --bin index-gen` из `tools/index-gen/`,
//! читает/пишет относительно корня репозитория (два уровня выше).

use serde_yaml::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

const EXCLUDED_TOP_LEVEL_FIELDS: &[&str] = &["note", "open_questions", "sources"];

struct Element {
    passport_path: PathBuf,
    value: Value,
}

fn main() {
    let repo_root = find_repo_root();
    let elements_dir = repo_root.join("elements");

    let mut elements: BTreeMap<String, Element> = BTreeMap::new();
    for entry in fs::read_dir(&elements_dir).unwrap_or_else(|e| panic!("не удалось прочитать {elements_dir:?}: {e}")) {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let passport_path = path.join("passport.yaml");
        if !passport_path.exists() {
            continue; // директория без паспорта — не элемент (specs/Passport.md, I1)
        }
        let id = path.file_name().unwrap().to_string_lossy().to_string();
        let text = fs::read_to_string(&passport_path)
            .unwrap_or_else(|e| panic!("не удалось прочитать {passport_path:?}: {e}"));
        let value: Value = serde_yaml::from_str(&text)
            .unwrap_or_else(|e| panic!("не удалось распарсить {passport_path:?}: {e}"));
        elements.insert(id, Element { passport_path, value });
    }

    let mut element_hashes: HashMap<String, String> = HashMap::new();
    let ids: Vec<String> = elements.keys().cloned().collect();
    for id in &ids {
        compute_hash(id, &elements, &mut element_hashes, &mut Vec::new());
    }

    let mut members = serde_yaml::Sequence::new();
    for (id, element) in &elements {
        let v = &element.value;
        let depends_on: Vec<String> = v
            .get("depends_on")
            .and_then(|d| d.as_sequence())
            .map(|seq| seq.iter().filter_map(|d| d.get("id").and_then(|i| i.as_str()).map(String::from)).collect())
            .unwrap_or_default();

        let mut member = serde_yaml::Mapping::new();
        member.insert("id".into(), id.clone().into());
        member.insert(
            "path".into(),
            element.passport_path.strip_prefix(&repo_root).unwrap_or(&element.passport_path).to_string_lossy().into_owned().into(),
        );
        member.insert("kind".into(), str_field(v, "kind").into());
        member.insert("status".into(), str_field(v, "status").into());
        member.insert("version".into(), str_field(v, "version").into());
        member.insert("role".into(), str_field(v, "role").into());
        member.insert("depends_on".into(), Value::Sequence(depends_on.into_iter().map(Value::from).collect()));
        member.insert("element_hash".into(), element_hashes[id].clone().into());
        members.push(Value::Mapping(member));
    }

    let mut sorted_hashes: Vec<String> = element_hashes.values().cloned().collect();
    sorted_hashes.sort();
    let assembly_hash = sha256_hex(&sorted_hashes.join(","));

    let mut root = serde_yaml::Mapping::new();
    root.insert("generated_by".into(), "tools/index-gen (specs/Passport.md, разделы 1/5/6; I1)".into());
    root.insert("member_count".into(), (members.len() as u64).into());
    root.insert("assembly_hash".into(), assembly_hash.clone().into());
    root.insert("members".into(), Value::Sequence(members));

    let header = "# index.yaml — генерируется сканом (I1), руками не правится.\n\
                  # Перегенерировать: cargo run --release --bin index-gen (из tools/index-gen/).\n";
    let body = serde_yaml::to_string(&Value::Mapping(root)).expect("сериализация index.yaml");
    let out_path = repo_root.join("index.yaml");
    fs::write(&out_path, format!("{header}{body}")).unwrap_or_else(|e| panic!("не удалось записать {out_path:?}: {e}"));

    println!("Записано {out_path:?}: {} элементов, assembly_hash={assembly_hash}", ids.len());
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// `element_hash = hash(каноническая форма passport.yaml + element_hash
/// зависимостей)` — section 5. Рекурсивно по `depends_on[].id`;
/// неизвестные зависимости (вне нашего дерева) пропускаются — их нечем
/// хешировать.
fn compute_hash(
    id: &str,
    elements: &BTreeMap<String, Element>,
    cache: &mut HashMap<String, String>,
    stack: &mut Vec<String>,
) -> String {
    if let Some(h) = cache.get(id) {
        return h.clone();
    }
    if stack.contains(&id.to_string()) {
        panic!("цикл зависимостей через {id}: {stack:?} — passport.yaml описывает невозможный граф");
    }
    stack.push(id.to_string());

    let element = elements.get(id).expect("id пришёл из того же BTreeMap");
    let mut dep_hashes = Vec::new();
    if let Some(deps) = element.value.get("depends_on").and_then(|d| d.as_sequence()) {
        for dep in deps {
            if let Some(dep_id) = dep.get("id").and_then(|i| i.as_str()) {
                if elements.contains_key(dep_id) {
                    dep_hashes.push(compute_hash(dep_id, elements, cache, stack));
                }
            }
        }
    }
    dep_hashes.sort();

    let canonical = canonical_string(&strip_excluded(&element.value));
    let combined = format!("{canonical}|{}", dep_hashes.join(","));
    let hash = sha256_hex(&combined);

    stack.pop();
    cache.insert(id.to_string(), hash.clone());
    hash
}

/// Убрать `note`/`open_questions`/`sources` на верхнем уровне — section 5:
/// "без полей, не меняющих обещаний".
fn strip_excluded(value: &Value) -> Value {
    match value {
        Value::Mapping(map) => {
            let mut out = serde_yaml::Mapping::new();
            for (k, v) in map {
                if let Value::String(key) = k {
                    if EXCLUDED_TOP_LEVEL_FIELDS.contains(&key.as_str()) {
                        continue;
                    }
                }
                out.insert(k.clone(), v.clone());
            }
            Value::Mapping(out)
        }
        other => other.clone(),
    }
}

/// Детерминированная текстовая форма: ключи отсортированы рекурсивно,
/// без зависимости от порядка вставки serde_yaml::Mapping.
fn canonical_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Sequence(seq) => {
            let items: Vec<String> = seq.iter().map(canonical_string).collect();
            format!("[{}]", items.join(","))
        }
        Value::Mapping(map) => {
            let mut entries: Vec<(String, String)> =
                map.iter().map(|(k, v)| (canonical_string(k), canonical_string(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let items: Vec<String> = entries.into_iter().map(|(k, v)| format!("{k}:{v}")).collect();
            format!("{{{}}}", items.join(","))
        }
        Value::Tagged(t) => canonical_string(&t.value),
    }
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Ищет корень репозитория, поднимаясь от текущей директории до той, где
/// лежит корневой `passport.yaml` (kind: assembly) — не зависит от того,
/// откуда запущен бинарник (cargo run из tools/index-gen/ или из корня).
fn find_repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("cwd");
    loop {
        if dir.join("passport.yaml").exists() && dir.join("elements").is_dir() {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => panic!("не нашёл корень репозитория (passport.yaml + elements/) выше текущей директории"),
        }
    }
}

