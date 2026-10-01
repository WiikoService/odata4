//! Вывод `$metadata` (CSDL XML и CSDL JSON) побайтно как `OData4_Метаданные` (язык 1С).
//!
//! Вход — описание модели для вывода CSDL (контракт `OData4_Метаданные.ОписаниеCSDL`, версия 1), JSON:
//!
//! ```text
//! {
//!   "Версия": 1,
//!   "ЕстьОписаниеТипов": false,                  // комплексные типы TypeDescription и квалификаторов
//!   "Перечисления": [{"Имя": "…", "Члены": ["…", …]}, …],   // в порядке вывода; члены — в порядке вывода
//!   "Наборы": [НАБОР, …],                        // в порядке Модель.Порядок
//!   "Коллекции": [НАБОР, …]                      // наборы из ТабличныеЧасти, которых нет в «Наборы» (скрытые)
//! }
//! НАБОР = {"Имя", "Вид", "Родитель", "ИмяВВладельце", "Скрытый", "Ключ": ["…"], "Свойства": [СВОЙСТВО, …],
//!          "ТабличныеЧасти": ["имя набора", …], "ВиртуальныеТаблицы": [ТАБЛИЦА, …]}
//! СВОЙСТВО = ["Имя", "ТипEdm", "Вид", ВКлюче, Длина, Разрядность, Дробная, "ИмяНавигации", "ЦелевойНабор"]
//! ТАБЛИЦА = {"Имя", "ИмяТипа", "ВозвратНабор", "Свойства": [СВОЙСТВО, …], "Параметры": [["Имя", "ТипEdm"], …]}
//! ```
//!
//! Правила вывода — те же, что в `OData4_Метаданные` и `OData4_Действия` (виды наборов «ТабличнаяЧасть»,
//! «Регистр», «Документ», виды свойств «Ссылка», «Составной», «Хранилище»); запись XML — как у `ЗаписьXML`
//! платформы (`csdl_xml`), JSON — как у `ЗаписьJSON` без переносов строк (`csdl_json`).

use serde_json::Value;

use crate::csdl_json::write_json;
use crate::csdl_xml::XmlWriter;

pub struct Model {
    pub enums: Vec<EnumType>,
    pub type_description: bool,
    pub sets: Vec<Set>,
    pub collections: Vec<Set>,
}

pub struct EnumType {
    pub name: String,
    pub members: Vec<String>,
}

pub struct Set {
    pub name: String,
    pub kind: String,
    pub parent: String,
    pub name_in_owner: String,
    pub hidden: bool,
    pub key: Vec<String>,
    pub properties: Vec<Property>,
    pub tables: Vec<String>,
    pub functions: Vec<Function>,
}

pub struct Property {
    pub name: String,
    pub edm: String,
    pub kind: String,
    pub in_key: bool,
    pub length: u64,
    pub precision: u64,
    pub scale: u64,
    pub navigation: String,
    pub target: String,
}

pub struct Function {
    pub name: String,
    pub type_name: String,
    pub returns_set: bool,
    pub properties: Vec<Property>,
    pub parameters: Vec<(String, String)>,
}

/// Документ `$metadata` в формате `format` ("XML" или "JSON") по описанию модели `input`. Ошибка — текст
/// причины (описание не по контракту, недопустимый в XML символ).
pub fn csdl_document(input: &str, format: &str) -> Result<String, String> {
    let model = parse_model(input)?;
    match format {
        "XML" => write_xml(&model),
        "JSON" => write_json(&model),
        _ => Err(format!("неизвестный формат $metadata: {format}")),
    }
}

/// Разбор описания модели (контракт — в заголовке модуля).
pub fn parse_model(input: &str) -> Result<Model, String> {
    let value: Value = serde_json::from_str(input).map_err(|e| format!("описание модели не JSON: {e}"))?;
    let root = value.as_object().ok_or("описание модели: ожидался объект")?;
    match root.get("Версия").and_then(Value::as_u64) {
        Some(1) => {}
        other => return Err(format!("описание модели: неизвестная версия {other:?}")),
    }
    let mut enums = Vec::new();
    for item in array(root.get("Перечисления"), "Перечисления")? {
        enums.push(EnumType {
            name: string(item.get("Имя"), "Перечисления.Имя")?,
            members: strings(item.get("Члены"), "Члены")?,
        });
    }
    let sets = array(root.get("Наборы"), "Наборы")?.iter().map(parse_set).collect::<Result<Vec<_>, _>>()?;
    let collections = match root.get("Коллекции") {
        None => Vec::new(),
        some => array(some, "Коллекции")?.iter().map(parse_set).collect::<Result<Vec<_>, _>>()?,
    };
    Ok(Model {
        enums,
        type_description: boolean(root.get("ЕстьОписаниеТипов"), "ЕстьОписаниеТипов")?,
        sets,
        collections,
    })
}

fn parse_set(item: &Value) -> Result<Set, String> {
    let mut functions = Vec::new();
    for table in array(item.get("ВиртуальныеТаблицы"), "ВиртуальныеТаблицы")? {
        let mut parameters = Vec::new();
        for parameter in array(table.get("Параметры"), "Параметры")? {
            match parameter.as_array().map(Vec::as_slice) {
                Some([Value::String(name), Value::String(edm)]) => parameters.push((name.clone(), edm.clone())),
                _ => return Err(format!("описание модели: параметр {parameter}")),
            }
        }
        functions.push(Function {
            name: string(table.get("Имя"), "ВиртуальныеТаблицы.Имя")?,
            type_name: string(table.get("ИмяТипа"), "ИмяТипа")?,
            returns_set: boolean(table.get("ВозвратНабор"), "ВозвратНабор")?,
            properties: properties(table.get("Свойства"))?,
            parameters,
        });
    }
    Ok(Set {
        name: string(item.get("Имя"), "Наборы.Имя")?,
        kind: string(item.get("Вид"), "Вид")?,
        parent: string(item.get("Родитель"), "Родитель")?,
        name_in_owner: string(item.get("ИмяВВладельце"), "ИмяВВладельце")?,
        hidden: boolean(item.get("Скрытый"), "Скрытый")?,
        key: strings(item.get("Ключ"), "Ключ")?,
        properties: properties(item.get("Свойства"))?,
        tables: strings(item.get("ТабличныеЧасти"), "ТабличныеЧасти")?,
        functions,
    })
}

fn properties(value: Option<&Value>) -> Result<Vec<Property>, String> {
    let mut result = Vec::new();
    for item in array(value, "Свойства")? {
        let parts =
            item.as_array().filter(|p| p.len() == 9).ok_or_else(|| format!("описание модели: свойство {item}"))?;
        result.push(Property {
            name: string(parts.first(), "свойство: имя")?,
            edm: string(parts.get(1), "свойство: тип")?,
            kind: string(parts.get(2), "свойство: вид")?,
            in_key: boolean(parts.get(3), "свойство: в ключе")?,
            length: number(parts.get(4), "свойство: длина")?,
            precision: number(parts.get(5), "свойство: разрядность")?,
            scale: number(parts.get(6), "свойство: дробная часть")?,
            navigation: string(parts.get(7), "свойство: навигация")?,
            target: string(parts.get(8), "свойство: целевой набор")?,
        });
    }
    Ok(result)
}

fn array<'a>(value: Option<&'a Value>, what: &str) -> Result<&'a Vec<Value>, String> {
    value.and_then(Value::as_array).ok_or_else(|| format!("описание модели: {what} — ожидался массив"))
}

fn string(value: Option<&Value>, what: &str) -> Result<String, String> {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("описание модели: {what} — ожидалась строка"))
}

fn strings(value: Option<&Value>, what: &str) -> Result<Vec<String>, String> {
    array(value, what)?.iter().map(|v| string(Some(v), what)).collect()
}

fn boolean(value: Option<&Value>, what: &str) -> Result<bool, String> {
    value.and_then(Value::as_bool).ok_or_else(|| format!("описание модели: {what} — ожидалось Булево"))
}

fn number(value: Option<&Value>, what: &str) -> Result<u64, String> {
    value.and_then(Value::as_u64).ok_or_else(|| format!("описание модели: {what} — ожидалось целое число"))
}

impl Model {
    /// Набор по имени (как `OData4_Модель.НайтиНабор` построенной модели): сначала наборы, потом коллекции.
    pub fn find(&self, name: &str) -> Result<&Set, String> {
        self.sets
            .iter()
            .chain(self.collections.iter())
            .find(|s| s.name == name)
            .ok_or_else(|| format!("описание модели: нет набора {name}"))
    }
}

/// Имя комплексного типа строк коллекции (`ИмяСтрочногоТипа`).
pub fn row_type_name(owner: &Set, collection: &Set) -> String {
    if collection.kind == "Регистр" {
        format!("{}_RowType", owner.name)
    } else if collection.hidden {
        collection.name.clone()
    } else {
        format!("{}_RowType", collection.name)
    }
}

/// Действия набора (`OData4_Действия.ДействияНабора`): Post и Unpost у любого документа.
pub fn actions(set: &Set) -> &'static [&'static str] {
    if set.kind == "Документ" {
        &["Post", "Unpost"]
    } else {
        &[]
    }
}

/// Комплексные типы описания типов (`ТипыОписанияТипов`): имя и свойства «имя:тип».
pub const TYPE_DESCRIPTION: [(&str, &[(&str, &str)]); 5] = [
    (
        "TypeDescription",
        &[
            ("Types", "Collection(Edm.String)"),
            ("NumberQualifiers", "StandardODATA.NumberQualifiers"),
            ("StringQualifiers", "StandardODATA.StringQualifiers"),
            ("DateQualifiers", "StandardODATA.DateQualifiers"),
            ("BinaryDataQualifiers", "StandardODATA.BinaryDataQualifiers"),
        ],
    ),
    ("NumberQualifiers", &[("AllowedSign", "Edm.String"), ("Digits", "Edm.Int16"), ("FractionDigits", "Edm.Int16")]),
    ("StringQualifiers", &[("AllowedLength", "Edm.String"), ("Length", "Edm.Int64")]),
    ("DateQualifiers", &[("DateFractions", "Edm.String")]),
    ("BinaryDataQualifiers", &[("AllowedLength", "Edm.String"), ("Length", "Edm.Int64")]),
];

fn write_xml(model: &Model) -> Result<String, String> {
    let mut w = XmlWriter::new();
    w.start("edmx:Edmx");
    w.attr("xmlns:edmx", "http://docs.oasis-open.org/odata/ns/edmx")?;
    w.attr("Version", "4.0")?;
    w.start("edmx:DataServices");
    w.start("Schema");
    w.attr("xmlns", "http://docs.oasis-open.org/odata/ns/edm")?;
    w.attr("Namespace", "StandardODATA")?;
    for e in &model.enums {
        w.start("EnumType");
        w.attr("Name", &e.name)?;
        for (index, member) in e.members.iter().enumerate() {
            w.start("Member");
            w.attr("Name", member)?;
            w.attr("Value", &index.to_string())?;
            w.end();
        }
        w.end();
    }
    if model.type_description {
        for (name, properties) in TYPE_DESCRIPTION {
            w.start("ComplexType");
            w.attr("Name", name)?;
            for (property, edm) in properties {
                w.start("Property");
                w.attr("Name", property)?;
                w.attr("Type", edm)?;
                w.attr("Nullable", "false")?;
                w.end();
            }
            w.end();
        }
    }
    for set in &model.sets {
        if set.kind == "ТабличнаяЧасть" {
            xml_type(&mut w, model, "ComplexType", &format!("{}_RowType", set.name), set, false)?;
        } else if set.kind == "Регистр" && !set.parent.is_empty() {
            xml_type(&mut w, model, "ComplexType", &format!("{}_RowType", set.parent), set, false)?;
        }
        for table in &set.tables {
            let collection = model.find(table)?;
            if collection.hidden {
                xml_type(&mut w, model, "ComplexType", &collection.name, collection, false)?;
            }
        }
        xml_type(&mut w, model, "EntityType", &set.name, set, true)?;
        for function in set.functions.iter().filter(|f| !f.returns_set) {
            w.start("ComplexType");
            w.attr("Name", &function.type_name)?;
            for property in &function.properties {
                xml_property(&mut w, property, false, true)?;
            }
            w.end();
        }
    }
    for set in &model.sets {
        for function in &set.functions {
            w.start("Function");
            w.attr("Name", &function.name)?;
            w.attr("IsBound", "true")?;
            w.attr("IsComposable", "false")?;
            w.start("Parameter");
            w.attr("Name", "bindingParameter")?;
            w.attr("Type", &format!("Collection(StandardODATA.{})", set.name))?;
            w.end();
            for (name, edm) in &function.parameters {
                w.start("Parameter");
                w.attr("Name", name)?;
                w.attr("Type", edm)?;
                w.attr("Nullable", "true")?;
                w.end();
            }
            w.start("ReturnType");
            let result = if function.returns_set { &set.name } else { &function.type_name };
            w.attr("Type", &format!("Collection(StandardODATA.{result})"))?;
            w.end();
            w.end();
        }
        for action in actions(set) {
            w.start("Action");
            w.attr("Name", action)?;
            w.attr("IsBound", "true")?;
            w.start("Parameter");
            w.attr("Name", "bindingParameter")?;
            w.attr("Type", &format!("StandardODATA.{}", set.name))?;
            w.end();
            w.start("Parameter");
            w.attr("Name", "PostingModeOperational")?;
            w.attr("Type", "Edm.Boolean")?;
            w.attr("Nullable", "true")?;
            w.end();
            w.end();
        }
    }
    w.start("EntityContainer");
    w.attr("Name", "EnterpriseV8")?;
    for set in &model.sets {
        w.start("EntitySet");
        w.attr("Name", &set.name)?;
        w.attr("EntityType", &format!("StandardODATA.{}", set.name))?;
        for property in navigations(set) {
            w.start("NavigationPropertyBinding");
            w.attr("Path", &property.navigation)?;
            w.attr("Target", &property.target)?;
            w.end();
        }
        w.end();
    }
    w.end();
    w.end();
    w.end();
    w.end();
    Ok(w.finish())
}

/// Свойства-ссылки с навигацией (NavigationProperty и привязки набора).
pub fn navigations(set: &Set) -> impl Iterator<Item = &Property> {
    set.properties.iter().filter(|p| p.kind == "Ссылка" && !p.navigation.is_empty())
}

fn xml_type(
    w: &mut XmlWriter,
    model: &Model,
    element: &str,
    name: &str,
    set: &Set,
    with_key: bool,
) -> Result<(), String> {
    w.start(element);
    w.attr("Name", name)?;
    if with_key {
        w.start("Key");
        for property in &set.key {
            w.start("PropertyRef");
            w.attr("Name", property)?;
            w.end();
        }
        w.end();
    }
    for property in &set.properties {
        xml_property(w, property, with_key, false)?;
    }
    for table in &set.tables {
        let collection = model.find(table)?;
        w.start("Property");
        w.attr("Name", &collection.name_in_owner)?;
        w.attr("Type", &format!("Collection(StandardODATA.{})", row_type_name(set, collection)))?;
        w.end();
    }
    if with_key {
        for property in navigations(set) {
            w.start("NavigationProperty");
            w.attr("Name", &property.navigation)?;
            w.attr("Type", &format!("StandardODATA.{}", property.target))?;
            w.end();
        }
    }
    w.end();
    Ok(())
}

/// `ЗаписатьСвойство`: поток Edm.Stream — только в EntityType; `all_nullable` — свойства типа результата
/// виртуальной таблицы.
fn xml_property(w: &mut XmlWriter, p: &Property, in_entity: bool, all_nullable: bool) -> Result<(), String> {
    if in_entity || p.kind != "Хранилище" {
        w.start("Property");
        w.attr("Name", &p.name)?;
        w.attr("Type", &p.edm)?;
        if p.in_key && !all_nullable {
            w.attr("Nullable", "false")?;
        }
        if p.edm == "Edm.String" && p.length > 0 {
            w.attr("MaxLength", &p.length.to_string())?;
        }
        if p.edm == "Edm.Decimal" && p.precision > 0 {
            w.attr("Precision", &p.precision.to_string())?;
            w.attr("Scale", &p.scale.to_string())?;
        }
        w.end();
    }
    if p.kind == "Хранилище" {
        w.start("Property");
        w.attr("Name", &format!("{}_Base64Data", p.name))?;
        w.attr("Type", "Edm.Binary")?;
        w.end();
    }
    if p.kind == "Составной" || p.kind == "Хранилище" {
        w.start("Property");
        w.attr("Name", &format!("{}_Type", p.name))?;
        w.attr("Type", "Edm.String")?;
        w.end();
    }
    Ok(())
}
