//! Запись XML как у `ЗаписьXML` платформы 1С (`УстановитьСтроку("UTF-8")`, параметры записи по умолчанию),
//! проверено живьём на 8.3.27:
//! - объявление `<?xml version="1.0" encoding="UTF-8"?>`, каждый начальный тег — с новой строки (LF) и отступом
//!   табуляциями по глубине; конечный тег элемента с вложенными элементами — тоже с новой строки и отступом;
//!   элемент без вложенных — `<Имя …/>`; в конце документа перевода строки нет;
//! - в значении атрибута `&`, `<`, `>`, `"` — ссылки `&amp;`, `&lt;`, `&gt;`, `&quot;`; апостроф, TAB, LF, CR,
//!   DEL, C1, NBSP, U+2028/2029, BOM — как есть; прочие символы C0, U+FFFE, U+FFFF — исключение платформы
//!   «Текст XML содержит недопустимый символ» (здесь — ошибка, запрос обслуживает язык 1С с тем же исключением).

pub struct XmlWriter {
    out: String,
    stack: Vec<String>,
    /// Начальный тег последнего элемента ещё не закрыт (`>` или `/>` не записан).
    open: bool,
}

impl Default for XmlWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl XmlWriter {
    pub fn new() -> XmlWriter {
        XmlWriter { out: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), stack: Vec::new(), open: false }
    }

    pub fn start(&mut self, name: &str) {
        if self.open {
            self.out.push('>');
        }
        self.newline(self.stack.len());
        self.out.push('<');
        self.out.push_str(name);
        self.stack.push(name.to_string());
        self.open = true;
    }

    pub fn attr(&mut self, name: &str, value: &str) -> Result<(), String> {
        self.out.push(' ');
        self.out.push_str(name);
        self.out.push_str("=\"");
        escape_attribute(&mut self.out, value)?;
        self.out.push('"');
        Ok(())
    }

    pub fn end(&mut self) {
        let Some(name) = self.stack.pop() else {
            return;
        };
        if self.open {
            self.out.push_str("/>");
            self.open = false;
            return;
        }
        self.newline(self.stack.len());
        self.out.push_str("</");
        self.out.push_str(&name);
        self.out.push('>');
    }

    pub fn finish(mut self) -> String {
        while !self.stack.is_empty() {
            self.end();
        }
        self.out
    }

    fn newline(&mut self, depth: usize) {
        self.out.push('\n');
        for _ in 0..depth {
            self.out.push('\t');
        }
    }
}

/// Значение атрибута по правилам `ЗаписьXML`; недопустимый символ — ошибка с его кодом.
pub fn escape_attribute(out: &mut String, value: &str) -> Result<(), String> {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            '\u{0}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}' => {
                return Err(format!("Текст XML содержит недопустимый символ U+{:04X}", c as u32));
            }
            _ => out.push(c),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_like_platform() {
        // Образец — вывод ЗаписьXML платформы 8.3.27 (проба 8.3).
        let mut w = XmlWriter::new();
        w.start("a:R");
        w.attr("xmlns:a", "urn:x&\"<").unwrap();
        w.attr("Version", "4.0").unwrap();
        w.start("E");
        w.attr("xmlns", "urn:y").unwrap();
        w.start("Empty");
        w.end();
        w.start("P");
        w.start("Q");
        w.attr("V", "эмодзи 😀 & <tag> \"q\" 'a'").unwrap();
        w.end();
        w.end();
        w.end();
        w.end();
        assert_eq!(
            w.finish(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a:R xmlns:a=\"urn:x&amp;&quot;&lt;\" Version=\"4.0\">\n\t<E xmlns=\"urn:y\">\n\t\t<Empty/>\n\t\t<P>\n\t\t\t<Q V=\"эмодзи 😀 &amp; &lt;tag&gt; &quot;q&quot; 'a'\"/>\n\t\t</P>\n\t</E>\n</a:R>"
        );
    }

    #[test]
    fn attribute_characters() {
        let mut out = String::new();
        escape_attribute(&mut out, "x\t\n\r\u{7F}\u{80}\u{85}\u{9F}\u{A0}\u{AD}\u{2028}\u{2029}\u{FEFF}y").unwrap();
        assert_eq!(out, "x\t\n\r\u{7F}\u{80}\u{85}\u{9F}\u{A0}\u{AD}\u{2028}\u{2029}\u{FEFF}y");
        for c in ['\u{0}', '\u{1}', '\u{8}', '\u{B}', '\u{C}', '\u{E}', '\u{1F}', '\u{FFFE}', '\u{FFFF}'] {
            assert!(escape_attribute(&mut String::new(), &format!("x{c}y")).is_err(), "U+{:04X}", c as u32);
        }
    }
}
