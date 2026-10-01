//! Граница с компонентой: JSON на входе и на выходе.
//!
//! Вход — массив пар `[["$filter", "…"], ["$top", "5"], …]` в порядке перебора параметров 1С.
//! Выход — ровно один из объектов:
//! - `{"options": {…}}` — дерево разбора всех параметров (формат `OData4_Разбор.РазобратьПараметры`);
//! - `{"error": {"message": "…", "status": 400|501}}` — ошибка разбора, текст как у 1С;
//! - `{"failure": "…"}` — вход не разобран (не JSON, не массив пар); это сбой вызова, а не ошибка запроса.

use serde_json::{json, Value};

use crate::options::parse_options;
use crate::text::{units, Unit};

pub fn request_json(input: &str) -> String {
    let pairs = match read_pairs(input) {
        Ok(pairs) => pairs,
        Err(message) => return failure_json(&message),
    };
    match parse_options(&pairs) {
        Ok(options) => json!({ "options": options.to_json() }).to_string(),
        Err(e) => json!({ "error": { "message": e.message, "status": e.status } }).to_string(),
    }
}

/// `{"failure": "…"}` — сбой вызова (вход не разобран, паника в компоненте).
pub fn failure_json(message: &str) -> String {
    json!({ "failure": message }).to_string()
}

fn read_pairs(input: &str) -> Result<Vec<(String, Vec<Unit>)>, String> {
    let value: Value = serde_json::from_str(input).map_err(|e| format!("параметры не JSON: {e}"))?;
    let Some(items) = value.as_array() else {
        return Err("параметры: ожидался массив пар".to_string());
    };
    let mut pairs = Vec::with_capacity(items.len());
    for item in items {
        match item.as_array().map(Vec::as_slice) {
            Some([Value::String(name), Value::String(text)]) => pairs.push((name.clone(), units(text))),
            _ => return Err(format!("параметры: ожидалась пара строк, получено {item}")),
        }
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_error_failure() {
        let ok: Value = serde_json::from_str(&request_json(r#"[["$top","5"],["x","1"]]"#)).unwrap();
        assert_eq!(ok["options"]["top"], json!(5));
        assert_eq!(ok["options"]["filter"], Value::Null);
        let err: Value = serde_json::from_str(&request_json(r#"[["$filter","Code eq"]]"#)).unwrap();
        assert_eq!(err, json!({"error": {"message": "$filter: ожидалось выражение в позиции 8", "status": 400}}));
        let fail: Value = serde_json::from_str(&request_json("{}")).unwrap();
        assert_eq!(fail, json!({"failure": "параметры: ожидался массив пар"}));
        let fail: Value = serde_json::from_str(&request_json("[[1,2]]")).unwrap();
        assert!(fail["failure"].as_str().unwrap().starts_with("параметры: ожидалась пара строк"));
    }
}
