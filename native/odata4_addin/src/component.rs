//! Объект компоненты `AddIn.<имя подключения>.OData4Native`: методы `Версия()`, `РазобратьПараметры(json)`;
//! выдача JSON (этап 8.1) — `ЕстьСловарьВыдачи(ключ)`, `ЗагрузитьСловарьВыдачи(ключ, json)`,
//! `ВыдатьJSON(описание, данные)`; разбор тел (8.2, `bodies.rs`) — `РазобратьТело(текст)`,
//! `РазобратьПакет(тип, текст)`; `$metadata` (8.3) — `ВывестиCSDL(описание, формат)`.

use std::error::Error;
use std::panic::catch_unwind;

use addin1c::{name, AddinResult, CStr1C, Connection, MethodInfo, Methods, PropInfo, SimpleAddin, Variant};
use odata4_core::api::failure_json;

pub struct OData4Native {
    last_error: Option<Box<dyn Error>>,
}

impl OData4Native {
    pub fn new() -> OData4Native {
        OData4Native { last_error: None }
    }

    fn version(&mut self, ret_value: &mut Variant) -> AddinResult {
        ret_value.set_str1c(env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }

    fn parse(&mut self, input: &mut Variant, ret_value: &mut Variant) -> AddinResult {
        let answer = parse_request(input.get_str1c()?);
        ret_value.set_str1c(answer)?;
        Ok(())
    }

    fn csdl(&mut self, input: &mut Variant, format: &mut Variant, ret_value: &mut Variant) -> AddinResult {
        let answer = csdl_request(input.get_str1c()?, format.get_str1c()?);
        ret_value.set_str1c(answer)?;
        Ok(())
    }
}

impl OData4Native {
    /// `ЕстьСловарьВыдачи(ключ)` → Булево.
    fn has_dictionary(&mut self, key: &mut Variant, ret_value: &mut Variant) -> AddinResult {
        let key = String::from_utf16_lossy(key.get_str1c()?);
        ret_value.set_bool(catch_unwind(|| odata4_core::output::has_dictionary(&key)).unwrap_or(false));
        Ok(())
    }

    /// `ЗагрузитьСловарьВыдачи(ключ, json)` → "" или текст ошибки.
    fn load_dictionary(&mut self, key: &mut Variant, json: &mut Variant, ret_value: &mut Variant) -> AddinResult {
        let answer = match (String::from_utf16(key.get_str1c()?), String::from_utf16(json.get_str1c()?)) {
            (Ok(key), Ok(json)) => catch_unwind(|| odata4_core::output::load_dictionary(&key, &json))
                .unwrap_or_else(|_| Err("паника при загрузке словаря".to_string()))
                .err()
                .unwrap_or_default(),
            _ => "словарь: неверная строка UTF-16".to_string(),
        };
        ret_value.set_str1c(answer.as_str())?;
        Ok(())
    }

    /// `ВыдатьJSON(описание, данные)` → текст ответа (начинается с `{`) или `!` и текст ошибки.
    fn output_json(&mut self, desc: &mut Variant, data: &mut Variant, ret_value: &mut Variant) -> AddinResult {
        let answer = output_request(desc.get_str1c()?, data.get_str1c()?);
        ret_value.set_str1c(answer.as_str())?;
        Ok(())
    }
}

/// Выдача JSON по строкам 1С (UTF-16): текст ответа или `!` и причина сбоя (неверный UTF-16, паника, ошибка выдачи).
pub fn output_request(desc: &[u16], data: &[u16]) -> String {
    let (Ok(desc), Ok(data)) = (String::from_utf16(desc), String::from_utf16(data)) else {
        return "!неверная строка UTF-16".to_string();
    };
    match catch_unwind(|| odata4_core::output::output_json(&desc, &data)) {
        Ok(Ok(text)) => text,
        Ok(Err(message)) => format!("!{message}"),
        Err(_) => "!паника при выдаче".to_string(),
    }
}

impl Default for OData4Native {
    fn default() -> Self {
        Self::new()
    }
}

/// Строка 1С (UTF-16) с JSON пар → ответ `odata4_core::request_json`. Неверный UTF-16 и паника — `failure`.
pub fn parse_request(input: &[u16]) -> String {
    let Ok(text) = String::from_utf16(input) else {
        return failure_json("параметры: неверная строка UTF-16");
    };
    catch_unwind(|| odata4_core::request_json(&text)).unwrap_or_else(|_| failure_json("паника при разборе"))
}

/// `ВывестиCSDL`: описание модели (контракт `odata4_core::csdl`, строка 1С) и формат "XML" | "JSON" → документ
/// `$metadata`. Сбой (описание не по контракту, недопустимый символ, неверный UTF-16, паника) — `{"failure": "…"}`:
/// документ CSDL так не начинается (XML — `<?xml`, JSON — `{"$Version"`).
pub fn csdl_request(input: &[u16], format: &[u16]) -> String {
    let (Ok(text), Ok(format)) = (String::from_utf16(input), String::from_utf16(format)) else {
        return failure_json("описание модели: неверная строка UTF-16");
    };
    catch_unwind(|| odata4_core::csdl_document(&text, &format))
        .unwrap_or_else(|_| Err("паника при выводе $metadata".to_string()))
        .unwrap_or_else(|message| failure_json(&message))
}

impl SimpleAddin for OData4Native {
    fn name() -> &'static CStr1C {
        name!("OData4Native")
    }

    fn init(&mut self, _interface: &'static Connection) -> bool {
        true
    }

    fn save_error(&mut self, err: Option<Box<dyn Error>>) {
        self.last_error = err;
    }

    fn methods() -> &'static [MethodInfo<Self>] {
        &[
            MethodInfo { name: name!("Версия"), method: Methods::Method0(Self::version) },
            MethodInfo { name: name!("РазобратьПараметры"), method: Methods::Method1(Self::parse) },
            MethodInfo {
                name: name!("ЕстьСловарьВыдачи"), method: Methods::Method1(Self::has_dictionary)
            },
            MethodInfo {
                name: name!("ЗагрузитьСловарьВыдачи"), method: Methods::Method2(Self::load_dictionary)
            },
            MethodInfo { name: name!("ВыдатьJSON"), method: Methods::Method2(Self::output_json) },
            MethodInfo { name: name!("РазобратьТело"), method: Methods::Method1(Self::parse_body) },
            MethodInfo { name: name!("РазобратьПакет"), method: Methods::Method2(Self::parse_batch) },
            MethodInfo { name: name!("ВывестиCSDL"), method: Methods::Method2(Self::csdl) },
        ]
    }

    fn properties() -> &'static [PropInfo<Self>] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn parse_request_answers() {
        assert!(parse_request(&utf16(r#"[["$top","5"]]"#)).starts_with(r#"{"options":"#));
        assert_eq!(
            parse_request(&utf16(r#"[["$search","x"]]"#)),
            r#"{"error":{"message":"Параметр запроса не поддерживается: $search","status":501}}"#
        );
        assert_eq!(parse_request(&[0xD800]), r#"{"failure":"параметры: неверная строка UTF-16"}"#);
    }

    #[test]
    fn method_names() {
        let names: Vec<&CStr1C> = OData4Native::methods().iter().map(|m| m.name).collect();
        assert!(names[0] == name!("Версия"));
        assert!(names[1] == name!("РазобратьПараметры"));
        assert!(names.contains(&name!("ВывестиCSDL")));
    }

    #[test]
    fn csdl_request_answers() {
        let model = r#"{"Версия":1,"ЕстьОписаниеТипов":false,"Перечисления":[],"Наборы":[],"Коллекции":[]}"#;
        assert!(csdl_request(&utf16(model), &utf16("XML")).starts_with("<?xml"));
        assert!(csdl_request(&utf16(model), &utf16("JSON")).starts_with(r#"{"$Version":"4.0""#));
        assert!(csdl_request(&utf16("{}"), &utf16("XML")).starts_with(r#"{"failure":"#));
        assert!(csdl_request(&[0xD800], &utf16("XML")).starts_with(r#"{"failure":"#));
    }
}
