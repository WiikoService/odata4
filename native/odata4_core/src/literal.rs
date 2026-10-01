//! Литералы: даты, время, числа, типизированные `префикс'…'` — как в `OData4_Разбор` (область «Лексер»).

use crate::error::{bad_literal, ParseError};
use crate::text::{
    at, count_digits, digits_value, eq_ascii, is, is_digit, is_hex, is_letter, is_name_start, lower_keyword,
    matches_pattern, string, Unit,
};

/// Узел `literal`: `ty` — `null`, `Edm.*` или тип перечисления; `value` — текст или `None` у `null`.
#[derive(Clone, Debug, PartialEq)]
pub struct Literal {
    pub ty: String,
    pub value: Option<String>,
}

impl Literal {
    pub fn new(ty: &str, value: impl Into<String>) -> Literal {
        Literal { ty: ty.to_string(), value: Some(value.into()) }
    }

    pub fn null() -> Literal {
        Literal { ty: "null".to_string(), value: None }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self.ty.as_str(), "Edm.Int64" | "Edm.Decimal" | "Edm.Double" | "Edm.Single")
    }
}

/// `ЭтоГУИД`: 36 символов по шаблону, следом не символ имени.
pub fn is_guid(t: &[Unit], pos: usize) -> bool {
    matches_pattern(t, pos, "hhhhhhhh-hhhh-hhhh-hhhh-hhhhhhhhhhhh") && !crate::text::is_name_char(at(t, pos + 36))
}

/// `ДлинаДолей`: длина «.ddd» с позиции `pos`; 0 — долей нет.
fn fraction_len(t: &[Unit], pos: usize) -> usize {
    if !is(at(t, pos), b'.') {
        return 0;
    }
    let n = count_digits(t, pos + 1);
    if n > 0 {
        n + 1
    } else {
        0
    }
}

/// `ДлинаДатыВремени`: 10 — дата, больше — дата и время с необязательными секундами, долями и зоной; 0 — не дата.
pub fn datetime_len(t: &[Unit], pos: usize) -> usize {
    if !matches_pattern(t, pos, "0000-00-00") {
        return 0;
    }
    if !matches_pattern(t, pos + 10, "T00:00") {
        return 10;
    }
    let mut len = 16;
    if matches_pattern(t, pos + len, ":00") {
        len += 3;
        len += fraction_len(t, pos + len);
    }
    if is(at(t, pos + len), b'Z') {
        len += 1;
    } else if matches_pattern(t, pos + len, "+00:00") || matches_pattern(t, pos + len, "-00:00") {
        len += 6;
    }
    len
}

/// `ДлинаВремени`: hh:mm[:ss[.fff]]; 0 — не время.
pub fn time_len(t: &[Unit], pos: usize) -> usize {
    if !matches_pattern(t, pos, "00:00") {
        return 0;
    }
    let mut len = 5;
    if matches_pattern(t, pos + len, ":00") {
        len += 3;
        len += fraction_len(t, pos + len);
    }
    len
}

/// `ЕстьЗона`: последний символ `Z` или (длина больше 16 и шестой с конца — `+`/`-`).
pub fn has_zone(s: &[Unit]) -> bool {
    let n = s.len();
    is(s.last().copied(), b'Z') || (n > 16 && (is(at(s, n - 5), b'+') || is(at(s, n - 5), b'-')))
}

fn number(s: &[Unit], from: usize, len: usize) -> u64 {
    digits_value(s.get(from..(from + len).min(s.len())).unwrap_or(&[]))
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        2 => {
            if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `ПроверитьВремя`: часы 0–23, минуты и секунды 0–59.
pub fn check_time(value: &[Unit], raw: &[Unit], pos: usize, param: &str) -> Result<(), ParseError> {
    let hours = number(value, 0, 2);
    let minutes = number(value, 3, 2);
    let seconds = if is(at(value, 6), b':') { number(value, 6, 2) } else { 0 };
    if hours > 23 || minutes > 59 || seconds > 59 {
        return Err(bad_literal(param, raw, pos));
    }
    Ok(())
}

/// `ПроверитьДатуВремя`: календарь (год от 1), время, смещение зоны (часы 0–14, минуты 0–59).
pub fn check_datetime(value: &[Unit], raw: &[Unit], pos: usize, param: &str) -> Result<(), ParseError> {
    let year = number(value, 0, 4);
    let month = number(value, 5, 2);
    let day = number(value, 8, 2);
    if year < 1 || !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return Err(bad_literal(param, raw, pos));
    }
    if value.len() > 10 {
        check_time(&value[11.min(value.len())..], raw, pos, param)?;
    }
    let n = value.len();
    if has_zone(value)
        && !is(value.last().copied(), b'Z')
        && (number(value, n - 5, 2) > 14 || number(value, n - 2, 2) > 59)
    {
        return Err(bad_literal(param, raw, pos));
    }
    Ok(())
}

/// `ТипЦелого`: `Edm.Int64` в диапазоне Int64 (с минусом — до 9223372036854775808), иначе `Edm.Decimal`
/// до 29 значащих цифр, иначе `None`.
pub fn integer_type(digits: &[Unit], negative: bool) -> Option<&'static str> {
    let mut significant = digits;
    while significant.len() > 1 && significant[0] == b'0' as Unit {
        significant = &significant[1..];
    }
    if significant.len() > 29 {
        return None;
    }
    let value: u128 = significant.iter().fold(0u128, |acc, c| acc * 10 + (*c as u128 - 0x30));
    let limit: u128 = if negative { 9_223_372_036_854_775_808 } else { 9_223_372_036_854_775_807 };
    Some(if value <= limit { "Edm.Int64" } else { "Edm.Decimal" })
}

fn is_hex_text(s: &[Unit]) -> bool {
    !s.is_empty() && s.len().is_multiple_of(2) && s.iter().all(|&c| is_hex(Some(c)))
}

const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_index(c: Unit) -> Option<u32> {
    if c >= 128 {
        return None;
    }
    BASE64URL.iter().position(|&b| b as Unit == c).map(|i| i as u32)
}

/// `ЭтоBase64URL` (RFC 4648 §5): до двух `=` только в конце и только до длины, кратной 4; длина без `=`
/// не даёт остаток 1; неиспользуемые биты последнего символа — нули. Возвращает данные без `=`.
fn base64url_data(s: &[Unit]) -> Option<&[Unit]> {
    let mut data = s;
    let mut padding = 0;
    while is(data.last().copied(), b'=') {
        data = &data[..data.len() - 1];
        padding += 1;
    }
    let len = data.len();
    if len == 0 || len % 4 == 1 || padding > 2 || (padding > 0 && !(len + padding).is_multiple_of(4)) {
        return None;
    }
    if !data.iter().all(|&c| base64url_index(c).is_some()) {
        return None;
    }
    let last = base64url_index(data[len - 1])?;
    if (len % 4 == 2 && last % 16 != 0) || (len % 4 == 3 && last % 4 != 0) {
        return None;
    }
    Some(data)
}

/// Проверенный base64url → шестнадцатеричные цифры в верхнем регистре.
fn base64url_hex(data: &[Unit]) -> String {
    let mut bits: u32 = 0;
    let mut count = 0;
    let mut hex = String::new();
    for &c in data {
        bits = ((bits << 6) | base64url_index(c).unwrap_or(0)) & 0xFFFF;
        count += 6;
        if count >= 8 {
            count -= 8;
            hex.push_str(&format!("{:02X}", (bits >> count) & 0xFF));
        }
    }
    hex
}

/// `ЭтоДлительность`: [+|-]P[nD][T[nH][nM][n[.n]S]], хотя бы одна часть; после T — хотя бы одна часть времени.
fn is_duration(s: &[Unit]) -> bool {
    let mut pos = if is(at(s, 1), b'-') || is(at(s, 1), b'+') { 2 } else { 1 };
    if !is(at(s, pos), b'P') {
        return false;
    }
    pos += 1;
    let mut parts = 0;
    let n = count_digits(s, pos);
    if n > 0 && is(at(s, pos + n), b'D') {
        pos += n + 1;
        parts += 1;
    }
    if is(at(s, pos), b'T') {
        pos += 1;
        let mut time_parts = 0;
        for unit in *b"HM" {
            let n = count_digits(s, pos);
            if n > 0 && is(at(s, pos + n), unit) {
                pos += n + 1;
                time_parts += 1;
            }
        }
        let mut n = count_digits(s, pos);
        if n > 0 {
            n += fraction_len(s, pos + n);
        }
        if n > 0 && is(at(s, pos + n), b'S') {
            pos += n + 1;
            time_parts += 1;
        }
        if time_parts == 0 {
            return false;
        }
        parts += time_parts;
    }
    parts > 0 && pos == s.len() + 1
}

/// `ЭтоИмяЧлена`: начало имени без `$`, далее буквы, цифры, `_`.
fn is_member_name(s: &[Unit]) -> bool {
    let first = at(s, 1);
    if !is_name_start(first) || is(first, b'$') {
        return false;
    }
    s[1..].iter().all(|&c| {
        let c = Some(c);
        is_letter(c) || is_digit(c) || is(c, b'_')
    })
}

/// `ТипизированныйЛитерал`: guid, datetime, datetimeoffset, time, X, binary, duration, `Пространство.Тип'Член'`.
pub fn typed_literal(
    prefix: &[Unit],
    content: &[Unit],
    raw: &[Unit],
    pos: usize,
    param: &str,
) -> Result<Literal, ParseError> {
    let kind = lower_keyword(prefix);
    let len = content.len();
    if eq_ascii(&kind, "guid") && len == 36 && is_guid(content, 1) {
        return Ok(Literal::new("Edm.Guid", string(content).to_ascii_lowercase()));
    }
    if eq_ascii(&kind, "datetime") && datetime_len(content, 1) == len && len > 10 && !has_zone(content) {
        check_datetime(content, raw, pos, param)?;
        return Ok(Literal::new("Edm.DateTime", string(content)));
    }
    if eq_ascii(&kind, "datetimeoffset") && datetime_len(content, 1) == len && len > 10 && has_zone(content) {
        check_datetime(content, raw, pos, param)?;
        return Ok(Literal::new("Edm.DateTimeOffset", string(content)));
    }
    if eq_ascii(&kind, "time") && time_len(content, 1) == len && len > 0 {
        check_time(content, raw, pos, param)?;
        return Ok(Literal::new("Edm.TimeOfDay", string(content)));
    }
    if (eq_ascii(&kind, "x") || eq_ascii(&kind, "binary")) && is_hex_text(content) {
        return Ok(Literal::new("Edm.Binary", string(content).to_ascii_uppercase()));
    }
    if eq_ascii(&kind, "binary") {
        if let Some(data) = base64url_data(content) {
            return Ok(Literal::new("Edm.Binary", base64url_hex(data)));
        }
    }
    if eq_ascii(&kind, "duration") && is_duration(content) {
        return Ok(Literal::new("Edm.Duration", string(content)));
    }
    if prefix.contains(&(b'.' as Unit)) && is_member_name(content) {
        return Ok(Literal { ty: string(prefix), value: Some(string(content)) });
    }
    Err(bad_literal(param, raw, pos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::units;

    #[test]
    fn integer_ranges() {
        assert_eq!(integer_type(&units("9223372036854775807"), false), Some("Edm.Int64"));
        assert_eq!(integer_type(&units("9223372036854775808"), false), Some("Edm.Decimal"));
        assert_eq!(integer_type(&units("9223372036854775808"), true), Some("Edm.Int64"));
        assert_eq!(integer_type(&units("0000000000000000000000000000001"), false), Some("Edm.Int64"));
        assert_eq!(integer_type(&units("123456789012345678901234567890"), false), None);
    }

    #[test]
    fn durations() {
        for ok in ["P1D", "-P1DT2H3M4.5S", "PT1S", "+PT0.5S", "PT1H"] {
            assert!(is_duration(&units(ok)), "{ok}");
        }
        for bad in ["", "P", "PT", "P1Y", "garbage", "P1DT", "PT1.S", "P1D1H"] {
            assert!(!is_duration(&units(bad)), "{bad}");
        }
    }

    #[test]
    fn base64url() {
        assert_eq!(base64url_data(&units("Cv8=")).map(base64url_hex), Some("0AFF".to_string()));
        assert_eq!(base64url_data(&units("Cw==")).map(base64url_hex), Some("0B".to_string()));
        assert_eq!(base64url_data(&units("Cv8")).map(base64url_hex), Some("0AFF".to_string()));
        for bad in ["A", "==", "Cv9=", "Cv8===", "C=v8", "Cv+8"] {
            assert!(base64url_data(&units(bad)).is_none(), "{bad}");
        }
    }

    #[test]
    fn zones_and_lengths() {
        assert!(has_zone(&units("2024-01-31T10:20:30Z")));
        assert!(has_zone(&units("2024-01-31T10:20-03:00")));
        assert!(!has_zone(&units("2024-01-31T10:20")));
        assert_eq!(datetime_len(&units("2024-01-31T10:20:30.123+03:00"), 1), 29);
        assert_eq!(datetime_len(&units("2024-01-31T1"), 1), 10);
        assert_eq!(time_len(&units("13:20:00.5"), 1), 10);
        assert_eq!(time_len(&units("13:2"), 1), 0);
    }
}
