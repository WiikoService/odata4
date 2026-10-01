//! Методы разбора тел (этап 8.2): `РазобратьТело(Текст)` и `РазобратьПакет(ТипСодержимого, Текст)`.
//!
//! Ответ — строка:
//! - внутренний формат 1С (начинается с `{`) — результат для `ЗначениеИзСтрокиВнутр`;
//! - `""` — отказ: тело не строгий JSON, ошибка формата пакета или случай, который компонента не берёт; разбирает BSL
//!   (он же даёт статус и текст ошибки);
//! - `"!…"` — сбой компоненты (неверная строка на входе, паника): BSL пишет сбой в журнал и разбирает сам.

use std::panic::{catch_unwind, AssertUnwindSafe};

use addin1c::{AddinResult, Variant};

use crate::component::OData4Native;

impl OData4Native {
    pub(crate) fn parse_body(&mut self, text: &mut Variant, ret_value: &mut Variant) -> AddinResult {
        let answer = body_answer(text.get_str1c()?);
        ret_value.set_str1c(answer.as_str())?;
        Ok(())
    }

    pub(crate) fn parse_batch(
        &mut self,
        content_type: &mut Variant,
        text: &mut Variant,
        ret_value: &mut Variant,
    ) -> AddinResult {
        let content_type = content_type.get_str1c()?.to_vec();
        let answer = batch_answer(&content_type, text.get_str1c()?);
        ret_value.set_str1c(answer.as_str())?;
        Ok(())
    }
}

/// Ответ `РазобратьТело`; паника — `"!…"`.
pub fn body_answer(text: &[u16]) -> String {
    answer(catch_unwind(AssertUnwindSafe(|| odata4_core::body_internal(text))))
}

/// Ответ `РазобратьПакет`; паника — `"!…"`.
pub fn batch_answer(content_type: &[u16], text: &[u16]) -> String {
    answer(catch_unwind(AssertUnwindSafe(|| odata4_core::batch_internal(content_type, text))))
}

fn answer(result: std::thread::Result<Option<String>>) -> String {
    match result {
        Ok(Some(internal)) => internal,
        Ok(None) => String::new(),
        Err(_) => "!паника при разборе тела".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn answers() {
        assert!(body_answer(&utf16(r#"{"a":1}"#)).starts_with("{\"#\",3d48feae"));
        assert_eq!(body_answer(&utf16("[1]")), "");
        assert_eq!(body_answer(&[0xD800]), "");
        assert_eq!(batch_answer(&utf16("text/plain"), &utf16("x")), "");
        assert!(batch_answer(
            &utf16("application/json"),
            &utf16(r#"{"requests":[{"id":"1","method":"GET","url":"x"}]}"#)
        )
        .starts_with("{\"#\",4238019d"));
        assert_eq!(answer(Err(Box::new(1))), "!паника при разборе тела");
    }
}
