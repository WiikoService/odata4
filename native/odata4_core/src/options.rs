//! Все системные параметры запроса — как `OData4_Разбор.РазобратьПараметры`: пары обрабатываются в данном
//! порядке, первая ошибка прерывает разбор; параметры без `$` не рассматриваются.

use crate::ast::Options;
use crate::error::ParseError;
use crate::lists::{is_unsigned_int, parse_expand, parse_orderby, parse_select};
use crate::parser::parse_filter;
use crate::text::{digits_value, eq_ascii, string, Unit};

/// Пары «имя параметра → значение» в порядке перебора `ПараметрыЗапроса` 1С.
pub fn parse_options(pairs: &[(String, Vec<Unit>)]) -> Result<Options, ParseError> {
    let mut o = Options::default();
    for (name, value) in pairs {
        if !name.starts_with('$') {
            continue;
        }
        match name.as_str() {
            "$filter" => o.base.filter = Some(parse_filter(value)?),
            "$orderby" => o.base.orderby = Some(parse_orderby(value)?),
            "$select" => o.base.select = Some(parse_select(value)?),
            "$expand" => o.base.expand = Some(parse_expand(value)?),
            "$top" | "$skip" => {
                if !is_unsigned_int(value) {
                    return Err(ParseError::bad_request(format!("{name}: ожидалось неотрицательное целое число")));
                }
                let number = Some(digits_value(value));
                if name == "$top" {
                    o.base.top = number;
                } else {
                    o.base.skip = number;
                }
            }
            "$count" => {
                if eq_ascii(value, "true") {
                    o.base.count = Some(true);
                } else if eq_ascii(value, "false") {
                    o.base.count = Some(o.base.count == Some(true));
                } else {
                    return Err(ParseError::bad_request("$count: ожидалось true или false"));
                }
            }
            "$inlinecount" => {
                if eq_ascii(value, "allpages") {
                    o.base.count = Some(true);
                } else if eq_ascii(value, "none") {
                    o.base.count = Some(o.base.count == Some(true));
                } else {
                    return Err(ParseError::bad_request("$inlinecount: ожидалось allpages или none"));
                }
            }
            "$skiptoken" => o.skiptoken = Some(string(value)),
            "$format" => o.format = Some(string(value)),
            "$search" | "$apply" | "$compute" => {
                return Err(ParseError::not_implemented(format!("Параметр запроса не поддерживается: {name}")));
            }
            _ => return Err(ParseError::bad_request(format!("Неизвестный параметр запроса: {name}"))),
        }
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::units;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, Vec<Unit>)> {
        list.iter().map(|(name, value)| (name.to_string(), units(value))).collect()
    }

    fn error(list: &[(&str, &str)]) -> ParseError {
        parse_options(&pairs(list)).expect_err("ожидалась ошибка")
    }

    /// Компонента `$apply` не разбирает: прямой вызов с `$apply` — 501 (фасад `OData4_Движок` вынимает
    /// `$apply` до вызова компоненты и разбирает его на языке 1С; общий пример этого больше не закрепляет).
    #[test]
    fn top_level_apply_is_not_implemented() {
        let e = error(&[("$apply", "identity")]);
        assert_eq!(e.status, 501);
        assert_eq!(e.message, "Параметр запроса не поддерживается: $apply");
        for name in ["$search", "$compute"] {
            let e = error(&[(name, "x")]);
            assert_eq!(
                (e.status, e.message.as_str()),
                (501, format!("Параметр запроса не поддерживается: {name}").as_str())
            );
        }
    }

    /// Первая ошибка — по порядку пар: ошибка `$filter` перед `$apply` называется она.
    #[test]
    fn first_error_follows_pair_order() {
        let e = error(&[("$filter", "Code eq"), ("$apply", "identity")]);
        assert_eq!(e.status, 400);
        let e = error(&[("$apply", "identity"), ("$filter", "Code eq")]);
        assert_eq!(e.status, 501);
    }
}
