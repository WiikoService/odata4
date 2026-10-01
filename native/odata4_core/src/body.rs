//! Разбор JSON-тела записи (этап 8.2): дерево — как у `ПрочитатьJSON(Чтение, Истина)`, результат — внутренний формат
//! 1С для `ЗначениеИзСтрокиВнутр`.
//!
//! `ЧтениеJSON` 1С снисходительнее RFC 8259: принимает висячие запятые, одинарные кавычки, `[1 2]`, `01`, `1e`,
//! сырые управляющие символы в строках и игнорирует всё после первого значения; при этом одиночный CR после
//! значения — ошибка. Компонента разбирает только строгий JSON (RFC 8259, без одиночного CR и сырых управляющих
//! символов в строках) и отвечает отказом (`None`) на всё прочее: такое тело разбирает BSL — он же даёт текст
//! ошибки, если тело неверно. Отказ и при вложенности глубже `MAX_DEPTH`, и при числе длиннее
//! `MAX_NUMBER_CHARS` символов в десятичной записи.

use crate::onec::Internal;
use crate::text::Unit;
use std::collections::HashMap;

/// Наибольшая вложенность массивов и объектов (корень — 1). Глубже — отказ в пользу BSL.
pub const MAX_DEPTH: usize = 64;

/// Наибольшая длина десятичной записи числа (1С раскрывает `1e400` в 401 цифру). Длиннее — отказ.
pub const MAX_NUMBER_CHARS: usize = 400;

/// Значение JSON. Ключи объекта — без повторов, в порядке первого появления; повтор ключа заменяет значение
/// (как `Вставить` у `ПрочитатьJSON`: позиция первого, значение последнего).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Нормализованная десятичная запись, как `ЗначениеВСтрокуВнутр` и `XMLСтрока` числа.
    Num(String),
    Str(Vec<Unit>),
    Arr(Vec<Json>),
    Obj(Vec<(Vec<Unit>, Json)>),
}

impl Json {
    /// `Соответствие.Получить(ключ)`.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(items) => {
                let key: Vec<Unit> = key.encode_utf16().collect();
                items.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
            }
            _ => None,
        }
    }

    /// Запись во внутреннем формате: объект — `Соответствие`, массив — `Массив`, null — `Неопределено`.
    pub fn write(&self, w: &mut Internal) {
        match self {
            Json::Null => w.undefined(),
            Json::Bool(b) => w.boolean(*b),
            Json::Num(n) => w.number(n),
            Json::Str(s) => w.string(s),
            Json::Arr(items) => w.array(items, |w, v| v.write(w)),
            Json::Obj(items) => w.map(items, |w, v| v.write(w)),
        }
    }
}

/// Тело записи → внутренний формат `Соответствие` или отказ (не строгий JSON, не объект, предел).
pub fn body_internal(text: &[Unit]) -> Option<String> {
    let value = parse_json(text)?;
    if !matches!(value, Json::Obj(_)) {
        return None;
    }
    let mut w = Internal::new();
    value.write(&mut w);
    Some(w.finish())
}

/// Строгий разбор всего текста: одно значение, вокруг — только пробелы JSON.
pub fn parse_json(text: &[Unit]) -> Option<Json> {
    let mut p = Parser { s: text, i: 0 };
    p.ws()?;
    let value = p.value(1)?;
    p.ws()?;
    if p.i == text.len() {
        Some(value)
    } else {
        None
    }
}

struct Parser<'a> {
    s: &'a [Unit],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Unit> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c as Unit) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// Пробелы JSON: пробел, табуляция, LF и CR только перед LF (одиночный CR 1С отвергает).
    fn ws(&mut self) -> Option<()> {
        while let Some(c) = self.peek() {
            match c {
                0x20 | 0x09 | 0x0A => self.i += 1,
                0x0D if self.s.get(self.i + 1) == Some(&0x0A) => self.i += 2,
                0x0D => return None,
                _ => break,
            }
        }
        Some(())
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        match self.peek()? {
            0x7B => self.object(depth),
            0x5B => self.array(depth),
            0x22 => self.string().map(Json::Str),
            0x74 => self.literal("true", Json::Bool(true)),
            0x66 => self.literal("false", Json::Bool(false)),
            0x6E => self.literal("null", Json::Null),
            0x2D | 0x30..=0x39 => self.number(),
            _ => None,
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Option<Json> {
        for b in word.bytes() {
            if !self.eat(b) {
                return None;
            }
        }
        Some(value)
    }

    fn object(&mut self, depth: usize) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.i += 1;
        let mut items: Vec<(Vec<Unit>, Json)> = Vec::new();
        let mut index: HashMap<Vec<Unit>, usize> = HashMap::new();
        self.ws()?;
        if self.eat(b'}') {
            return Some(Json::Obj(items));
        }
        loop {
            self.ws()?;
            if self.peek() != Some(0x22) {
                return None;
            }
            let key = self.string()?;
            self.ws()?;
            if !self.eat(b':') {
                return None;
            }
            self.ws()?;
            let value = self.value(depth + 1)?;
            match index.get(&key) {
                Some(&at) => items[at].1 = value,
                None => {
                    index.insert(key.clone(), items.len());
                    items.push((key, value));
                }
            }
            self.ws()?;
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Some(Json::Obj(items));
            }
            return None;
        }
    }

    fn array(&mut self, depth: usize) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.i += 1;
        let mut items = Vec::new();
        self.ws()?;
        if self.eat(b']') {
            return Some(Json::Arr(items));
        }
        loop {
            self.ws()?;
            items.push(self.value(depth + 1)?);
            self.ws()?;
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Some(Json::Arr(items));
            }
            return None;
        }
    }

    /// Строка в двойных кавычках. `\uXXXX` даёт единицу UTF-16 как есть (1С хранит и одиночные половины пар);
    /// сырые символы меньше U+0020 и сырая половина пары без пары — отказ.
    fn string(&mut self) -> Option<Vec<Unit>> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                0x22 => return Some(out),
                0x5C => {
                    let e = self.peek()?;
                    self.i += 1;
                    out.push(match e {
                        0x22 => 0x22,
                        0x5C => 0x5C,
                        0x2F => 0x2F,
                        0x62 => 0x08,
                        0x66 => 0x0C,
                        0x6E => 0x0A,
                        0x72 => 0x0D,
                        0x74 => 0x09,
                        0x75 => self.hex4()?,
                        _ => return None,
                    });
                }
                0x00..=0x1F => return None,
                0xD800..=0xDBFF => {
                    if !matches!(self.peek(), Some(0xDC00..=0xDFFF)) {
                        return None;
                    }
                    out.push(c);
                    out.push(self.s[self.i]);
                    self.i += 1;
                }
                0xDC00..=0xDFFF => return None,
                _ => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Option<Unit> {
        let mut v: Unit = 0;
        for _ in 0..4 {
            let c = self.peek()?;
            let d = match c {
                0x30..=0x39 => c - 0x30,
                0x41..=0x46 => c - 0x41 + 10,
                0x61..=0x66 => c - 0x61 + 10,
                _ => return None,
            };
            v = v * 16 + d;
            self.i += 1;
        }
        Some(v)
    }

    fn digits(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(c @ 0x30..=0x39) = self.peek() {
            out.push(c as u8);
            self.i += 1;
        }
        out
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
    fn number(&mut self) -> Option<Json> {
        let negative = self.eat(b'-');
        let int = self.digits();
        if int.is_empty() || (int.len() > 1 && int[0] == b'0') {
            return None;
        }
        let mut frac = Vec::new();
        if self.eat(b'.') {
            frac = self.digits();
            if frac.is_empty() {
                return None;
            }
        }
        let mut exp: i64 = 0;
        if self.eat(b'e') || self.eat(b'E') {
            let exp_negative = if self.eat(b'-') {
                true
            } else {
                self.eat(b'+');
                false
            };
            let digits = self.digits();
            if digits.is_empty() {
                return None;
            }
            for d in digits {
                exp = exp.checked_mul(10)?.checked_add((d - b'0') as i64)?;
                if exp > 1_000_000 {
                    return None;
                }
            }
            if exp_negative {
                exp = -exp;
            }
        }
        normalize_number(negative, &int, &frac, exp).map(Json::Num)
    }
}

/// Десятичная запись числа, как её пишут `ЗначениеВСтрокуВнутр` и `XMLСтрока`: без показателя, без ведущих нулей
/// целой части и хвостовых нулей дробной, `-0` → `0`, `1e-5` → `0.00001`. Длиннее `MAX_NUMBER_CHARS` — `None`.
pub fn normalize_number(negative: bool, int: &[u8], frac: &[u8], exp: i64) -> Option<String> {
    let mut digits: Vec<u8> = int.iter().chain(frac).copied().collect();
    // Число = 0.digits × 10^point.
    let mut point = int.len() as i64 + exp;
    let lead = digits.iter().take_while(|d| **d == b'0').count();
    digits.drain(..lead);
    point -= lead as i64;
    while digits.last() == Some(&b'0') {
        digits.pop();
    }
    if digits.is_empty() {
        return Some("0".to_string());
    }
    let n = digits.len() as i64;
    let len = if point <= 0 {
        2 - point + n
    } else if point >= n {
        point
    } else {
        n + 1
    } + negative as i64;
    if len > MAX_NUMBER_CHARS as i64 {
        return None;
    }
    let mut out = String::with_capacity(len as usize);
    if negative {
        out.push('-');
    }
    let digits = std::str::from_utf8(&digits).ok()?;
    if point <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-point) as usize));
        out.push_str(digits);
    } else if point >= n {
        out.push_str(digits);
        out.extend(std::iter::repeat_n('0', (point - n) as usize));
    } else {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::units;

    fn num(s: &str) -> Option<String> {
        match parse_json(&units(s))? {
            Json::Num(n) => Some(n),
            _ => None,
        }
    }

    #[test]
    fn numbers_like_1c() {
        for (text, expected) in [
            ("-0", "0"),
            ("1e5", "100000"),
            ("1E+5", "100000"),
            ("-3E-2", "-0.03"),
            ("0.1", "0.1"),
            ("123.4500", "123.45"),
            ("1e-5", "0.00001"),
            ("1.5e1", "15"),
            ("100e-2", "1"),
            ("-0.0", "0"),
            ("0e0", "0"),
            ("-1.0e-0", "-1"),
            (
                "12345678901234567890123456789012345678901234567890",
                "12345678901234567890123456789012345678901234567890",
            ),
            ("1e30", "1000000000000000000000000000000"),
            ("1e-30", "0.000000000000000000000000000001"),
        ] {
            assert_eq!(num(text).as_deref(), Some(expected), "{text}");
        }
        for text in ["01", "1.", ".5", "+1", "-", "1e", "1e+", "NaN", "1e400", "1e-400", "1e99999999999999999999"] {
            assert_eq!(num(text), None, "{text}");
        }
        assert_eq!(num("1e398").map(|n| n.len()), Some(399));
    }

    #[test]
    fn strict_json_only() {
        for text in [
            "{\"a\":1,}",
            "[1,]",
            "['a']",
            "[1 2]",
            "{\"a\":1} x",
            "{a:1}",
            "[\"\\x\"]",
            "[\"a\tb\"]",
            "[1]\r",
            "\r[1]",
            "[\"\\u00\"]",
            "[True]",
            "",
            " ",
            "[\u{a0}1]",
        ] {
            assert_eq!(parse_json(&units(text)), None, "{text:?}");
        }
        for text in ["[1]\r\n", "\t\n [1] ", "{\"a\" \t: 1 }", "[]", "{}", "\"x\""] {
            assert!(parse_json(&units(text)).is_some(), "{text:?}");
        }
    }

    #[test]
    fn strings_keep_utf16_units() {
        let parsed = parse_json(&units("[\"\\ud800\\uD83D\\uDE00😀\\b\\f\\/\"]")).unwrap();
        assert_eq!(parsed, Json::Arr(vec![Json::Str(vec![0xD800, 0xD83D, 0xDE00, 0xD83D, 0xDE00, 8, 12, 0x2F])]));
        assert_eq!(parse_json(&[0x22, 0xD800, 0x22]), None);
        assert_eq!(parse_json(&[0x22, 0xDC00, 0x22]), None);
    }

    #[test]
    fn duplicate_key_keeps_first_position_last_value() {
        let parsed = parse_json(&units("{\"z\":1,\"a\":2,\"z\":3}")).unwrap();
        assert_eq!(parsed, Json::Obj(vec![(units("z"), Json::Num("3".into())), (units("a"), Json::Num("2".into()))]));
    }

    #[test]
    fn depth_limit() {
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse_json(&units(&ok)).is_some());
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert_eq!(parse_json(&units(&deep)), None);
        assert_eq!(parse_json(&units(&"[".repeat(100_000))), None);
    }

    #[test]
    fn body_is_object() {
        assert_eq!(body_internal(&units("[1]")), None);
        assert_eq!(
            body_internal(&units("{\"a\":null,\"b\":[true]}")).unwrap(),
            "{\"#\",3d48feae-a9c6-4c5a-a099-9eb6477630c6,\n{2,\n{\n{\"S\",\"a\"},\n{\"U\"}\n},\n{\n{\"S\",\"b\"},\n\
             {\"#\",51e7a0d2-530b-11d4-b98a-008048da3034,\n{1,\n{\"B\",1}\n}\n}\n}\n}\n}"
        );
    }
}
