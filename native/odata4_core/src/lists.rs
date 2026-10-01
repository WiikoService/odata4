//! Списки параметров: `$orderby`, `$select`, `$expand` и вложенные параметры раскрытия —
//! как `OData4_Разбор` (`СписокПорядка`, `СписокВыбора`, `ПутьВыбора`, `СписокРаскрытия`, `ОпцииРаскрытия`).

use crate::ast::{ExpandItem, ExpandOptions, OrderItem};
use crate::error::ParseError;
use crate::lexer::Kind;
use crate::parser::{is_name, is_sign, Parser, R};
use crate::text::{count_digits, digits_value, starts_with_dollar, string, Unit};

/// `$orderby` → массив «выражение, направление».
pub fn parse_orderby(text: &[Unit]) -> R<Vec<OrderItem>> {
    let mut p = Parser::new(text, "$orderby")?;
    let items = p.orderby_list()?;
    p.finish()?;
    Ok(items)
}

/// `$select` → массив путей (путь — массив имён, `["*"]` — все).
pub fn parse_select(text: &[Unit]) -> R<Vec<Vec<String>>> {
    let mut p = Parser::new(text, "$select")?;
    let items = p.select_list()?;
    p.finish()?;
    Ok(items)
}

/// `$expand` → массив «путь, параметры раскрытия».
pub fn parse_expand(text: &[Unit]) -> R<Vec<ExpandItem>> {
    let mut p = Parser::new(text, "$expand")?;
    let items = p.expand_list()?;
    p.finish()?;
    Ok(items)
}

/// `ЭтоЦелоеБезЗнака`: от 1 до 9 цифр без знака и суффикса.
pub fn is_unsigned_int(text: &[Unit]) -> bool {
    !text.is_empty() && text.len() <= 9 && count_digits(text, 1) == text.len()
}

impl Parser<'_> {
    /// `СписокПорядка`: элементы до «;», «)» или конца.
    pub(crate) fn orderby_list(&mut self) -> R<Vec<OrderItem>> {
        let mut items = Vec::new();
        loop {
            let expression = self.expression()?;
            let mut direction = "asc";
            if is_name(self.current(), "asc") {
                self.next();
            } else if is_name(self.current(), "desc") {
                direction = "desc";
                self.next();
            }
            items.push(OrderItem { expression, direction });
            if !is_sign(self.current(), b',') {
                break;
            }
            self.next();
        }
        Ok(items)
    }

    pub(crate) fn select_list(&mut self) -> R<Vec<Vec<String>>> {
        let mut items = Vec::new();
        loop {
            items.push(self.select_path()?);
            if !is_sign(self.current(), b',') {
                break;
            }
            self.next();
        }
        Ok(items)
    }

    /// `ПутьВыбора`: «*» или Имя {/ Имя} с «*» только последним.
    fn select_path(&mut self) -> R<Vec<String>> {
        let mut segments = Vec::new();
        loop {
            let t = self.current().clone();
            if is_sign(&t, b'*') {
                self.next();
                segments.push("*".to_string());
                return Ok(segments);
            }
            if t.kind != Kind::Name || starts_with_dollar(&t.text) {
                return Err(self.expected("имя"));
            }
            self.next();
            segments.push(string(&t.text));
            if !is_sign(self.current(), b'/') {
                return Ok(segments);
            }
            self.next();
        }
    }

    /// `СписокРаскрытия`: скобки параметров раскрытия считаются в общей вложенности.
    pub(crate) fn expand_list(&mut self) -> R<Vec<ExpandItem>> {
        let mut items = Vec::new();
        loop {
            let path = self.select_path()?;
            let mut options = ExpandOptions::default();
            let open = self.current().clone();
            if is_sign(&open, b'(') {
                self.next();
                self.enter(&open)?;
                self.expand_options(&mut options)?;
                self.expect_sign(b')')?;
                self.leave();
            }
            items.push(ExpandItem { path, options });
            if !is_sign(self.current(), b',') {
                break;
            }
            self.next();
        }
        Ok(items)
    }

    /// `ОпцииРаскрытия`: `$имя=значение` через «;»; повторный параметр заменяет прежний.
    fn expand_options(&mut self, options: &mut ExpandOptions) -> R<()> {
        loop {
            let t = self.current().clone();
            if t.kind != Kind::Name || !starts_with_dollar(&t.text) {
                return Err(self.expected("параметр раскрытия"));
            }
            self.next();
            self.expect_sign(b'=')?;
            let name = string(&t.text);
            match name.as_str() {
                "$filter" => options.filter = Some(self.expression()?),
                "$orderby" => options.orderby = Some(self.orderby_list()?),
                "$select" => options.select = Some(self.select_list()?),
                "$expand" => options.expand = Some(self.expand_list()?),
                "$top" | "$skip" => {
                    let value = self.current().clone();
                    if value.kind != Kind::Literal || !is_unsigned_int(&value.text) {
                        return Err(self.expected("неотрицательное целое число"));
                    }
                    self.next();
                    let number = Some(digits_value(&value.text));
                    if name == "$top" {
                        options.top = number;
                    } else {
                        options.skip = number;
                    }
                }
                "$count" => {
                    let value = self.current().clone();
                    if !is_name(&value, "true") && !is_name(&value, "false") {
                        return Err(self.expected("true или false"));
                    }
                    self.next();
                    options.count = Some(is_name(&value, "true"));
                }
                "$levels" | "$search" | "$apply" | "$compute" => {
                    return Err(ParseError::not_implemented(format!("Параметр запроса не поддерживается: {name}")));
                }
                _ => {
                    return Err(ParseError::bad_request(format!(
                        "{}: неизвестный параметр {} в позиции {}",
                        self.param(),
                        name,
                        t.pos
                    )));
                }
            }
            if !is_sign(self.current(), b';') {
                break;
            }
            self.next();
        }
        Ok(())
    }
}
