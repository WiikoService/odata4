//! Текст параметра — единицы UTF-16, как строка 1С: позиции с 1, «символ за концом текста» — `None`
//! (у 1С `Сред` за концом даёт пустую строку). Классы символов повторяют `OData4_Разбор`.

use crate::letters::{LETTERS, LOWER_ASCII, UPPER_ASCII};

/// Единица текста 1С: `СтрДлина` и `Сред` считают единицы UTF-16 (проба Task 1, Step 1). Если бы 1С
/// считала символы Unicode, менялись бы только `Unit`, `units`, `string` и `char_at`.
pub type Unit = u16;

/// Строка Rust → единицы UTF-16 (как `СтрДлина`/`Сред` 1С).
pub fn units(s: &str) -> Vec<Unit> {
    s.encode_utf16().collect()
}

/// Единицы UTF-16 → строка Rust; одиночная половина суррогатной пары → U+FFFD.
pub fn string(u: &[Unit]) -> String {
    String::from_utf16_lossy(u)
}

/// Символ для текста ошибки с позиции `pos`: суррогатная пара — целиком (две единицы), иначе одна единица.
pub fn char_at(t: &[Unit], pos: usize) -> &[Unit] {
    let is_pair = matches!(at(t, pos), Some(0xD800..=0xDBFF)) && matches!(at(t, pos + 1), Some(0xDC00..=0xDFFF));
    &t[pos - 1..pos - 1 + if is_pair { 2 } else { 1 }]
}

/// `Сред(Текст, pos, 1)`: единица в позиции `pos` (с 1) или `None` за пределами текста.
pub fn at(t: &[Unit], pos: usize) -> Option<Unit> {
    if pos >= 1 && pos <= t.len() {
        Some(t[pos - 1])
    } else {
        None
    }
}

/// Единица равна ASCII-символу `ch`.
pub fn is(c: Option<Unit>, ch: u8) -> bool {
    c == Some(ch as Unit)
}

pub fn is_digit(c: Option<Unit>) -> bool {
    matches!(c, Some(u) if (0x30..=0x39).contains(&u))
}

pub fn is_hex(c: Option<Unit>) -> bool {
    matches!(c, Some(u) if u < 128 && (u as u8).is_ascii_hexdigit())
}

/// `ЭтоБуква`: `ВРег(С) <> НРег(С)` — по таблице, снятой с 1С (`letters.rs`).
pub fn is_letter(c: Option<Unit>) -> bool {
    match c {
        Some(u) => LETTERS
            .binary_search_by(|&(lo, hi)| {
                if hi < u {
                    std::cmp::Ordering::Less
                } else if lo > u {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok(),
        None => false,
    }
}

pub fn is_name_start(c: Option<Unit>) -> bool {
    is_letter(c) || is(c, b'_') || is(c, b'$')
}

pub fn is_name_char(c: Option<Unit>) -> bool {
    is_letter(c) || is_digit(c) || is(c, b'_') || is(c, b'.')
}

/// Единицы совпадают с ASCII-строкой `s`.
pub fn eq_ascii(u: &[Unit], s: &str) -> bool {
    u.len() == s.len() && u.iter().zip(s.bytes()).all(|(a, b)| *a == b as Unit)
}

pub fn starts_with_dollar(u: &[Unit]) -> bool {
    u.first() == Some(&(b'$' as Unit))
}

/// `СовпадаетСШаблоном`: `0` — цифра, `h` — шестнадцатеричная, прочее — как есть (шаблон ASCII).
pub fn matches_pattern(t: &[Unit], pos: usize, pattern: &str) -> bool {
    if pos == 0 || pos > t.len() + 1 || t.len() + 1 - pos < pattern.len() {
        return false;
    }
    pattern.bytes().enumerate().all(|(i, p)| {
        let c = at(t, pos + i);
        match p {
            b'0' => is_digit(c),
            b'h' => is_hex(c),
            _ => is(c, p),
        }
    })
}

/// `ЧислоЦифр`: сколько цифр подряд с позиции `pos`.
pub fn count_digits(t: &[Unit], pos: usize) -> usize {
    let mut n = 0;
    while is_digit(at(t, pos + n)) {
        n += 1;
    }
    n
}

/// Значение цифр (`Число` над проверенными цифрами); прочие единицы не учитываются.
pub fn digits_value(u: &[Unit]) -> u64 {
    u.iter()
        .filter(|c| (0x30..=0x39).contains(*c))
        .fold(0u64, |acc, c| acc.saturating_mul(10).saturating_add((*c - 0x30) as u64))
}

/// `НРег` для сравнения с ключевыми словами ASCII: ASCII — как обычно, прочие — по таблице 1С
/// (символы, у которых `НРег` — один символ ASCII); остальные остаются как есть и ни с чем не совпадут.
pub fn lower_keyword(u: &[Unit]) -> Vec<Unit> {
    u.iter()
        .map(|&c| {
            if c < 128 {
                (c as u8).to_ascii_lowercase() as Unit
            } else {
                match LOWER_ASCII.binary_search_by_key(&c, |&(k, _)| k) {
                    Ok(i) => LOWER_ASCII[i].1 as Unit,
                    Err(_) => c,
                }
            }
        })
        .collect()
}

/// `ВРег` одной единицы, если результат — один символ ASCII (для суффиксов чисел M, L, D, F).
pub fn upper_ascii(c: Option<Unit>) -> Option<u8> {
    let c = c?;
    if c < 128 {
        return Some((c as u8).to_ascii_uppercase());
    }
    UPPER_ASCII.binary_search_by_key(&c, |&(k, _)| k).ok().map(|i| UPPER_ASCII[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(ch: char) -> Option<Unit> {
        Some(ch as Unit)
    }

    #[test]
    fn letters_and_names() {
        for ch in ['A', 'z', 'Я', 'ё', 'Ё'] {
            assert!(is_letter(c(ch)), "{ch}");
        }
        for ch in ['0', '_', ' ', '$', '.', '\''] {
            assert!(!is_letter(c(ch)), "{ch}");
        }
        assert!(!is_letter(None));
        assert!(!is_letter(Some(0xD83D)));
        assert!(is_name_start(c('$')) && is_name_start(c('_')) && !is_name_start(c('1')));
        assert!(is_name_char(c('.')) && is_name_char(c('7')) && !is_name_char(c('-')));
    }

    #[test]
    fn positions_are_utf16_units() {
        let t = units("a😀b");
        assert_eq!(t.len(), 4);
        assert_eq!(at(&t, 4), Some(b'b' as Unit));
        assert_eq!(at(&t, 0), None);
        assert_eq!(at(&t, 5), None);
        assert_eq!(string(char_at(&t, 2)), "😀");
        assert_eq!(string(char_at(&t, 4)), "b");
        assert_eq!(char_at(&[0xD83D], 1), &[0xD83D]);
    }

    #[test]
    fn patterns_and_digits() {
        let t = units("x2024-01-31");
        assert!(matches_pattern(&t, 2, "0000-00-00"));
        assert!(!matches_pattern(&t, 3, "0000-00-00"));
        assert!(!matches_pattern(&t, 13, "0"));
        assert!(matches_pattern(&units("aF09"), 1, "hhhh"));
        assert_eq!(count_digits(&units("12a"), 1), 2);
        assert_eq!(digits_value(&units("007")), 7);
    }

    #[test]
    fn keyword_case() {
        assert_eq!(lower_keyword(&units("GUID")), units("guid"));
        assert_eq!(upper_ascii(c('m')), Some(b'M'));
        assert_eq!(upper_ascii(None), None);
        assert!(eq_ascii(&units("eq"), "eq") && !eq_ascii(&units("EQ"), "eq"));
    }
}
