//! Лексер — как `OData4_Разбор.Лексемы`: виды Имя, Строка, Литерал, Знак, Конец; позиции с 1 в единицах UTF-16.
//! Ошибка лексера прерывает разбор сразу (у 1С — исключение до начала разбора).

use crate::error::{bad_literal, ParseError};
use crate::literal::{
    check_datetime, check_time, datetime_len, has_zone, integer_type, is_guid, time_len, typed_literal, Literal,
};
use crate::text::{at, char_at, is, is_digit, is_name_char, is_name_start, string, upper_ascii, Unit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Name,
    Str,
    Literal,
    Sign,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Str(Vec<Unit>),
    Literal(Literal),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: Kind,
    /// Текст лексемы как в исходном тексте (у строки — с кавычками).
    pub text: Vec<Unit>,
    pub value: Value,
    /// Позиция первого символа, с 1; у `End` — длина текста + 1.
    pub pos: usize,
}

impl Token {
    fn new(kind: Kind, text: &[Unit], value: Value, pos: usize) -> Token {
        Token { kind, text: text.to_vec(), value, pos }
    }
}

const SIGNS: &[u8] = b"(),/:=;*-";

/// Лексемы текста `t` параметра `param` (имя параметра нужно для текстов ошибок).
pub fn tokens(t: &[Unit], param: &str) -> Result<Vec<Token>, ParseError> {
    let n = t.len();
    let mut out = Vec::new();
    let mut pos = 1;
    while pos <= n {
        let c = at(t, pos);
        if is(c, b' ') || is(c, b'\t') || is(c, b'\n') || is(c, b'\r') {
            pos += 1;
        } else if is(c, b'\'') {
            let end = string_end(t, pos, param)?;
            out.push(Token::new(Kind::Str, &t[pos - 1..end], Value::Str(string_content(t, pos, end)), pos));
            pos = end + 1;
        } else if is_guid(t, pos) {
            let raw = &t[pos - 1..pos + 35];
            let literal = Literal::new("Edm.Guid", string(raw).to_ascii_lowercase());
            out.push(Token::new(Kind::Literal, raw, Value::Literal(literal), pos));
            pos += 36;
        } else if is_digit(c) {
            pos = number_literal(t, pos, param, &mut out)?;
        } else if is_name_start(c) {
            let mut end = pos;
            while end < n && is_name_char(at(t, end + 1)) {
                end += 1;
            }
            let name = &t[pos - 1..end];
            if is(at(t, end + 1), b'\'') {
                let close = string_end(t, end + 1, param)?;
                let raw = &t[pos - 1..close];
                let literal = typed_literal(name, &string_content(t, end + 1, close), raw, pos, param)?;
                out.push(Token::new(Kind::Literal, raw, Value::Literal(literal), pos));
                pos = close + 1;
            } else {
                out.push(Token::new(Kind::Name, name, Value::None, pos));
                pos = end + 1;
            }
        } else if matches!(c, Some(u) if u < 128 && SIGNS.contains(&(u as u8))) {
            out.push(Token::new(Kind::Sign, &t[pos - 1..pos], Value::None, pos));
            pos += 1;
        } else {
            return Err(ParseError::bad_request(format!(
                "{param}: недопустимый символ «{}» в позиции {pos}",
                string(char_at(t, pos))
            )));
        }
    }
    out.push(Token::new(Kind::End, &[], Value::None, n + 1));
    Ok(out)
}

/// `КонецСтроки`: позиция закрывающей кавычки строки, открытой в `start`; `''` внутри — кавычка.
fn string_end(t: &[Unit], start: usize, param: &str) -> Result<usize, ParseError> {
    let mut pos = start + 1;
    while pos <= t.len() {
        if is(at(t, pos), b'\'') {
            if is(at(t, pos + 1), b'\'') {
                pos += 2;
                continue;
            }
            return Ok(pos);
        }
        pos += 1;
    }
    Err(ParseError::bad_request(format!("{param}: незакрытая строка в позиции {start}")))
}

/// `СодержимоеСтроки`: текст между кавычками `start` и `end`, `''` → `'`.
fn string_content(t: &[Unit], start: usize, end: usize) -> Vec<Unit> {
    let inner = &t[start..end - 1];
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        out.push(inner[i]);
        if inner[i] == b'\'' as Unit && inner.get(i + 1) == Some(&(b'\'' as Unit)) {
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// `ПроверитьКонецЛитерала`: литерал не продолжается символом имени («12abc», «2024-01-31and»).
fn check_literal_end(t: &[Unit], raw: &[Unit], pos: usize, param: &str) -> Result<(), ParseError> {
    let next = at(t, pos + raw.len());
    if is_name_char(next) {
        let mut shown = raw.to_vec();
        shown.extend(next);
        return Err(bad_literal(param, &shown, pos));
    }
    Ok(())
}

/// `ДобавитьЧисловойЛитерал`: дата, дата и время, время или число с позиции `pos`; возвращает позицию после литерала.
fn number_literal(t: &[Unit], pos: usize, param: &str, out: &mut Vec<Token>) -> Result<usize, ParseError> {
    let len = datetime_len(t, pos);
    if len > 0 {
        let raw = &t[pos - 1..pos - 1 + len];
        let ty = if len == 10 {
            "Edm.Date"
        } else if has_zone(raw) {
            "Edm.DateTimeOffset"
        } else {
            "Edm.DateTime"
        };
        check_literal_end(t, raw, pos, param)?;
        check_datetime(raw, raw, pos, param)?;
        out.push(Token::new(Kind::Literal, raw, Value::Literal(Literal::new(ty, string(raw))), pos));
        return Ok(pos + len);
    }
    let len = time_len(t, pos);
    if len > 0 {
        let raw = &t[pos - 1..pos - 1 + len];
        check_literal_end(t, raw, pos, param)?;
        check_time(raw, raw, pos, param)?;
        out.push(Token::new(Kind::Literal, raw, Value::Literal(Literal::new("Edm.TimeOfDay", string(raw))), pos));
        return Ok(pos + len);
    }
    let n = t.len();
    let mut end = pos;
    while end < n && is_digit(at(t, end + 1)) {
        end += 1;
    }
    let mut ty = "Edm.Int64";
    if is(at(t, end + 1), b'.') && is_digit(at(t, end + 2)) {
        ty = "Edm.Decimal";
        end += 2;
        while end < n && is_digit(at(t, end + 1)) {
            end += 1;
        }
    }
    let e = at(t, end + 1);
    let sign = at(t, end + 2);
    if (is(e, b'e') || is(e, b'E'))
        && (is_digit(sign) || ((is(sign, b'+') || is(sign, b'-')) && is_digit(at(t, end + 3))))
    {
        ty = "Edm.Double";
        end += 2;
        while end < n && is_digit(at(t, end + 1)) {
            end += 1;
        }
    }
    let value = &t[pos - 1..end];
    let suffix = match upper_ascii(at(t, end + 1)) {
        Some(b'M') => {
            ty = "Edm.Decimal";
            Some(b'M')
        }
        Some(b'L') if ty == "Edm.Int64" => Some(b'L'),
        Some(b'D') => {
            ty = "Edm.Double";
            Some(b'D')
        }
        Some(b'F') => {
            ty = "Edm.Single";
            Some(b'F')
        }
        _ => None,
    };
    if suffix.is_some() {
        end += 1;
    }
    let raw = &t[pos - 1..end];
    check_literal_end(t, raw, pos, param)?;
    if ty == "Edm.Int64" {
        match integer_type(value, false) {
            Some(checked) if !(suffix == Some(b'L') && checked != "Edm.Int64") => ty = checked,
            _ => return Err(bad_literal(param, raw, pos)),
        }
    }
    let literal = Literal::new(ty, string(value).to_ascii_lowercase());
    out.push(Token::new(Kind::Literal, raw, Value::Literal(literal), pos));
    Ok(end + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::units;

    fn describe(text: &str) -> String {
        let list = tokens(&units(text), "$filter").unwrap();
        list.iter().map(|t| format!("{:?}:{}@{}", t.kind, string(&t.text), t.pos)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn same_tokens_as_1c() {
        // Тот же пример, что в OData4_ТестРазбор.ЛексемыИПозиции.
        assert_eq!(
            describe("Имя eq 'a''b' and guid'CC8BE316-CCF8-11E2-9747-0019D1B09843'"),
            "Name:Имя@1 Name:eq@5 Str:'a''b'@8 Name:and@15 Literal:guid'CC8BE316-CCF8-11E2-9747-0019D1B09843'@19 End:@61"
        );
        let list = tokens(&units("'a''b'"), "$filter").unwrap();
        assert_eq!(list[0].value, Value::Str(units("a'b")));
    }

    #[test]
    fn numbers() {
        let lit = |text: &str| match &tokens(&units(text), "$filter").unwrap()[0].value {
            Value::Literal(l) => (l.ty.clone(), l.value.clone().unwrap()),
            other => panic!("{other:?}"),
        };
        assert_eq!(lit("1E-3"), ("Edm.Double".into(), "1e-3".into()));
        assert_eq!(lit("2.5d"), ("Edm.Double".into(), "2.5".into()));
        assert_eq!(lit("5L"), ("Edm.Int64".into(), "5".into()));
        assert_eq!(lit("1.5M"), ("Edm.Decimal".into(), "1.5".into()));
        assert_eq!(lit("2.5f"), ("Edm.Single".into(), "2.5".into()));
    }

    #[test]
    fn errors() {
        let err = |text: &str| tokens(&units(text), "$filter").unwrap_err().message;
        assert_eq!(err("1.5L"), "$filter: неверный литерал 1.5L в позиции 1");
        assert_eq!(err("'😀' 1x"), "$filter: неверный литерал 1x в позиции 6");
        assert_eq!(err("a #"), "$filter: недопустимый символ «#» в позиции 3");
        assert_eq!(err("a 😀"), "$filter: недопустимый символ «😀» в позиции 3");
        // Одиночная половина пары (из HTTP не приходит): позиция та же, символ — U+FFFD.
        let lone = tokens(&[b'a' as Unit, b' ' as Unit, 0xDC00], "$filter").unwrap_err().message;
        assert_eq!(lone, "$filter: недопустимый символ «\u{fffd}» в позиции 3");
    }
}
