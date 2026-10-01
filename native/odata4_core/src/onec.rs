//! Правила строк 1С для разбора тел (этап 8.2) и запись значений во внутреннем формате 1С
//! (`ЗначениеИзСтрокиВнутр`). Всё снято с платформы 8.3.27 (`docs/probe-results.md`, «Проба этапа 8», п. 5).

use crate::text::{lower_keyword, Unit};

/// Пробельные символы `СокрЛП`, `СокрП` и `ПустаяСтрока` (снято перебором всех символов BMP). Не совпадает с
/// `char::is_whitespace`: у 1С есть 28–31 и U+180E.
pub fn is_space(u: Unit) -> bool {
    matches!(
        u,
        9..=13
            | 28..=32
            | 0x85
            | 0xA0
            | 0x1680
            | 0x180E
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

/// `СокрЛП`.
pub fn trim(u: &[Unit]) -> &[Unit] {
    trim_end(trim_start(u))
}

pub fn trim_start(u: &[Unit]) -> &[Unit] {
    let start = u.iter().position(|c| !is_space(*c)).unwrap_or(u.len());
    &u[start..]
}

/// `СокрП`.
pub fn trim_end(u: &[Unit]) -> &[Unit] {
    let end = u.iter().rposition(|c| !is_space(*c)).map_or(0, |i| i + 1);
    &u[..end]
}

/// `ПустаяСтрока`.
pub fn is_blank(u: &[Unit]) -> bool {
    u.iter().all(|c| is_space(*c))
}

/// `НРег(u) = s`, где `s` — строчные ASCII.
pub fn lower_eq(u: &[Unit], s: &str) -> bool {
    crate::text::eq_ascii(&lower_keyword(u), s)
}

/// `ВРег` всей строки, если в ней только ASCII; иначе `None` (таблицы `ВРег` для прочих символов нет — отказ).
pub fn upper_all_ascii(u: &[Unit]) -> Option<Vec<Unit>> {
    u.iter().map(|&c| if c < 128 { Some((c as u8).to_ascii_uppercase() as Unit) } else { None }).collect()
}

/// Только ASCII (для имён заголовков, которые сравниваются через `НРег` между собой).
pub fn is_ascii(u: &[Unit]) -> bool {
    u.iter().all(|c| *c < 128)
}

/// Запись значения во внутреннем формате 1С — ровно как `ЗначениеВСтрокуВнутр` (переводы строк LF).
pub struct Internal {
    out: String,
}

const ARRAY: &str = "51e7a0d2-530b-11d4-b98a-008048da3034";
const MAP: &str = "3d48feae-a9c6-4c5a-a099-9eb6477630c6";
const STRUCTURE: &str = "4238019d-7e49-4fc9-91db-b6b951d5cf8e";

impl Internal {
    pub fn new() -> Internal {
        Internal { out: String::new() }
    }

    pub fn finish(self) -> String {
        self.out
    }

    /// `{"S","…"}`: кавычка удваивается, половина суррогатной пары — `"\hhhh` (строчные), прочее — как есть.
    pub fn string(&mut self, u: &[Unit]) {
        self.out.push_str("{\"S\",\"");
        self.raw_string(u);
        self.out.push_str("\"}");
    }

    pub fn str(&mut self, s: &str) {
        let u: Vec<Unit> = s.encode_utf16().collect();
        self.string(&u);
    }

    fn raw_string(&mut self, u: &[Unit]) {
        for &c in u {
            match c {
                0x22 => self.out.push_str("\"\""),
                0xD800..=0xDFFF => {
                    self.out.push_str("\"\\");
                    self.out.push_str(&format!("{c:04x}"));
                }
                _ => self.out.push(char::from_u32(c as u32).unwrap_or('\u{FFFD}')),
            }
        }
    }

    /// `{"N",<десятичная запись>}`; запись уже нормализована (`body::normalize_number`).
    pub fn number(&mut self, digits: &str) {
        self.out.push_str("{\"N\",");
        self.out.push_str(digits);
        self.out.push('}');
    }

    pub fn boolean(&mut self, b: bool) {
        self.out.push_str(if b { "{\"B\",1}" } else { "{\"B\",0}" });
    }

    /// `Неопределено`.
    pub fn undefined(&mut self) {
        self.out.push_str("{\"U\"}");
    }

    /// Начало коллекции из `count` элементов: `Массив`, `Соответствие` или `Структура`.
    fn begin(&mut self, uuid: &str, count: usize) {
        self.out.push_str("{\"#\",");
        self.out.push_str(uuid);
        self.out.push_str(",\n{");
        self.out.push_str(&count.to_string());
    }

    /// Конец коллекции: у пустой — `{0}`, у непустой — перевод строки перед `}`.
    fn end(&mut self, count: usize) {
        self.out.push_str(if count == 0 { "}\n}" } else { "\n}\n}" });
    }

    /// Массив: `items` пишет каждый элемент вызовом `write`.
    pub fn array<T>(&mut self, items: &[T], mut write: impl FnMut(&mut Internal, &T)) {
        self.begin(ARRAY, items.len());
        for item in items {
            self.out.push_str(",\n");
            write(self, item);
        }
        self.end(items.len());
    }

    /// `Соответствие` со строковыми ключами в порядке вставки (порядок обхода 1С после `ЗначениеИзСтрокиВнутр`
    /// тот же, что у соответствия, заполненного `Вставить` в этом порядке).
    pub fn map<T>(&mut self, items: &[(Vec<Unit>, T)], mut write: impl FnMut(&mut Internal, &T)) {
        self.begin(MAP, items.len());
        for (key, value) in items {
            self.out.push_str(",\n{\n");
            self.string(key);
            self.out.push_str(",\n");
            write(self, value);
            self.out.push_str("\n}");
        }
        self.end(items.len());
    }

    /// `Структура`: ключи — имена полей по порядку, значения пишет `write(индекс)`.
    pub fn structure(&mut self, keys: &[&str], mut write: impl FnMut(&mut Internal, usize)) {
        self.begin(STRUCTURE, keys.len());
        for (i, key) in keys.iter().enumerate() {
            self.out.push_str(",\n{\n");
            self.str(key);
            self.out.push_str(",\n");
            write(self, i);
            self.out.push_str("\n}");
        }
        self.end(keys.len());
    }
}

impl Default for Internal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::units;

    #[test]
    fn spaces_like_1c() {
        for u in [9, 10, 11, 12, 13, 28, 31, 32, 0x85, 0xA0, 0x180E, 0x2000, 0x200A, 0x3000] {
            assert!(is_space(u), "{u}");
        }
        for u in [0, 8, 14, 27, 33, 0x200B, 0xFEFF, 0x2060] {
            assert!(!is_space(u), "{u}");
        }
        assert_eq!(trim(&units("\u{a0} a b\t\r")), units("a b").as_slice());
        assert_eq!(trim_end(&units(" a ")), units(" a").as_slice());
        assert!(is_blank(&units("")) && is_blank(&units(" \u{3000}")) && !is_blank(&units(" x")));
    }

    #[test]
    fn case_like_1c() {
        assert!(lower_eq(&units("Content-TYPE"), "content-type"));
        assert!(lower_eq(&units("B\u{130}NARY"), "binary"));
        assert!(!lower_eq(&units("bинary"), "binary"));
        assert_eq!(upper_all_ascii(&units("patch")), Some(units("PATCH")));
        assert_eq!(upper_all_ascii(&units("pаtch")), None);
    }

    #[test]
    fn internal_format() {
        let mut w = Internal::new();
        w.string(&[b'a' as Unit, 0x22, 0xD83D, 0xDE00, 0, 10]);
        assert_eq!(w.finish(), "{\"S\",\"a\"\"\"\\d83d\"\\de00\u{0}\n\"}");
        let mut w = Internal::new();
        w.array(&[1, 2], |w, n| w.number(&n.to_string()));
        assert_eq!(w.finish(), "{\"#\",51e7a0d2-530b-11d4-b98a-008048da3034,\n{2,\n{\"N\",1},\n{\"N\",2}\n}\n}");
        let mut w = Internal::new();
        w.map::<bool>(&[], |_, _| {});
        assert_eq!(w.finish(), "{\"#\",3d48feae-a9c6-4c5a-a099-9eb6477630c6,\n{0}\n}");
    }
}
