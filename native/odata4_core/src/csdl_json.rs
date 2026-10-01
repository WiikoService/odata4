//! CSDL JSON как `OData4_Метаданные.ДокументCSDLJSON`: запись как у `ЗаписьJSON` платформы 1С без переносов
//! строк (`ПереносСтрокJSON.Нет`), проверено живьём на 8.3.27:
//! - без пробелов между элементами; управляющие символы U+0000–U+001F — `\uXXXX` с заглавными hex, кроме LF и CR
//!   (`\n`, `\r`); экранируются `"` и `\`, U+2028 и U+2029 (`\u2028`, `\u2029`); `/`, апостроф, DEL, NBSP, BOM —
//!   как есть;
//! - числа — десятичная запись без экспоненты (здесь только неотрицательные целые).

use crate::csdl::{actions, navigations, row_type_name, Model, Property, Set, TYPE_DESCRIPTION};

pub struct JsonWriter {
    out: String,
    /// На каждом уровне вложенности: записан ли уже первый элемент.
    first: Vec<bool>,
    /// Только что записано имя свойства: значение идёт без запятой.
    after_name: bool,
}

impl Default for JsonWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonWriter {
    pub fn new() -> JsonWriter {
        JsonWriter { out: String::new(), first: Vec::new(), after_name: false }
    }

    fn separator(&mut self) {
        if self.after_name {
            self.after_name = false;
            return;
        }
        if let Some(first) = self.first.last_mut() {
            if *first {
                *first = false;
            } else {
                self.out.push(',');
            }
        }
    }

    pub fn start_object(&mut self) {
        self.separator();
        self.out.push('{');
        self.first.push(true);
    }

    pub fn end_object(&mut self) {
        self.first.pop();
        self.out.push('}');
    }

    pub fn start_array(&mut self) {
        self.separator();
        self.out.push('[');
        self.first.push(true);
    }

    pub fn end_array(&mut self) {
        self.first.pop();
        self.out.push(']');
    }

    pub fn name(&mut self, name: &str) {
        self.separator();
        escape_string(&mut self.out, name);
        self.out.push(':');
        self.after_name = true;
    }

    pub fn string(&mut self, value: &str) {
        self.separator();
        escape_string(&mut self.out, value);
    }

    pub fn boolean(&mut self, value: bool) {
        self.separator();
        self.out.push_str(if value { "true" } else { "false" });
    }

    pub fn number(&mut self, value: u64) {
        self.separator();
        self.out.push_str(&value.to_string());
    }

    pub fn pair(&mut self, name: &str, value: &str) {
        self.name(name);
        self.string(value);
    }

    pub fn pair_bool(&mut self, name: &str, value: bool) {
        self.name(name);
        self.boolean(value);
    }

    pub fn finish(self) -> String {
        self.out
    }
}

/// Строка JSON в кавычках по правилам `ЗаписьJSON`.
pub fn escape_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{0}'..='\u{1F}' | '\u{2028}' | '\u{2029}' => out.push_str(&format!("\\u{:04X}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
}

/// Документ CSDL JSON по описанию модели.
pub fn write_json(model: &Model) -> Result<String, String> {
    let mut w = JsonWriter::new();
    w.start_object();
    w.pair("$Version", "4.0");
    w.pair("$EntityContainer", "StandardODATA.EnterpriseV8");
    w.name("StandardODATA");
    w.start_object();
    for e in &model.enums {
        w.name(&e.name);
        w.start_object();
        w.pair("$Kind", "EnumType");
        for (index, member) in e.members.iter().enumerate() {
            w.name(member);
            w.number(index as u64);
        }
        w.end_object();
    }
    if model.type_description {
        for (name, properties) in TYPE_DESCRIPTION {
            w.name(name);
            w.start_object();
            w.pair("$Kind", "ComplexType");
            for (property, edm) in properties {
                w.name(property);
                w.start_object();
                if let Some(inner) = edm.strip_prefix("Collection(").and_then(|s| s.strip_suffix(')')) {
                    w.pair_bool("$Collection", true);
                    w.pair("$Type", inner);
                } else if *edm != "Edm.String" {
                    w.pair("$Type", edm);
                }
                w.end_object();
            }
            w.end_object();
        }
    }
    // Перегрузки функций одного имени — одним массивом на уровне схемы, имена в порядке первого появления.
    let mut functions: Vec<(&str, Vec<(&Set, usize)>)> = Vec::new();
    for set in &model.sets {
        if set.kind == "ТабличнаяЧасть" {
            json_type(&mut w, model, "ComplexType", &format!("{}_RowType", set.name), set, false)?;
        } else if set.kind == "Регистр" && !set.parent.is_empty() {
            json_type(&mut w, model, "ComplexType", &format!("{}_RowType", set.parent), set, false)?;
        }
        for table in &set.tables {
            let collection = model.find(table)?;
            if collection.hidden {
                json_type(&mut w, model, "ComplexType", &collection.name, collection, false)?;
            }
        }
        json_type(&mut w, model, "EntityType", &set.name, set, true)?;
        for function in set.functions.iter().filter(|f| !f.returns_set) {
            w.name(&function.type_name);
            w.start_object();
            w.pair("$Kind", "ComplexType");
            for p in &function.properties {
                json_property(&mut w, &p.name, &p.edm, true, Some(p));
                if p.kind == "Составной" || p.kind == "Хранилище" {
                    json_property(&mut w, &format!("{}_Type", p.name), "Edm.String", true, None);
                }
            }
            w.end_object();
        }
        for (index, function) in set.functions.iter().enumerate() {
            match functions.iter_mut().find(|(name, _)| *name == function.name) {
                Some((_, overloads)) => overloads.push((set, index)),
                None => functions.push((&function.name, vec![(set, index)])),
            }
        }
    }
    for (name, overloads) in &functions {
        w.name(name);
        w.start_array();
        for (set, index) in overloads {
            let function = &set.functions[*index];
            w.start_object();
            w.pair("$Kind", "Function");
            w.pair_bool("$IsBound", true);
            w.name("$Parameter");
            w.start_array();
            w.start_object();
            w.pair("$Name", "bindingParameter");
            w.pair_bool("$Collection", true);
            w.pair("$Type", &format!("StandardODATA.{}", set.name));
            w.end_object();
            for (parameter, edm) in &function.parameters {
                w.start_object();
                w.pair("$Name", parameter);
                w.pair("$Type", edm);
                w.pair_bool("$Nullable", true);
                w.end_object();
            }
            w.end_array();
            w.name("$ReturnType");
            w.start_object();
            w.pair_bool("$Collection", true);
            let result = if function.returns_set { &set.name } else { &function.type_name };
            w.pair("$Type", &format!("StandardODATA.{result}"));
            w.end_object();
            w.end_object();
        }
        w.end_array();
    }
    for action in ["Post", "Unpost"] {
        let sets: Vec<&Set> = model.sets.iter().filter(|s| actions(s).contains(&action)).collect();
        if sets.is_empty() {
            continue;
        }
        w.name(action);
        w.start_array();
        for set in sets {
            w.start_object();
            w.pair("$Kind", "Action");
            w.pair_bool("$IsBound", true);
            w.name("$Parameter");
            w.start_array();
            w.start_object();
            w.pair("$Name", "bindingParameter");
            w.pair("$Type", &format!("StandardODATA.{}", set.name));
            w.end_object();
            w.start_object();
            w.pair("$Name", "PostingModeOperational");
            w.pair("$Type", "Edm.Boolean");
            w.pair_bool("$Nullable", true);
            w.end_object();
            w.end_array();
            w.end_object();
        }
        w.end_array();
    }
    w.name("EnterpriseV8");
    w.start_object();
    w.pair("$Kind", "EntityContainer");
    for set in &model.sets {
        w.name(&set.name);
        w.start_object();
        w.pair_bool("$Collection", true);
        w.pair("$Type", &format!("StandardODATA.{}", set.name));
        let bindings: Vec<&Property> = navigations(set).collect();
        if !bindings.is_empty() {
            w.name("$NavigationPropertyBinding");
            w.start_object();
            for p in bindings {
                w.pair(&p.navigation, &p.target);
            }
            w.end_object();
        }
        w.end_object();
    }
    w.end_object();
    w.end_object();
    w.end_object();
    Ok(w.finish())
}

/// `ЗаписатьТипJSON`: тип сущности или строки — те же члены, что в CSDL XML.
fn json_type(
    w: &mut JsonWriter,
    model: &Model,
    kind: &str,
    name: &str,
    set: &Set,
    with_key: bool,
) -> Result<(), String> {
    w.name(name);
    w.start_object();
    w.pair("$Kind", kind);
    if with_key {
        w.name("$Key");
        w.start_array();
        for property in &set.key {
            w.string(property);
        }
        w.end_array();
    }
    for p in &set.properties {
        if with_key || p.kind != "Хранилище" {
            json_property(w, &p.name, &p.edm, !p.in_key, Some(p));
        }
        if p.kind == "Хранилище" {
            json_property(w, &format!("{}_Base64Data", p.name), "Edm.Binary", true, None);
        }
        if p.kind == "Составной" || p.kind == "Хранилище" {
            json_property(w, &format!("{}_Type", p.name), "Edm.String", true, None);
        }
    }
    for table in &set.tables {
        let collection = model.find(table)?;
        w.name(&collection.name_in_owner);
        w.start_object();
        w.pair_bool("$Collection", true);
        w.pair("$Type", &format!("StandardODATA.{}", row_type_name(set, collection)));
        w.end_object();
    }
    if with_key {
        for p in navigations(set) {
            w.name(&p.navigation);
            w.start_object();
            w.pair("$Kind", "NavigationProperty");
            w.pair("$Type", &format!("StandardODATA.{}", p.target));
            w.pair_bool("$Nullable", true);
            w.end_object();
        }
    }
    w.end_object();
    Ok(())
}

/// `ЗаписатьСвойствоJSON`: `$Type` опускается у Edm.String, `$Nullable` пишется только true.
fn json_property(w: &mut JsonWriter, name: &str, edm: &str, nullable: bool, p: Option<&Property>) {
    w.name(name);
    w.start_object();
    if edm != "Edm.String" {
        w.pair("$Type", edm);
    }
    if nullable {
        w.pair_bool("$Nullable", true);
    }
    if let Some(p) = p {
        if edm == "Edm.String" && p.length > 0 {
            w.name("$MaxLength");
            w.number(p.length);
        }
        if edm == "Edm.Decimal" && p.precision > 0 {
            w.name("$Precision");
            w.number(p.precision);
            w.name("$Scale");
            w.number(p.scale);
        }
    }
    w.end_object();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_like_platform() {
        // Образцы — вывод ЗаписьJSON платформы 8.3.27 (проба 8.3).
        let cases = [
            ("x\u{0}y", r#""x\u0000y""#),
            ("x\u{8}y", r#""x\u0008y""#),
            ("x\ty", r#""x\u0009y""#),
            ("x\ny", r#""x\ny""#),
            ("x\u{C}y", r#""x\u000Cy""#),
            ("x\ry", r#""x\ry""#),
            ("x\u{1F}y", r#""x\u001Fy""#),
            ("x\"y", r#""x\"y""#),
            ("x'y", r#""x'y""#),
            ("x/y", r#""x/y""#),
            ("x\\y", r#""x\\y""#),
            ("x\u{7F}y", "\"x\u{7F}y\""),
            ("x\u{A0}y", "\"x\u{A0}y\""),
            ("x\u{2028}y", r#""x\u2028y""#),
            ("x\u{2029}y", r#""x\u2029y""#),
            ("x\u{FEFF}y", "\"x\u{FEFF}y\""),
        ];
        for (input, expected) in cases {
            let mut out = String::new();
            escape_string(&mut out, input);
            assert_eq!(out, expected, "{input:?}");
        }
    }

    #[test]
    fn separators() {
        let mut w = JsonWriter::new();
        w.start_object();
        w.pair("a", "b");
        w.name("c");
        w.start_array();
        w.number(1);
        w.boolean(true);
        w.start_object();
        w.end_object();
        w.end_array();
        w.name("d");
        w.start_object();
        w.pair_bool("e", false);
        w.end_object();
        w.end_object();
        assert_eq!(w.finish(), r#"{"a":"b","c":[1,true,{}],"d":{"e":false}}"#);
    }
}
