//! Дерево разбора — формат раздела «Дерево разбора» плана этапа 2 (общий с `OData4_Разбор`).

use serde_json::{json, Map, Value};

use crate::literal::Literal;

/// Узел выражения. `Cast` — `cast(выражение, 'Тип')` или `cast('Тип')` (operand — `None`); имя типа — как записано.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Literal(Literal),
    Path { variable: String, segments: Vec<String> },
    Binary { op: String, left: Box<Node>, right: Box<Node> },
    Unary { op: &'static str, operand: Box<Node> },
    In { operand: Box<Node>, items: Vec<Node> },
    Call { name: String, args: Vec<Node> },
    Cast { operand: Option<Box<Node>>, ty: String },
    Lambda { op: String, collection: Box<Node>, variable: String, predicate: Option<Box<Node>> },
}

/// Элемент `$orderby`.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderItem {
    pub expression: Node,
    pub direction: &'static str,
}

/// Элемент `$expand`: путь (`["*"]` — все) и вложенные параметры.
#[derive(Clone, Debug, PartialEq)]
pub struct ExpandItem {
    pub path: Vec<String>,
    pub options: ExpandOptions,
}

/// Параметры коллекции и раскрытия (`OData4_Разбор.НовыеОпции`): отсутствующие — `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpandOptions {
    pub filter: Option<Node>,
    pub orderby: Option<Vec<OrderItem>>,
    pub select: Option<Vec<Vec<String>>>,
    pub expand: Option<Vec<ExpandItem>>,
    pub top: Option<u64>,
    pub skip: Option<u64>,
    pub count: Option<bool>,
}

/// Параметры запроса целиком (`OData4_Разбор.РазобратьПараметры`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub base: ExpandOptions,
    pub skiptoken: Option<String>,
    pub format: Option<String>,
}

impl Node {
    pub fn to_json(&self) -> Value {
        match self {
            Node::Literal(l) => json!({"kind": "literal", "type": l.ty, "value": l.value}),
            Node::Path { variable, segments } => json!({"kind": "path", "segments": segments, "variable": variable}),
            Node::Binary { op, left, right } => {
                json!({"kind": "binary", "left": left.to_json(), "op": op, "right": right.to_json()})
            }
            Node::Unary { op, operand } => json!({"kind": "unary", "op": op, "operand": operand.to_json()}),
            Node::In { operand, items } => json!({
                "items": items.iter().map(Node::to_json).collect::<Vec<_>>(),
                "kind": "in",
                "operand": operand.to_json()
            }),
            Node::Call { name, args } => json!({
                "args": args.iter().map(Node::to_json).collect::<Vec<_>>(),
                "kind": "call",
                "name": name
            }),
            Node::Cast { operand, ty } => json!({
                "kind": "cast",
                "operand": operand.as_ref().map(|o| o.to_json()),
                "type": ty
            }),
            Node::Lambda { op, collection, variable, predicate } => json!({
                "collection": collection.to_json(),
                "kind": "lambda",
                "op": op,
                "predicate": predicate.as_ref().map(|p| p.to_json()),
                "variable": variable
            }),
        }
    }
}

pub fn orderby_json(items: &[OrderItem]) -> Value {
    Value::Array(
        items.iter().map(|i| json!({"direction": i.direction, "expression": i.expression.to_json()})).collect(),
    )
}

pub fn select_json(paths: &[Vec<String>]) -> Value {
    json!(paths)
}

pub fn expand_json(items: &[ExpandItem]) -> Value {
    Value::Array(items.iter().map(|i| json!({"options": i.options.to_json(), "path": i.path})).collect())
}

impl ExpandOptions {
    /// Ключи по возрастанию, как `Канон` тестов 1С.
    fn fill(&self, map: &mut Map<String, Value>) {
        map.insert("count".into(), json!(self.count));
        map.insert("expand".into(), self.expand.as_deref().map_or(Value::Null, expand_json));
        map.insert("filter".into(), self.filter.as_ref().map_or(Value::Null, Node::to_json));
        map.insert("orderby".into(), self.orderby.as_deref().map_or(Value::Null, orderby_json));
        map.insert("select".into(), self.select.as_deref().map_or(Value::Null, select_json));
        map.insert("skip".into(), json!(self.skip));
        map.insert("top".into(), json!(self.top));
    }

    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        self.fill(&mut map);
        Value::Object(map)
    }
}

impl Options {
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        self.base.fill(&mut map);
        map.insert("format".into(), json!(self.format));
        map.insert("skiptoken".into(), json!(self.skiptoken));
        let mut sorted: Vec<(String, Value)> = map.into_iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        Value::Object(sorted.into_iter().collect())
    }
}
