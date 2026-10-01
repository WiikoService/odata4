//! Выдача OData JSON (этап 8.1): текст ответа коллекции или сущности по описанию вывода (узел плана `OData4_Чтение`)
//! и строкам во внутреннем формате 1С, побайтно как `OData4_Сериализация.ЗаписатьСвойство` и
//! `OData4_Чтение.ЗаписатьСвойстваУзла` с `ЗаписьJSON`. Типы ссылок, значения перечислений, представления типов
//! и смещения часового пояса — словарь (строит 1С, хранится на процесс по ключу).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde_json::Value;

use crate::internal::{self, Node, Table};

/// Словарь выдачи: по UUID типа ссылки — имя `StandardODATA.…`, признак перечисления и представление типа;
/// значения перечислений (в том числе системных: виды движений, вид счёта) по ключу «UUID типа|атом значения» —
/// имя; UUID типа УникальныйИдентификатор; смещения пояса базы — точки местного времени `ГГГГММДДччммсс` по
/// возрастанию и смещение, действующее с этой точки.
#[derive(Debug, Default)]
pub struct Dictionary {
    types: HashMap<String, TypeInfo>,
    enums: HashMap<String, String>,
    uuid_type: String,
    tz: Vec<(String, String)>,
}

#[derive(Debug)]
struct TypeInfo {
    name: String,
    is_enum: bool,
    presentation: String,
}

fn dictionaries() -> &'static RwLock<HashMap<String, Arc<Dictionary>>> {
    static STORE: OnceLock<RwLock<HashMap<String, Arc<Dictionary>>>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Есть ли словарь с ключом.
pub fn has_dictionary(key: &str) -> bool {
    dictionaries().read().map(|m| m.contains_key(key)).unwrap_or(false)
}

/// Загружает словарь: JSON `{"types":{"uuid":["StandardODATA.Catalog_X",false,"Представление"]},
/// "enums":{"uuid|атом":"Имя"},"uuid":"uuid","tz":[["00010101000000","+03:00"],…]}`. Словарей больше 8 —
/// прежние забываются (ключ меняется только со сменой конфигурации).
pub fn load_dictionary(key: &str, json: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("словарь: {e}"))?;
    let mut d = Dictionary::default();
    if let Some(types) = v.get("types").and_then(Value::as_object) {
        for (uuid, info) in types {
            let name = info.get(0).and_then(Value::as_str).ok_or("словарь: тип без имени")?;
            d.types.insert(
                uuid.clone(),
                TypeInfo {
                    name: name.to_string(),
                    is_enum: info.get(1).and_then(Value::as_bool).unwrap_or(false),
                    presentation: info.get(2).and_then(Value::as_str).unwrap_or("").to_string(),
                },
            );
        }
    }
    if let Some(enums) = v.get("enums").and_then(Value::as_object) {
        for (k, name) in enums {
            d.enums.insert(k.clone(), name.as_str().unwrap_or("").to_string());
        }
    }
    d.uuid_type = v.get("uuid").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(tz) = v.get("tz").and_then(Value::as_array) {
        for point in tz {
            let at = point.get(0).and_then(Value::as_str).ok_or("словарь: точка пояса")?;
            let offset = point.get(1).and_then(Value::as_str).ok_or("словарь: смещение пояса")?;
            d.tz.push((at.to_string(), offset.to_string()));
        }
        d.tz.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let mut store = dictionaries().write().map_err(|_| "словарь: хранилище недоступно")?;
    if store.len() >= 8 && !store.contains_key(key) {
        store.clear();
    }
    store.insert(key.to_string(), Arc::new(d));
    Ok(())
}

/// Текст ответа. `desc` — описание вывода (JSON, см. `OData4_ВыдачаNative`), `data` — `ЗначениеВСтрокуВнутр`
/// массива таблиц: основная выборка и таблицы коллекций. Ошибка — неизвестный тип значения, вид, который пишет
/// только язык 1С, испорченные данные: вызывающий отвечает языком 1С.
pub fn output_json(desc: &str, data: &str) -> Result<String, String> {
    let d: Value = serde_json::from_str(desc).map_err(|e| format!("описание: {e}"))?;
    let key = d.get("dict").and_then(Value::as_str).ok_or("описание: нет ключа словаря")?;
    let dict = dictionaries()
        .read()
        .map_err(|_| "словарь: хранилище недоступно")?
        .get(key)
        .cloned()
        .ok_or_else(|| format!("нет словаря {key}"))?;
    let root = internal::parse(data)?;
    let items = internal::array_items(&root)?;
    let mut tables = Vec::with_capacity(items.len());
    for item in items {
        tables.push(internal::table(item)?);
    }
    let main = tables.first().ok_or("данные: нет основной таблицы")?;
    let codes: HashMap<String, usize> = d
        .get("tables")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| v.as_u64().map(|i| (k.clone(), i as usize))).collect())
        .unwrap_or_default();
    let node = compile(d.get("node").ok_or("описание: нет узла")?, main, &tables, &codes, 0)?;
    let strict = d.get("mode").and_then(Value::as_str) != Some("Совместимый");
    let w = Writer { dict: &dict, strict, groups: build_groups(&node, &tables)?, tables: &tables };
    let from = d.get("from").and_then(Value::as_u64).unwrap_or(0) as usize;
    let to = (d.get("to").and_then(Value::as_u64).unwrap_or(main.rows.len() as u64) as usize).min(main.rows.len());
    let id_null = d.get("idNull").and_then(Value::as_bool).unwrap_or(false);
    if d.get("kind").and_then(Value::as_str) == Some("rows") {
        // Порция выдачи одним ответом: только объекты строк через запятую, без обрамления.
        let mut out = String::with_capacity(data.len() / 2 + 16);
        write_rows(&mut out, &w, &node, &main.rows, from, to, id_null)?;
        return Ok(out);
    }
    let mut out = String::with_capacity(data.len() / 2 + 256);
    out.push('{');
    out.push_str("\"@odata.context\":");
    escape(&mut out, d.get("context").and_then(Value::as_str).unwrap_or(""));
    match d.get("kind").and_then(Value::as_str) {
        Some("entity") => {
            let row = main.rows.get(from).ok_or("данные: нет строки сущности")?;
            let mut first = false;
            w.props(&mut out, &node, row, &mut first)?;
        }
        _ => {
            if let Some(count) = d.get("count").filter(|c| !c.is_null()) {
                out.push_str(",\"@odata.count\":");
                out.push_str(&count.to_string());
            }
            out.push_str(",\"value\":[");
            write_rows(&mut out, &w, &node, &main.rows, from, to, id_null)?;
            out.push(']');
            if let Some(next) = d.get("next").and_then(Value::as_str) {
                out.push_str(",\"@odata.nextLink\":");
                escape(&mut out, next);
            }
        }
    }
    out.push('}');
    Ok(out)
}

/// Объекты строк `from..to` основной выборки через запятую.
fn write_rows(
    out: &mut String,
    w: &Writer,
    node: &OutNode,
    rows: &[&[Node]],
    from: usize,
    to: usize,
    id_null: bool,
) -> Result<(), String> {
    for (n, row) in rows.iter().enumerate().take(to).skip(from) {
        if n > from {
            out.push(',');
        }
        out.push('{');
        let mut first = true;
        if id_null {
            out.push_str("\"@odata.id\":null");
            first = false;
        }
        w.props(out, node, row, &mut first)?;
        out.push('}');
    }
    Ok(())
}

/// Узел вывода с колонками, найденными в таблице, где лежат его строки.
struct OutNode {
    props: Vec<Prop>,
    version: Option<usize>,
    reference: Option<usize>,
    tabs: Vec<Tab>,
    expands: Vec<(String, OutNode)>,
}

struct Prop {
    name: String,
    kind: String,
    col: Option<usize>,
    edm: String,
    date_parts: String,
    null_empty: bool,
    nav_key: bool,
    nested: Option<OutNode>,
}

struct Tab {
    name: String,
    table: usize,
    count: bool,
    skip: usize,
    first: i64,
    node: OutNode,
}

/// Предел вложенности узлов описания (раскрытия и табличные части): у плана он меньше (глубина $expand).
const MAX_DEPTH: usize = 32;

fn compile(
    v: &Value,
    table: &Table,
    tables: &[Table],
    codes: &HashMap<String, usize>,
    depth: usize,
) -> Result<OutNode, String> {
    if depth > MAX_DEPTH {
        return Err("описание: слишком глубокий узел".to_string());
    }
    let col = |name: Option<&str>| -> Result<Option<usize>, String> {
        match name {
            None | Some("") => Ok(None),
            Some(n) => table.column(n).map(Some).ok_or_else(|| format!("данные: нет колонки {n}")),
        }
    };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut props = Vec::new();
    for p in v.get("p").and_then(Value::as_array).into_iter().flatten() {
        let nested = match p.get("node") {
            Some(n) if !n.is_null() => Some(compile(n, table, tables, codes, depth + 1)?),
            _ => None,
        };
        props.push(Prop {
            name: s(p, "n"),
            kind: s(p, "k"),
            col: col(p.get("c").and_then(Value::as_str))?,
            edm: s(p, "e"),
            date_parts: s(p, "d"),
            null_empty: p.get("z").and_then(Value::as_bool).unwrap_or(false),
            nav_key: p.get("nk").and_then(Value::as_bool).unwrap_or(false),
            nested,
        });
    }
    let mut tabs = Vec::new();
    for t in v.get("t").and_then(Value::as_array).into_iter().flatten() {
        let code = s(t, "code");
        let index = *codes.get(&code).ok_or_else(|| format!("описание: нет таблицы коллекции {code}"))?;
        let own = tables.get(index).ok_or("данные: нет таблицы коллекции")?;
        tabs.push(Tab {
            name: s(t, "n"),
            table: index,
            count: t.get("cnt").and_then(Value::as_bool).unwrap_or(false),
            skip: t.get("s").and_then(Value::as_u64).unwrap_or(0) as usize,
            first: t.get("f").and_then(Value::as_i64).unwrap_or(-1),
            node: compile(t.get("node").ok_or("описание: табличная часть без узла")?, own, tables, codes, depth + 1)?,
        });
    }
    let mut expands = Vec::new();
    for x in v.get("x").and_then(Value::as_array).into_iter().flatten() {
        let node = compile(x.get("node").ok_or("описание: раскрытие без узла")?, table, tables, codes, depth + 1)?;
        expands.push((s(x, "n"), node));
    }
    Ok(OutNode {
        props,
        version: col(v.get("v").and_then(Value::as_str))?,
        reference: col(v.get("r").and_then(Value::as_str))?,
        tabs,
        expands,
    })
}

/// Строки коллекций по владельцу: номер таблицы → (ключ ссылки владельца → номера строк по порядку таблицы).
type Groups = HashMap<usize, HashMap<String, Vec<usize>>>;

fn build_groups(node: &OutNode, tables: &[Table]) -> Result<Groups, String> {
    let mut groups = Groups::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        for t in &n.tabs {
            if let std::collections::hash_map::Entry::Vacant(entry) = groups.entry(t.table) {
                let table = &tables[t.table];
                let owner = t.node.reference.ok_or("описание: у табличной части нет колонки владельца")?;
                let mut by_owner: HashMap<String, Vec<usize>> = HashMap::new();
                for (i, row) in table.rows.iter().enumerate() {
                    if let Some(k) = ref_key(value(row, Some(owner))) {
                        by_owner.entry(k).or_default().push(i);
                    }
                }
                entry.insert(by_owner);
            }
            stack.push(&t.node);
        }
        for (_, x) in &n.expands {
            stack.push(x);
        }
        for p in &n.props {
            if let Some(nested) = &p.nested {
                stack.push(nested);
            }
        }
    }
    Ok(groups)
}

static UNDEFINED: OnceLock<Node<'static>> = OnceLock::new();

fn undefined() -> &'static Node<'static> {
    UNDEFINED.get_or_init(|| Node::List(vec![Node::Str("U".into())]))
}

/// Значение колонки строки; нет колонки или значения (хвост строки) — Неопределено.
fn value<'n, 'a>(row: &'n [Node<'a>], col: Option<usize>) -> &'n Node<'a> {
    match col.and_then(|c| row.get(c)) {
        Some(v) => v,
        None => undefined(),
    }
}

/// Вид значения по первому элементу: "S", "N", "D", "B", "L", "U", "#", "T".
fn tag<'n>(v: &'n Node) -> &'n str {
    v.at(0).and_then(Node::str).unwrap_or("")
}

fn is_null(v: &Node) -> bool {
    tag(v) == "L"
}

/// Ключ ссылки для группировки строк коллекций: «UUID типа|атом»; NULL и не ссылка — нет ключа.
fn ref_key(v: &Node) -> Option<String> {
    if tag(v) != "#" {
        return None;
    }
    Some(format!("{}|{}", v.at(1)?.atom()?, v.at(2)?.atom()?))
}

const ZERO_GUID: &str = "00000000-0000-0000-0000-000000000000";

/// GUID ссылки из атома `NN:hex32` (байты — части 4, 5, 3, 2, 1 GUID).
fn guid_from_ref(atom: &str) -> Option<String> {
    let hex = atom.rsplit(':').next()?;
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("{}-{}-{}-{}-{}", &hex[24..32], &hex[20..24], &hex[16..20], &hex[0..4], &hex[4..16]))
}

fn empty_ref(atom: &str) -> bool {
    atom.rsplit(':').next().is_some_and(|h| h.bytes().all(|b| b == b'0'))
}

struct Writer<'d, 'n, 'a> {
    dict: &'d Dictionary,
    strict: bool,
    groups: Groups,
    tables: &'d [Table<'n, 'a>],
}

impl Writer<'_, '_, '_> {
    fn key(out: &mut String, first: &mut bool, name: &str) {
        if !*first {
            out.push(',');
        }
        *first = false;
        escape(out, name);
        out.push(':');
    }

    /// Свойства узла, как `ЗаписатьСвойстваУзла`.
    fn props(&self, out: &mut String, node: &OutNode, row: &[Node], first: &mut bool) -> Result<(), String> {
        if node.version.is_some() {
            if let Some(ver) = value(row, node.version).at(1).and_then(Node::str).filter(|s| !s.is_empty()) {
                Self::key(out, first, "@odata.etag");
                escape(out, &format!("W/\"{ver}\""));
            }
        }
        for p in &node.props {
            if p.kind == "Вложенный" {
                let nested = p.nested.as_ref().ok_or("описание: вложенное свойство без узла")?;
                Self::key(out, first, &p.name);
                if nested_empty(nested, row) {
                    out.push_str("null");
                } else {
                    out.push('{');
                    let mut f = true;
                    self.props(out, nested, row, &mut f)?;
                    out.push('}');
                }
                continue;
            }
            self.prop(out, first, p, value(row, p.col))?;
        }
        for t in &node.tabs {
            let rows: &[usize] = match ref_key(value(row, node.reference)) {
                Some(k) => self.groups.get(&t.table).and_then(|g| g.get(&k)).map(Vec::as_slice).unwrap_or(&[]),
                None => &[],
            };
            let total = rows.len();
            if t.count {
                Self::key(out, first, &format!("{}@odata.count", t.name));
                out.push_str(&total.to_string());
            }
            Self::key(out, first, &t.name);
            out.push('[');
            let last = if t.first < 0 { total } else { total.min(t.skip + t.first as usize) };
            let tables = &self.tables[t.table].rows;
            for (n, &i) in rows.iter().enumerate().take(last).skip(t.skip) {
                if n > t.skip {
                    out.push(',');
                }
                out.push('{');
                let mut f = true;
                self.props(out, &t.node, tables[i], &mut f)?;
                out.push('}');
            }
            out.push(']');
        }
        for (name, x) in &node.expands {
            Self::key(out, first, name);
            if is_null(value(row, x.reference)) {
                out.push_str("null");
            } else {
                out.push('{');
                let mut f = true;
                self.props(out, x, row, &mut f)?;
                out.push('}');
            }
        }
        Ok(())
    }

    /// Свойство, как `OData4_Сериализация.ЗаписатьСвойство`.
    fn prop(&self, out: &mut String, first: &mut bool, p: &Prop, v: &Node) -> Result<(), String> {
        let kind = p.kind.as_str();
        match kind {
            "Суррогат" => {
                Self::key(out, first, &p.name);
                out.push('0');
                return Ok(());
            }
            "Пусто" => return Ok(()),
            _ => {}
        }
        if is_null(v) && p.null_empty && (kind == "Ссылка" || kind == "Примитив") {
            Self::key(out, first, &p.name);
            if kind == "Ссылка" {
                escape(out, ZERO_GUID);
            } else if p.edm == "Edm.String" {
                out.push_str("\"\"");
            } else if p.edm == "Edm.Boolean" {
                out.push_str("false");
            } else if !p.date_parts.is_empty() {
                out.push_str("\"0001-01-01T00:00:00\"");
            } else {
                out.push('0');
            }
            return Ok(());
        }
        if is_null(v) && kind != "Хранилище" {
            Self::key(out, first, &p.name);
            out.push_str("null");
            if kind == "Составной" {
                Self::key(out, first, &format!("{}_Type", p.name));
                out.push_str("\"\"");
            }
            return Ok(());
        }
        if kind == "Составной" {
            let (text, type_name) = self.composite(v)?;
            Self::key(out, first, &p.name);
            match text {
                Some(t) => escape(out, &t),
                None => out.push_str("null"),
            }
            Self::key(out, first, &format!("{}_Type", p.name));
            escape(out, &type_name);
            return Ok(());
        }
        match kind {
            "Хранилище" | "ОписаниеТипов" | "ТочкаМаршрута" => {
                return Err(format!("вид {kind} пишет язык 1С"))
            }
            _ => {}
        }
        Self::key(out, first, &p.name);
        match kind {
            "ВидДвижения" => {
                let name = if tag(v) == "#" { self.enum_name(v).unwrap_or_default() } else { String::new() };
                escape(out, &name);
            }
            "ТипДокумента" => {
                let pres = if tag(v) == "T" {
                    let uuid = v.at(1).and_then(Node::atom).unwrap_or("");
                    self.dict
                        .types
                        .get(uuid)
                        .map(|t| t.presentation.clone())
                        .ok_or_else(|| format!("нет типа {uuid}"))?
                } else {
                    String::new()
                };
                escape(out, &pres);
            }
            _ if kind == "Ключ" || kind == "Ссылка" || p.edm == "Edm.Guid" => escape(out, &self.guid(v)?),
            "Перечисление" => {
                let name = self.enum_value_name(v)?;
                if name.is_empty() && p.edm != "Edm.String" {
                    out.push_str("null");
                } else {
                    escape(out, &name);
                }
            }
            _ if !p.date_parts.is_empty() => match self.date(v, &p.date_parts)? {
                Some(t) => escape(out, &t),
                None => out.push_str("null"),
            },
            _ => self.primitive(out, v)?,
        }
        Ok(())
    }

    fn primitive(&self, out: &mut String, v: &Node) -> Result<(), String> {
        match tag(v) {
            "S" => escape(out, v.at(1).and_then(Node::str).unwrap_or("")),
            "N" => out.push_str(v.at(1).and_then(Node::atom).ok_or("число без значения")?),
            "B" => out.push_str(if v.at(1).and_then(Node::atom) == Some("1") { "true" } else { "false" }),
            "U" | "L" => out.push_str("null"),
            other => return Err(format!("значение вида {other} без правила выдачи")),
        }
        Ok(())
    }

    /// `OData4_Сериализация.ГУИД`: NULL и Неопределено — нулевой GUID, УникальныйИдентификатор — как есть, ссылка —
    /// её GUID.
    fn guid(&self, v: &Node) -> Result<String, String> {
        match tag(v) {
            "L" | "U" => Ok(ZERO_GUID.to_string()),
            "#" => {
                let uuid = v.at(1).and_then(Node::atom).unwrap_or("");
                let atom = v.at(2).and_then(Node::atom).ok_or("ссылка без значения")?;
                if uuid == self.dict.uuid_type {
                    return Ok(atom.to_string());
                }
                if !self.dict.types.contains_key(uuid) {
                    return Err(format!("нет типа {uuid}"));
                }
                guid_from_ref(atom).ok_or_else(|| format!("ссылка {atom}"))
            }
            other => Err(format!("значение вида {other} вместо ссылки")),
        }
    }

    fn enum_name(&self, v: &Node) -> Option<String> {
        let uuid = v.at(1)?.atom()?;
        let atom = v.at(2)?.atom()?;
        self.dict.enums.get(&format!("{uuid}|{atom}")).cloned()
    }

    /// `ИмяЗначенияПеречисления`: NULL и пустое значение — "", иначе имя значения.
    fn enum_value_name(&self, v: &Node) -> Result<String, String> {
        match tag(v) {
            "L" | "U" => Ok(String::new()),
            "#" => {
                let atom = v.at(2).and_then(Node::atom).unwrap_or("");
                if empty_ref(atom) {
                    return Ok(String::new());
                }
                self.enum_name(v).ok_or_else(|| format!("нет значения перечисления {atom}"))
            }
            other => Err(format!("значение вида {other} вместо перечисления")),
        }
    }

    /// `ПредставлениеДаты`; None — null (пустая дата в строгом режиме).
    fn date(&self, v: &Node, parts: &str) -> Result<Option<String>, String> {
        let raw = match tag(v) {
            "D" => v.at(1).and_then(Node::atom).ok_or("дата без значения")?,
            "L" => "00010101000000",
            other => return Err(format!("значение вида {other} вместо даты")),
        };
        if raw.len() != 14 || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("дата {raw}"));
        }
        let empty = raw == "00010101000000";
        let xml =
            format!("{}-{}-{}T{}:{}:{}", &raw[0..4], &raw[4..6], &raw[6..8], &raw[8..10], &raw[10..12], &raw[12..14]);
        if !self.strict {
            return Ok(Some(if empty { "0001-01-01T00:00:00Z".to_string() } else { xml + "Z" }));
        }
        if empty {
            return Ok(None);
        }
        Ok(Some(match parts {
            "Дата" => xml[0..10].to_string(),
            "Время" => xml[11..19].to_string(),
            _ => {
                let offset = self.offset(raw).ok_or("нет смещения пояса")?;
                xml + offset
            }
        }))
    }

    fn offset(&self, local: &str) -> Option<&str> {
        let i = self.dict.tz.partition_point(|(at, _)| at.as_str() <= local);
        self.dict.tz.get(i.checked_sub(1)?).map(|(_, o)| o.as_str())
    }

    /// Значение и `_Type` составного свойства (`ЗначениеСоставного`, `ИмяТипаЗначения`); значение None — null.
    fn composite(&self, v: &Node) -> Result<(Option<String>, String), String> {
        Ok(match tag(v) {
            "U" => (Some(String::new()), "StandardODATA.Undefined".to_string()),
            "S" => (Some(v.at(1).and_then(Node::str).unwrap_or("").to_string()), "Edm.String".to_string()),
            "N" => {
                (Some(v.at(1).and_then(Node::atom).ok_or("число без значения")?.to_string()), "Edm.Decimal".to_string())
            }
            "B" => (
                Some(if v.at(1).and_then(Node::atom) == Some("1") { "true" } else { "false" }.to_string()),
                "Edm.Boolean".to_string(),
            ),
            "D" => (self.date(v, "ДатаВремя")?, "Edm.DateTimeOffset".to_string()),
            "#" => {
                let uuid = v.at(1).and_then(Node::atom).unwrap_or("");
                if uuid == self.dict.uuid_type {
                    return Ok((Some(v.at(2).and_then(Node::atom).unwrap_or("").to_string()), "Edm.Guid".to_string()));
                }
                let info = self.dict.types.get(uuid).ok_or_else(|| format!("нет типа {uuid}"))?;
                let text = if info.is_enum { self.enum_value_name(v)? } else { self.guid(v)? };
                (Some(text), info.name.clone())
            }
            other => return Err(format!("составное значение вида {other}")),
        })
    }
}

/// `ВложенныйПуст`: каждое значение узла — NULL, у ключа навигации — ещё и пустая ссылка.
fn nested_empty(node: &OutNode, row: &[Node]) -> bool {
    for p in &node.props {
        if let Some(n) = &p.nested {
            if !nested_empty(n, row) {
                return false;
            }
            continue;
        }
        if p.col.is_none() {
            return false;
        }
        let v = value(row, p.col);
        if is_null(v) || (p.nav_key && !filled(v)) {
            continue;
        }
        return false;
    }
    true
}

/// `ЗначениеЗаполнено` для значений внутреннего формата.
fn filled(v: &Node) -> bool {
    match tag(v) {
        "U" | "L" => false,
        "S" => !v.at(1).and_then(Node::str).unwrap_or("").is_empty(),
        "N" => v.at(1).and_then(Node::atom).is_some_and(|n| n.bytes().any(|b| (b'1'..=b'9').contains(&b))),
        "B" => v.at(1).and_then(Node::atom) == Some("1"),
        "D" => v.at(1).and_then(Node::atom) != Some("00010101000000"),
        "#" => !v.at(2).and_then(Node::atom).is_some_and(empty_ref),
        _ => true,
    }
}

/// Строка JSON, как `ЗаписьJSON` без экранирования: `\"`, `\\`, `\n`, `\r`, прочие управляющие — `\u00XX`
/// (заглавные), ` `, ` `; остальное как есть.
pub fn escape(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
