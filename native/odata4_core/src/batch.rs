//! Разбор тела `$batch` (этап 8.2) — порт `OData4_Пакет.РазобратьПакет` (multipart/mixed и JSON batch).
//! Результат — внутренний формат 1С структуры контракта `РазобратьПакет`:
//! `Структура("Формат, Операции")`, операция — `Структура("Ид, Набор, Метод, Адрес, Заголовки, Тело, ЗависитОт")`.
//!
//! Любое нарушение формата — отказ (`None`): пакет разбирает BSL и сам даёт статус и текст ошибки. Отказ и там, где
//! поведение 1С не повторить без её таблиц: не-ASCII в методе (`ВРег`) и в именах заголовков multipart (`НРег` при
//! слиянии повторов), несколько вариантов написания Content-Type в заголовках JSON-операции (выбор зависит от
//! порядка обхода `Соответствие`), тело base64url, которое не декодируется в строгий UTF-8 без BOM и NUL.
//!
//! Для JSON batch BSL дорабатывает результат (`OData4_РазборТелNative`): заголовки операции копируются в новое
//! `Соответствие` в порядке обхода (как `ЗаголовкиJSON`), а тело, которое BSL пишет `ЗаписатьJSON`, приходит
//! `Массив` из одного значения JSON — порядок ключей объекта при записи определяет хеш-порядок `Соответствие` 1С,
//! его в Rust не повторить.

use std::collections::{HashMap, HashSet};

use crate::body::{parse_json, Json};
use crate::onec::{is_ascii, is_blank, lower_eq, trim, trim_end, upper_all_ascii, Internal};
use crate::text::{lower_keyword, Unit};

const LF: Unit = 0x0A;
const CR: Unit = 0x0D;

/// Тело операции: строка или значение JSON, которое BSL запишет `ЗаписатьJSON`.
#[derive(Debug, PartialEq)]
pub enum Body {
    Text(Vec<Unit>),
    Json(Json),
}

#[derive(Debug, PartialEq)]
pub struct Operation {
    pub id: Vec<Unit>,
    pub set: Vec<Unit>,
    pub method: Vec<Unit>,
    pub url: Vec<Unit>,
    pub headers: Vec<(Vec<Unit>, Vec<Unit>)>,
    pub body: Body,
    pub depends_on: Vec<Vec<Unit>>,
}

impl Operation {
    fn new() -> Operation {
        Operation {
            id: Vec::new(),
            set: Vec::new(),
            method: Vec::new(),
            url: Vec::new(),
            headers: Vec::new(),
            body: Body::Text(Vec::new()),
            depends_on: Vec::new(),
        }
    }
}

/// Пакет → внутренний формат или отказ. `content_type` — заголовок Content-Type запроса, `text` — тело без BOM.
pub fn batch_internal(content_type: &[Unit], text: &[Unit]) -> Option<String> {
    let (format, operations) = parse_batch(content_type, text)?;
    let mut w = Internal::new();
    w.structure(&["Формат", "Операции"], |w, i| {
        if i == 0 {
            w.str(format);
        } else {
            w.array(&operations, write_operation);
        }
    });
    Some(w.finish())
}

fn write_operation(w: &mut Internal, op: &Operation) {
    w.structure(
        &["Ид", "Набор", "Метод", "Адрес", "Заголовки", "Тело", "ЗависитОт"],
        |w, i| match i {
            0 => w.string(&op.id),
            1 => w.string(&op.set),
            2 => w.string(&op.method),
            3 => w.string(&op.url),
            4 => w.map(&op.headers, |w, v| w.string(v)),
            5 => match &op.body {
                Body::Text(t) => w.string(t),
                Body::Json(v) => w.array(std::slice::from_ref(v), |w, v| v.write(w)),
            },
            _ => w.array(&op.depends_on, |w, v| w.string(v)),
        },
    );
}

/// Формат ("multipart" | "json") и операции; `None` — отказ.
pub fn parse_batch(content_type: &[Unit], text: &[Unit]) -> Option<(&'static str, Vec<Operation>)> {
    let kind = content_kind(content_type);
    let (format, operations) = if eq(&kind, "multipart/mixed") {
        ("multipart", multipart_operations(content_type, text)?)
    } else if eq(&kind, "application/json") {
        ("json", json_operations(text)?)
    } else {
        return None;
    };
    if operations.is_empty() {
        return None;
    }
    Some((format, operations))
}

fn eq(u: &[Unit], s: &str) -> bool {
    crate::text::eq_ascii(u, s)
}

fn units(s: &str) -> Vec<Unit> {
    s.encode_utf16().collect()
}

/// `ВидСодержимого`: `НРег(СокрЛП(часть до первой «;»))`; символы вне ASCII (кроме İ и K) остаются как есть.
fn content_kind(t: &[Unit]) -> Vec<Unit> {
    let first = t.split(|c| *c == b';' as Unit).next().unwrap_or(&[]);
    lower_keyword(trim(first))
}

/// `СтрЗаменить(Текст, ВК + ПС, ПС)` и `СтрРазделить(…, ПС, Истина)`.
fn lines_crlf(text: &[Unit]) -> Vec<Vec<Unit>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c == CR && text.get(i + 1) == Some(&LF) {
            out.push(std::mem::take(&mut current));
            i += 2;
        } else if c == LF {
            out.push(std::mem::take(&mut current));
            i += 1;
        } else {
            current.push(c);
            i += 1;
        }
    }
    out.push(current);
    out
}

/// `СтрРазделить(Текст, ПС, Истина)`.
fn lines_lf(text: &[Unit]) -> Vec<Vec<Unit>> {
    text.split(|c| *c == LF).map(<[Unit]>::to_vec).collect()
}

/// `СтрСоединить(строки с Начало, ПС)`; `drop_blank_tail` — без пустых (`ПустаяСтрока`) строк в конце.
fn join_lines(lines: &[Vec<Unit>], start: usize, drop_blank_tail: bool) -> Vec<Unit> {
    let mut end = lines.len();
    if drop_blank_tail {
        while end > start && is_blank(&lines[end - 1]) {
            end -= 1;
        }
    }
    let mut out = Vec::new();
    for (n, line) in lines[start.min(end)..end].iter().enumerate() {
        if n > 0 {
            out.push(LF);
        }
        out.extend_from_slice(line);
    }
    out
}

// ---- multipart ----

/// `ГраницаИзТипа`: параметр boundary (кавычки снимаются); `None` — нет.
fn boundary_of(t: &[Unit]) -> Option<Vec<Unit>> {
    let params: Vec<&[Unit]> = t.split(|c| *c == b';' as Unit).filter(|p| !p.is_empty()).collect();
    for param in params.iter().skip(1) {
        let p = trim(param);
        let Some(pos) = p.iter().position(|c| *c == b'=' as Unit) else {
            continue;
        };
        if !lower_eq(trim(&p[..pos]), "boundary") {
            continue;
        }
        let mut v = trim(&p[pos + 1..]);
        if v.len() >= 2 && v[0] == b'"' as Unit && v[v.len() - 1] == b'"' as Unit {
            v = &v[1..v.len() - 1];
        }
        return Some(v.to_vec());
    }
    None
}

/// `СтрокиЧастей`: части между разделителями «--граница» до закрывающего «--граница--».
fn split_parts(text: &[Unit], boundary: &[Unit]) -> Option<Vec<Vec<Vec<Unit>>>> {
    let mut delimiter = units("--");
    delimiter.extend_from_slice(boundary);
    let mut closing = delimiter.clone();
    closing.extend(units("--"));
    let mut parts = Vec::new();
    let mut current: Option<Vec<Vec<Unit>>> = None;
    let mut closed = false;
    for line in lines_crlf(text) {
        let check = trim_end(&line);
        if check == closing.as_slice() {
            if let Some(part) = current.take() {
                parts.push(part);
            }
            closed = true;
            break;
        } else if check == delimiter.as_slice() {
            if let Some(part) = current.take() {
                parts.push(part);
            }
            current = Some(Vec::new());
        } else if let Some(part) = current.as_mut() {
            part.push(line);
        }
    }
    if !closed {
        return None;
    }
    Some(parts)
}

/// Заголовки в порядке вставки (`Соответствие`, заполненное `Вставить` по порядку строк).
type Headers = Vec<(Vec<Unit>, Vec<Unit>)>;

/// `ПрочитатьЗаголовки`: с позиции `start` до пустой строки; результат — позиция после неё. Время линейно по
/// числу строк: имя ищется по нижнему регистру в `HashMap`, а не перебором прежних заголовков (отзыв безопасности).
fn read_headers(lines: &[Vec<Unit>], start: usize, headers: &mut Headers) -> Option<usize> {
    let mut i = start;
    let mut last: Option<usize> = None;
    let mut by_lower: HashMap<Vec<Unit>, usize> =
        headers.iter().enumerate().map(|(at, (k, _))| (lower_keyword(k), at)).collect();
    while i < lines.len() {
        let line = &lines[i];
        i += 1;
        if is_blank(line) {
            break;
        }
        let first = line[0];
        if first == b' ' as Unit || first == b'\t' as Unit {
            if let Some(at) = last {
                let value = &mut headers[at].1;
                value.push(b' ' as Unit);
                value.extend_from_slice(trim(line));
                continue;
            }
        }
        let pos = line.iter().position(|c| *c == b':' as Unit);
        let name = match pos {
            Some(pos) => trim(&line[..pos]),
            None => &[],
        };
        if name.is_empty() || name.contains(&(b' ' as Unit)) || !is_ascii(name) {
            return None;
        }
        let value = trim(&line[pos? + 1..]);
        match by_lower.get(&lower_keyword(name)) {
            Some(&at) => {
                let merged = &mut headers[at].1;
                merged.extend(units(", "));
                merged.extend_from_slice(value);
                last = Some(at);
            }
            None => {
                headers.push((name.to_vec(), value.to_vec()));
                by_lower.insert(lower_keyword(name), headers.len() - 1);
                last = Some(headers.len() - 1);
            }
        }
    }
    Some(i)
}

struct MimePart {
    content_type: Vec<Unit>,
    encoding: Vec<Unit>,
    id: Vec<Unit>,
    content: Vec<Unit>,
}

/// `ЧастьMIME`.
fn mime_part(lines: &[Vec<Unit>]) -> Option<MimePart> {
    let mut headers = Headers::new();
    let index = read_headers(lines, 0, &mut headers)?;
    let mut part = MimePart { content_type: Vec::new(), encoding: Vec::new(), id: Vec::new(), content: Vec::new() };
    for (name, value) in headers {
        if lower_eq(&name, "content-type") {
            part.content_type = value;
        } else if lower_eq(&name, "content-transfer-encoding") {
            part.encoding = value;
        } else if lower_eq(&name, "content-id") {
            part.id = without_angle_brackets(&value);
        }
    }
    part.content = join_lines(lines, index, false);
    Some(part)
}

fn without_angle_brackets(value: &[Unit]) -> Vec<Unit> {
    let v = trim(value);
    if v.len() >= 2 && v[0] == b'<' as Unit && v[v.len() - 1] == b'>' as Unit {
        v[1..v.len() - 1].to_vec()
    } else {
        v.to_vec()
    }
}

/// `ОперацииMultipart`.
fn multipart_operations(content_type: &[Unit], text: &[Unit]) -> Option<Vec<Operation>> {
    let boundary = boundary_of(content_type)?;
    if boundary.is_empty() {
        return None;
    }
    let mut operations = Vec::new();
    let mut set_boundaries: HashSet<Vec<Unit>> = HashSet::new();
    for lines in split_parts(text, &boundary)? {
        let part = mime_part(&lines)?;
        let kind = content_kind(&part.content_type);
        if eq(&kind, "application/http") {
            operations.push(http_operation(&part, &[])?);
        } else if eq(&kind, "multipart/mixed") {
            add_change_set(&mut operations, &part, &mut set_boundaries)?;
        } else {
            return None;
        }
    }
    Some(operations)
}

/// `ДобавитьНаборИзменений`.
fn add_change_set(operations: &mut Vec<Operation>, part: &MimePart, seen: &mut HashSet<Vec<Unit>>) -> Option<()> {
    let boundary = boundary_of(&part.content_type)?;
    if boundary.is_empty() || !seen.insert(boundary.clone()) {
        return None;
    }
    let parts = split_parts(&part.content, &boundary)?;
    if parts.is_empty() {
        return None;
    }
    let mut ids: HashSet<Vec<Unit>> = HashSet::new();
    for lines in parts {
        let inner = mime_part(&lines)?;
        if !eq(&content_kind(&inner.content_type), "application/http") {
            return None;
        }
        let operation = http_operation(&inner, &boundary)?;
        if !operation.id.is_empty() && !ids.insert(operation.id.clone()) {
            return None;
        }
        operations.push(operation);
    }
    Some(())
}

/// `ОперацияHTTP`: «МЕТОД адрес HTTP/1.1», заголовки, пустая строка, тело.
fn http_operation(part: &MimePart, set: &[Unit]) -> Option<Operation> {
    if !part.encoding.is_empty() && !lower_eq(&part.encoding, "binary") {
        return None;
    }
    let lines = lines_lf(&part.content);
    let start = lines.iter().position(|l| !is_blank(l))?;
    let request: Vec<Unit> =
        trim(&lines[start]).iter().map(|c| if *c == b'\t' as Unit { b' ' as Unit } else { *c }).collect();
    let words: Vec<&[Unit]> = request.split(|c| *c == b' ' as Unit).filter(|w| !w.is_empty()).collect();
    if words.len() != 3 || !upper_starts_http(words[2]) {
        return None;
    }
    let mut headers = Headers::new();
    let index = read_headers(&lines, start + 1, &mut headers)?;
    let mut operation = Operation::new();
    operation.id = part.id.clone();
    operation.set = set.to_vec();
    operation.method = upper_all_ascii(words[0])?;
    operation.url = words[1].to_vec();
    operation.headers = headers;
    operation.body = Body::Text(join_lines(&lines, index, true));
    Some(operation)
}

/// `СтрНачинаетсяС(ВРег(слово), "HTTP/")`: у 1С в H, T, P и «/» не переходит ни один символ вне ASCII.
fn upper_starts_http(word: &[Unit]) -> bool {
    word.len() >= 5 && word[..5].iter().zip(b"HTTP/").all(|(c, b)| *c < 128 && (*c as u8).to_ascii_uppercase() == *b)
}

// ---- JSON batch ----

/// `ОперацииJSON`.
fn json_operations(text: &[Unit]) -> Option<Vec<Operation>> {
    let root = parse_json(text)?;
    if !matches!(root, Json::Obj(_)) {
        return None;
    }
    let Some(Json::Arr(requests)) = root.get("requests") else {
        return None;
    };
    let mut operations = Vec::new();
    let mut ids: HashSet<Vec<Unit>> = HashSet::new();
    let mut groups: HashSet<Vec<Unit>> = HashSet::new();
    let mut previous_group: Vec<Unit> = Vec::new();
    for request in requests {
        if !matches!(request, Json::Obj(_)) {
            return None;
        }
        let mut operation = Operation::new();
        operation.id = string_property(request, "id", true)?;
        if ids.contains(&operation.id) {
            return None;
        }
        operation.method = upper_all_ascii(&string_property(request, "method", true)?)?;
        operation.url = string_property(request, "url", true)?;
        operation.set = string_property(request, "atomicityGroup", false)?;
        operation.headers = json_headers(request.get("headers"))?;
        operation.body = json_body(request, &operation.headers)?;
        operation.depends_on = json_depends_on(request.get("dependsOn"), &ids, &groups)?;
        if !operation.set.is_empty() && operation.set != previous_group {
            if groups.contains(&operation.set) || ids.contains(&operation.set) {
                return None;
            }
            groups.insert(operation.set.clone());
        }
        if groups.contains(&operation.id) {
            return None;
        }
        previous_group = operation.set.clone();
        ids.insert(operation.id.clone());
        operations.push(operation);
    }
    Some(operations)
}

/// Нет свойства или null — `Неопределено` у `Получить`.
fn present(value: Option<&Json>) -> Option<&Json> {
    value.filter(|v| !matches!(v, Json::Null))
}

/// `СвойствоСтрокой`: не строка — отказ; обязательное — нет или `ПустаяСтрока` — отказ.
fn string_property(request: &Json, name: &str, required: bool) -> Option<Vec<Unit>> {
    match present(request.get(name)) {
        None if required => None,
        None => Some(Vec::new()),
        Some(Json::Str(s)) if required && is_blank(s) => None,
        Some(Json::Str(s)) => Some(s.clone()),
        Some(_) => None,
    }
}

/// `ЗаголовкиJSON`: строка — как есть, число и булево — `XMLСтрока`, прочее — отказ. Порядок — порядок документа
/// (копию в порядке обхода делает BSL).
fn json_headers(value: Option<&Json>) -> Option<Headers> {
    let Some(value) = present(value) else {
        return Some(Headers::new());
    };
    let Json::Obj(items) = value else {
        return None;
    };
    items
        .iter()
        .map(|(k, v)| {
            let text = match v {
                Json::Str(s) => s.clone(),
                Json::Num(n) => units(n),
                Json::Bool(b) => units(if *b { "true" } else { "false" }),
                _ => return None,
            };
            Some((k.clone(), text))
        })
        .collect()
}

#[derive(PartialEq)]
enum BodyKind {
    None,
    Json,
    Text,
    Binary,
}

/// `ВидТела`.
fn body_kind(content_type: Option<&[Unit]>) -> BodyKind {
    let Some(t) = content_type.filter(|t| !is_blank(t)) else {
        return BodyKind::None;
    };
    let kind = content_kind(t);
    if eq(&kind, "application/json") || kind.ends_with(&units("+json")) {
        BodyKind::Json
    } else if kind.starts_with(&units("text/")) {
        BodyKind::Text
    } else {
        BodyKind::Binary
    }
}

/// `ТелоJSON`: нет или null — ""; двоичное — base64url в текст; строка не JSON-типа — как есть; прочее — значение
/// для `ЗаписатьJSON` в BSL. Больше одного написания Content-Type — отказ (выбор зависел бы от порядка обхода).
fn json_body(request: &Json, headers: &Headers) -> Option<Body> {
    let Some(value) = present(request.get("body")) else {
        return Some(Body::Text(Vec::new()));
    };
    let mut types = headers.iter().filter(|(k, _)| lower_eq(k, "content-type"));
    let content_type = types.next().map(|(_, v)| v.as_slice());
    if types.next().is_some() {
        return None;
    }
    let kind = body_kind(content_type);
    match (kind, value) {
        (BodyKind::Binary, Json::Str(s)) => from_base64url(s).map(Body::Text),
        (BodyKind::Binary, _) => None,
        (kind, Json::Str(s)) if kind != BodyKind::Json => Some(Body::Text(s.clone())),
        (_, v) => Some(Body::Json(v.clone())),
    }
}

/// `ЗависимостиJSON`: массив строк — id предшествующих запросов или имён групп.
fn json_depends_on(
    value: Option<&Json>,
    ids: &HashSet<Vec<Unit>>,
    groups: &HashSet<Vec<Unit>>,
) -> Option<Vec<Vec<Unit>>> {
    let Some(value) = present(value) else {
        return Some(Vec::new());
    };
    let Json::Arr(items) = value else {
        return None;
    };
    items
        .iter()
        .map(|item| match item {
            Json::Str(s) if ids.contains(s) || groups.contains(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// `ИзBase64URL`: строгий base64 после замены алфавита (как проверка обратным кодированием в BSL), затем UTF-8.
/// Неверный UTF-8, BOM в начале и NUL — отказ (как их декодирует 1С, не снималось).
fn from_base64url(value: &[Unit]) -> Option<Vec<Unit>> {
    let mut s: Vec<u8> = Vec::new();
    for &c in trim(value) {
        match c {
            CR | LF => {}
            0x2D => s.push(b'+'),
            0x5F => s.push(b'/'),
            c if c < 128 => s.push(c as u8),
            _ => return None,
        }
    }
    while s.last() == Some(&b'=') {
        s.pop();
    }
    if s.is_empty() {
        return Some(Vec::new());
    }
    if s.len() % 4 == 1 {
        return None;
    }
    let mut bits: u32 = 0;
    let mut count = 0;
    let mut bytes = Vec::with_capacity(s.len() * 3 / 4);
    for &c in &s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        bits = (bits << 6) | v;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    if bits != 0 {
        return None;
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) || bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    Some(text.encode_utf16().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<Unit> {
        units(s)
    }

    #[test]
    fn base64url() {
        assert_eq!(from_base64url(&u("PD94bWw_Pn5-fg")), Some(u("<?xml?>~~~")));
        assert_eq!(from_base64url(&u(" PD94bWw_Pn5-fg== ")), Some(u("<?xml?>~~~")));
        assert_eq!(from_base64url(&u("")), Some(Vec::new()));
        assert_eq!(from_base64url(&u("A")), None);
        assert_eq!(from_base64url(&u("AB")), None); // лишние биты
        assert_eq!(from_base64url(&u("AA")), None); // NUL
        assert_eq!(from_base64url(&u("0J_RgNC40LLQtdGC")), Some(u("Привет")));
        assert_eq!(from_base64url(&u("PD9 4")), None);
    }

    #[test]
    fn boundary() {
        assert_eq!(boundary_of(&u("multipart/mixed; boundary=\"b 1\"")), Some(u("b 1")));
        assert_eq!(boundary_of(&u("multipart/mixed;;BOUNDARY = x ")), Some(u("x")));
        assert_eq!(boundary_of(&u(";boundary=x")), None);
        assert_eq!(boundary_of(&u("multipart/mixed")), None);
    }

    #[test]
    fn multipart_with_change_set() {
        let text = "--b\r\nContent-Type: application/http\r\nContent-ID: <1>\r\n\r\nget Catalog_X HTTP/1.1\r\n\
                    Accept: application/json\r\n\r\n\r\n--b\r\nContent-Type: multipart/mixed; boundary=cs\r\n\r\n\
                    --cs\r\nContent-Type: application/http\r\n\r\nPOST Catalog_X HTTP/1.1\r\nX-A: 1\r\nx-a: 2\r\n\
                    \r\n{\"a\":1}\r\n\r\n--cs--\r\n--b--\r\n";
        let (format, ops) = parse_batch(&u("multipart/mixed; boundary=b"), &u(text)).unwrap();
        assert_eq!(format, "multipart");
        assert_eq!(ops.len(), 2);
        assert_eq!((ops[0].id.clone(), ops[0].method.clone(), ops[0].url.clone()), (u("1"), u("GET"), u("Catalog_X")));
        assert_eq!(ops[0].body, Body::Text(Vec::new()));
        assert_eq!(ops[1].set, u("cs"));
        assert_eq!(ops[1].headers, vec![(u("X-A"), u("1, 2"))]);
        assert_eq!(ops[1].body, Body::Text(u("{\"a\":1}")));
    }

    /// Отзыв безопасности: разбор линеен по числу заголовков. 50 000 разных имён при прежнем поиске перебором —
    /// 1,25·10⁹ сравнений с выделением памяти (минуты); повторы имени и строки продолжения сливаются, как в 1С.
    #[test]
    fn many_headers_linear() {
        let mut text = String::from("--b\r\nContent-Type: application/http\r\n\r\nGET Catalog_X HTTP/1.1\r\n");
        for i in 0..50_000 {
            text.push_str(&format!("X-H{i}: v\r\n"));
        }
        text.push_str("x-h7: w\r\n z\r\n\r\n--b--\r\n");
        let started = std::time::Instant::now();
        let (_, ops) = parse_batch(&u("multipart/mixed; boundary=b"), &u(&text)).unwrap();
        assert!(started.elapsed().as_secs() < 20, "разбор {:?}", started.elapsed());
        assert_eq!(ops[0].headers.len(), 50_000);
        assert_eq!(ops[0].headers[7], (u("X-H7"), u("v, w z")));
    }

    #[test]
    fn json_batch() {
        let text = r#"{"requests":[{"id":"1","method":"post","url":"Catalog_X","headers":{"Content-Type":"application/json","X-N":1.50},"body":{"b":1,"a":[2]}},
            {"id":"2","method":"GET","url":"$1","dependsOn":["1"],"atomicityGroup":"g"},
            {"id":"3","method":"PUT","url":"x","headers":{"content-type":"application/octet-stream"},"body":"0J_RgNC40LLQtdGC","atomicityGroup":"g"}]}"#;
        let (format, ops) = parse_batch(&u("application/json"), &u(text)).unwrap();
        assert_eq!(format, "json");
        assert_eq!(ops[0].method, u("POST"));
        assert_eq!(ops[0].headers[1], (u("X-N"), u("1.5")));
        assert!(matches!(ops[0].body, Body::Json(Json::Obj(_))));
        assert_eq!(ops[1].depends_on, vec![u("1")]);
        assert_eq!(ops[2].body, Body::Text(u("Привет")));
        for bad in [
            r#"{"requests":[]}"#,
            r#"{"requests":[{"id":"1","method":"GET","url":"x"},{"id":"1","method":"GET","url":"y"}]}"#,
            r#"{"requests":[{"id":"1","method":"GET","url":"x","atomicityGroup":"g"},{"id":"2","method":"GET","url":"x"},{"id":"3","method":"GET","url":"x","atomicityGroup":"g"}]}"#,
            r#"{"requests":[{"id":" ","method":"GET","url":"x"}]}"#,
            r#"{"requests":[{"id":"1","method":"GET","url":"x","dependsOn":["2"]}]}"#,
            r#"{"requests":[{"id":"1","method":"GET","url":"x","headers":{"a":null}}]}"#,
            r#"{"requests":[{"id":"1","method":"GET","url":"x","headers":{"Content-Type":"a/b","content-type":"a/b"},"body":"x"}]}"#,
        ] {
            assert_eq!(parse_batch(&u("application/json"), &u(bad)), None, "{bad}");
        }
    }

    #[test]
    fn internal_structure() {
        let text = r#"{"requests":[{"id":"1","method":"GET","url":"x"}]}"#;
        let s = batch_internal(&u("application/json"), &u(text)).unwrap();
        assert!(s.starts_with(
            "{\"#\",4238019d-7e49-4fc9-91db-b6b951d5cf8e,\n{2,\n{\n{\"S\",\"Формат\"},\n{\"S\",\"json\"}"
        ));
        assert!(s.contains("{\n{\"S\",\"Заголовки\"},\n{\"#\",3d48feae-a9c6-4c5a-a099-9eb6477630c6,\n{0}\n}\n}"));
    }
}
