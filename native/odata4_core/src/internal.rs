//! Разбор внутреннего формата 1С (`ЗначениеВСтрокуВнутр`): дерево списков `{…}`, строк `"…"` (кавычка
//! удваивается) и атомов (числа, UUID, `75:hex`, `#base64:…`). Разбор без рекурсии: стек списков.
//! Строки и атомы ссылаются на исходный текст; строка с удвоенными кавычками копируется один раз.

use std::borrow::Cow;

/// Узел внутреннего формата.
#[derive(Debug, Clone, PartialEq)]
pub enum Node<'a> {
    List(Vec<Node<'a>>),
    Str(Cow<'a, str>),
    Atom(&'a str),
}

impl<'a> Node<'a> {
    pub fn list(&self) -> Option<&[Node<'a>]> {
        match self {
            Node::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn atom(&self) -> Option<&'a str> {
        match self {
            Node::Atom(a) => Some(a),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Элемент списка по номеру.
    pub fn at(&self, index: usize) -> Option<&Node<'a>> {
        self.list().and_then(|items| items.get(index))
    }
}

/// Разбирает текст внутреннего формата в дерево. Ошибка — описание и позиция (в байтах).
pub fn parse(text: &str) -> Result<Node<'_>, String> {
    let bytes = text.as_bytes();
    let mut stack: Vec<Vec<Node>> = Vec::new();
    let mut root: Option<Node> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\r' | b'\n' | b' ' | b'\t' | b',' => {
                i += 1;
            }
            b'\xef' if text[i..].starts_with('\u{feff}') => {
                i += 3;
            }
            b'{' => {
                if root.is_some() {
                    return Err(format!("внутренний формат: лишние данные в позиции {i}"));
                }
                stack.push(Vec::new());
                i += 1;
            }
            b'}' => {
                let Some(items) = stack.pop() else {
                    return Err(format!("внутренний формат: лишняя }} в позиции {i}"));
                };
                push(&mut stack, &mut root, Node::List(items), i)?;
                i += 1;
            }
            b'"' => {
                let start = i + 1;
                let mut j = start;
                let mut special = false;
                loop {
                    match bytes.get(j) {
                        None => return Err(format!("внутренний формат: незакрытая строка с позиции {i}")),
                        Some(b'"') if bytes.get(j + 1) == Some(&b'"') => {
                            special = true;
                            j += 2;
                        }
                        // `"\hhhh` — единица UTF-16 (так 1С пишет половины суррогатных пар).
                        Some(b'"') if bytes.get(j + 1) == Some(&b'\\') => {
                            special = true;
                            j += 6;
                        }
                        Some(b'"') => break,
                        Some(_) => j += 1,
                    }
                }
                let raw = text.get(start..j).ok_or_else(|| format!("внутренний формат: строка с позиции {i}"))?;
                let value = if special { Cow::Owned(unescape(raw)?) } else { Cow::Borrowed(raw) };
                push(&mut stack, &mut root, Node::Str(value), i)?;
                i = j + 1;
            }
            _ => {
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b',' | b'}' | b'{' | b'\r' | b'\n') {
                    i += 1;
                }
                // `{#base64:…}` может переноситься по строкам: хвост атома до `}` без переводов строк не
                // нужен выдаче (хранилища пишет язык 1С), поэтому атом — первая строка.
                push(&mut stack, &mut root, Node::Atom(text[start..i].trim()), start)?;
            }
        }
    }
    if !stack.is_empty() {
        return Err("внутренний формат: незакрытый список".to_string());
    }
    root.ok_or_else(|| "внутренний формат: пустой текст".to_string())
}

/// Тело строки со спецпоследовательностями: `""` — кавычка, `"\hhhh` — единица UTF-16. Пары единиц собираются в
/// символ; одиночная половина суррогатной пары в строке Rust непредставима — ошибка (ответ пишет язык 1С).
fn unescape(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut units: Vec<u16> = Vec::new();
    let mut rest = raw;
    while let Some(pos) = rest.find('"') {
        let (head, tail) = rest.split_at(pos);
        if !head.is_empty() {
            flush(&mut out, &mut units)?;
            out.push_str(head);
        }
        if let Some(after) = tail.strip_prefix("\"\"") {
            flush(&mut out, &mut units)?;
            out.push('"');
            rest = after;
        } else if let Some(hex) = tail.strip_prefix("\"\\").and_then(|t| t.get(..4)) {
            units.push(u16::from_str_radix(hex, 16).map_err(|_| format!("внутренний формат: код {hex}"))?);
            rest = &tail[6..];
        } else {
            return Err("внутренний формат: кавычка внутри строки".to_string());
        }
    }
    flush(&mut out, &mut units)?;
    out.push_str(rest);
    Ok(out)
}

fn flush(out: &mut String, units: &mut Vec<u16>) -> Result<(), String> {
    if units.is_empty() {
        return Ok(());
    }
    let text = String::from_utf16(units).map_err(|_| "внутренний формат: одиночная половина суррогатной пары")?;
    out.push_str(&text);
    units.clear();
    Ok(())
}

fn push<'a>(
    stack: &mut [Vec<Node<'a>>],
    root: &mut Option<Node<'a>>,
    node: Node<'a>,
    pos: usize,
) -> Result<(), String> {
    match stack.last_mut() {
        Some(top) => {
            top.push(node);
            Ok(())
        }
        None if root.is_none() => {
            *root = Some(node);
            Ok(())
        }
        None => Err(format!("внутренний формат: лишние данные в позиции {pos}")),
    }
}

/// UUID типа ТаблицаЗначений и Массив во внутреннем формате.
pub const VALUE_TABLE: &str = "acf6192e-81ca-46ef-93a6-5a6968b78663";
pub const ARRAY: &str = "51e7a0d2-530b-11d4-b98a-008048da3034";

/// Таблица значений: имена колонок в порядке значений строки и строки (значения по позиции; недостающие в конце
/// строки значения — Неопределено).
#[derive(Debug)]
pub struct Table<'n, 'a> {
    pub columns: Vec<String>,
    pub rows: Vec<&'n [Node<'a>]>,
}

impl<'n, 'a> Table<'n, 'a> {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}

/// Элементы массива `{"#",<Массив>,{n, элементы…}}`.
pub fn array_items<'n, 'a>(node: &'n Node<'a>) -> Result<&'n [Node<'a>], String> {
    if node.at(0).and_then(Node::str) != Some("#") || node.at(1).and_then(Node::atom) != Some(ARRAY) {
        return Err("внутренний формат: ожидался Массив".to_string());
    }
    let body = node.at(2).and_then(Node::list).ok_or("внутренний формат: массив без тела")?;
    Ok(body.get(1..).unwrap_or(&[]))
}

/// Таблица значений `{"#",<ТаблицаЗначений>,{9,{n, колонки…},{2,n, пары «позиция, id»…,{1,m, строки…},…},…}}`.
/// Строка — `{2, номер, число значений, значения…, 0}`.
pub fn table<'n, 'a>(node: &'n Node<'a>) -> Result<Table<'n, 'a>, String> {
    let bad = |what: &str| format!("внутренний формат таблицы: {what}");
    if node.at(0).and_then(Node::str) != Some("#") || node.at(1).and_then(Node::atom) != Some(VALUE_TABLE) {
        return Err(bad("ожидалась ТаблицаЗначений"));
    }
    let body = node.at(2).ok_or_else(|| bad("нет тела"))?;
    let cols = body.at(1).and_then(Node::list).ok_or_else(|| bad("нет колонок"))?;
    let mut by_id: Vec<(String, String)> = Vec::new();
    for col in cols.iter().skip(1) {
        let id = col.at(0).and_then(Node::atom).ok_or_else(|| bad("колонка без номера"))?;
        let name = col.at(1).and_then(Node::str).ok_or_else(|| bad("колонка без имени"))?;
        by_id.push((id.to_string(), name.to_string()));
    }
    let data = body.at(2).and_then(Node::list).ok_or_else(|| bad("нет данных"))?;
    let count: usize =
        data.get(1).and_then(Node::atom).and_then(|a| a.parse().ok()).ok_or_else(|| bad("нет числа колонок"))?;
    if count != by_id.len() {
        return Err(bad("число колонок не совпадает"));
    }
    let mut columns = vec![String::new(); count];
    for k in 0..count {
        let slot: usize =
            data.get(2 + 2 * k).and_then(Node::atom).and_then(|a| a.parse().ok()).ok_or_else(|| bad("пары колонок"))?;
        let id = data.get(3 + 2 * k).and_then(Node::atom).ok_or_else(|| bad("пары колонок"))?;
        let name =
            by_id.iter().find(|(i, _)| i == id).map(|(_, n)| n.clone()).ok_or_else(|| bad("неизвестная колонка"))?;
        *columns.get_mut(slot).ok_or_else(|| bad("позиция колонки"))? = name;
    }
    let rows_node = data.get(2 + 2 * count).and_then(Node::list).ok_or_else(|| bad("нет строк"))?;
    let mut rows = Vec::with_capacity(rows_node.len().saturating_sub(2));
    for row in rows_node.iter().skip(2) {
        let items = row.list().ok_or_else(|| bad("строка не список"))?;
        let n: usize = items
            .get(2)
            .and_then(Node::atom)
            .and_then(|a| a.parse().ok())
            .ok_or_else(|| bad("строка без числа значений"))?;
        let values = items.get(3..3 + n).ok_or_else(|| bad("строка короче заявленного"))?;
        rows.push(values);
    }
    Ok(Table { columns, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "{\"#\",acf6192e-81ca-46ef-93a6-5a6968b78663,\n{9,\n{3,\n{0,\"С\",\n{\"Pattern\"},\"\",0},\n{1,\"Ч\",\n{\"Pattern\",\n{\"N\",10,2}\n},\"\",0},\n{3,\"Р\",\n{\"Pattern\"},\"\",0}\n},\n{2,3,0,0,1,1,2,3,\n{1,2,\n{2,0,3,\n{\"S\",\"a\"\"b\nc\"},\n{\"N\",-1.5},\n{\"#\",1376fb02-01a4-4809-8d8d-0e5a9a6ad441,75:a1080011d85708ff11dcbf62702a156d},0},\n{2,1,1,\n{\"U\"},0}\n},2,1},\n{0,0}\n}\n}";

    #[test]
    fn parses_table() {
        let root = parse(SAMPLE).unwrap();
        let t = table(&root).unwrap();
        assert_eq!(t.columns, vec!["С", "Ч", "Р"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0][0].at(1).and_then(Node::str), Some("a\"b\nc"));
        assert_eq!(t.rows[0][1].at(1).and_then(Node::atom), Some("-1.5"));
        assert_eq!(t.rows[1].len(), 1);
        assert_eq!(t.column("Р"), Some(2));
    }

    #[test]
    fn surrogate_units_in_strings() {
        assert!(parse("{\"S\",\"a\"\\d83d\"\\de00 b\tc\"\"q\"\\d83dd\"\\de00e\"}").is_err());
        let root = parse("{\"S\",\"a\"\\d83d\"\\de00 b\tc\"\"q\"}").unwrap();
        assert_eq!(root.at(1).and_then(Node::str), Some("a😀 b\tc\"q"));
        assert!(parse("{\"S\",\"a\"\\d83d b\"}").is_err());
        assert!(parse("{\"S\",\"a\"\\zz").is_err());
    }

    #[test]
    fn rejects_broken() {
        assert!(parse("{\"a\",").is_err());
        assert!(parse("}").is_err());
        assert!(parse("{\"a").is_err());
        assert!(parse("{1}{2}").is_err());
        assert!(parse("").is_err());
    }
}
