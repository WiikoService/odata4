//! Парсер с рекурсивным спуском — как `OData4_Разбор` (область «Разбор»): те же приоритеты, пределы
//! (текст ≤ 8000, вложенность ≤ 32, членов цепочки ≤ 100, высота дерева ≤ 150), тексты и позиции ошибок.
//! У 1С первая ошибка запоминается и разбор сворачивается обычными возвратами; здесь первая ошибка
//! возвращается сразу (`?`) — результат тот же: побеждает первая ошибка в порядке разбора.

use crate::ast::Node;
use crate::error::ParseError;
use crate::lexer::{tokens, Kind, Token, Value};
use crate::literal::{integer_type, Literal};
use crate::text::{count_digits, eq_ascii, starts_with_dollar, string, Unit};

pub(crate) type R<T> = Result<T, ParseError>;

const MAX_LENGTH: usize = 8000;
const MAX_DEPTH: u32 = 32;
const MAX_MEMBERS: u32 = 100;
const MAX_HEIGHT: u32 = 150;

pub(crate) struct Parser<'p> {
    param: &'p str,
    tokens: Vec<Token>,
    index: usize,
    variables: Vec<Vec<Unit>>,
    depth: u32,
    /// Высота дерева последнего разобранного узла (`Разбор.Высота`).
    height: u32,
}

/// `$filter` → узел выражения.
pub fn parse_filter(text: &[Unit]) -> R<Node> {
    let mut p = Parser::new(text, "$filter")?;
    let node = p.expression()?;
    p.finish()?;
    Ok(node)
}

pub(crate) fn is_name(t: &Token, name: &str) -> bool {
    t.kind == Kind::Name && eq_ascii(&t.text, name)
}

pub(crate) fn is_sign(t: &Token, sign: u8) -> bool {
    t.kind == Kind::Sign && t.text.len() == 1 && t.text[0] == sign as Unit
}

fn is_operator(t: &Token, list: &[&str]) -> bool {
    t.kind == Kind::Name && list.iter().any(|op| eq_ascii(&t.text, op))
}

fn operators(level: u8) -> &'static [&'static str] {
    match level {
        1 => &["or"],
        2 => &["and"],
        3 => &["eq", "ne"],
        4 => &["gt", "ge", "lt", "le"],
        5 => &["add", "sub"],
        _ => &["mul", "div", "divby", "mod"],
    }
}

/// `АрностьФункции`: допустимое число аргументов; `None` — функция неизвестна.
fn arity(name: &str) -> Option<&'static [usize]> {
    match name {
        "contains" | "startswith" | "endswith" | "indexof" | "concat" | "substringof" | "hassubset"
        | "hassubsequence" | "matchesPattern" | "geo.distance" | "geo.intersects" => Some(&[2]),
        "length" | "tolower" | "toupper" | "trim" | "year" | "month" | "day" | "hour" | "minute" | "second"
        | "fractionalseconds" | "totalseconds" | "date" | "time" | "totaloffsetminutes" | "round" | "floor"
        | "ceiling" | "geo.length" => Some(&[1]),
        "now" | "mindatetime" | "maxdatetime" => Some(&[0]),
        "substring" => Some(&[2, 3]),
        _ => None,
    }
}

impl<'p> Parser<'p> {
    /// `НовыйРазбор`: предел длины, затем лексемы (ошибка лексера — сразу).
    pub(crate) fn new(text: &[Unit], param: &'p str) -> R<Parser<'p>> {
        if text.len() > MAX_LENGTH {
            return Err(ParseError::bad_request(format!("{param}: слишком длинное выражение")));
        }
        Ok(Parser { param, tokens: tokens(text, param)?, index: 0, variables: Vec::new(), depth: 0, height: 0 })
    }

    /// Имя параметра запроса для текстов ошибок («$filter», «$expand», …).
    pub(crate) fn param(&self) -> &str {
        self.param
    }

    pub(crate) fn current(&self) -> &Token {
        &self.tokens[self.index]
    }

    /// `Следующая`: текущая лексема; на `Конец` позиция не двигается.
    pub(crate) fn next(&mut self) -> Token {
        let token = self.tokens[self.index].clone();
        if token.kind != Kind::End {
            self.index += 1;
        }
        token
    }

    pub(crate) fn expected(&self, what: &str) -> ParseError {
        ParseError::bad_request(format!("{}: ожидалось {} в позиции {}", self.param, what, self.current().pos))
    }

    pub(crate) fn expect_sign(&mut self, sign: u8) -> R<()> {
        if !is_sign(self.current(), sign) {
            return Err(self.expected(&format!("«{}»", sign as char)));
        }
        self.next();
        Ok(())
    }

    fn not_supported(&self, t: &Token) -> ParseError {
        ParseError::bad_request(format!("{}: не поддерживается {} в позиции {}", self.param, string(&t.text), t.pos))
    }

    fn depth_error(&self, pos: usize) -> ParseError {
        ParseError::bad_request(format!("{}: слишком глубокая вложенность в позиции {}", self.param, pos))
    }

    /// `ПроверитьКонец`: после разбора — только `Конец`.
    pub(crate) fn finish(&self) -> R<()> {
        let t = self.current();
        if t.kind != Kind::End {
            return Err(ParseError::bad_request(format!("{}: лишний текст в позиции {}", self.param, t.pos)));
        }
        Ok(())
    }

    pub(crate) fn enter(&mut self, t: &Token) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.depth_error(t.pos));
        }
        Ok(())
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

    fn check_members(&self, members: u32) -> R<u32> {
        if members >= MAX_MEMBERS {
            return Err(ParseError::bad_request(format!("{}: слишком длинное выражение", self.param)));
        }
        Ok(members + 1)
    }

    fn leaf(&mut self, node: Node) -> Node {
        self.height = 1;
        node
    }

    fn composite(&mut self, node: Node, sub_height: u32, pos: usize) -> R<Node> {
        self.height = sub_height + 1;
        if self.height > MAX_HEIGHT {
            return Err(self.depth_error(pos));
        }
        Ok(node)
    }

    pub(crate) fn expression(&mut self) -> R<Node> {
        self.chain(1)
    }

    fn operand(&mut self, level: u8) -> R<Node> {
        if level == 6 {
            self.unary()
        } else {
            self.chain(level + 1)
        }
    }

    /// `Цепочка`: бинарные операторы уровня 1–6, левоассоциативно.
    fn chain(&mut self, level: u8) -> R<Node> {
        let ops = operators(level);
        let mut left = self.operand(level)?;
        let mut left_height = self.height;
        let mut members = 1;
        while is_operator(self.current(), ops) {
            let op = self.next();
            members = self.check_members(members)?;
            let right = self.operand(level)?;
            let node = Node::Binary { op: string(&op.text), left: Box::new(left), right: Box::new(right) };
            let height = left_height.max(self.height);
            left = self.composite(node, height, op.pos)?;
            left_height = self.height;
        }
        Ok(left)
    }

    /// `Унарное`: `-` (перед числом без пробела — отрицательный литерал), `not`, иначе первичное с постфиксом.
    fn unary(&mut self) -> R<Node> {
        let t = self.current().clone();
        let op = if is_sign(&t, b'-') {
            self.next();
            let operand = self.current().clone();
            if let (Kind::Literal, Value::Literal(lit)) = (operand.kind, &operand.value) {
                if operand.pos == t.pos + 1 && lit.is_numeric() {
                    self.next();
                    let leaf = self.leaf(Node::Literal(negative_literal(&operand, lit)));
                    return self.postfix(leaf);
                }
            }
            "neg"
        } else if is_name(&t, "not") {
            self.next();
            "not"
        } else {
            let primary = self.primary()?;
            return self.postfix(primary);
        };
        self.enter(&t)?;
        let operand = self.unary()?;
        self.leave();
        let height = self.height;
        self.composite(Node::Unary { op, operand: Box::new(operand) }, height, t.pos)
    }

    /// `Постфикс`: `has` и `in` после первичного выражения, левоассоциативно.
    fn postfix(&mut self, first: Node) -> R<Node> {
        let mut left = first;
        let mut left_height = self.height;
        let mut members = 1;
        while is_operator(self.current(), &["has", "in"]) {
            let op = self.next();
            members = self.check_members(members)?;
            let node = if eq_ascii(&op.text, "in") {
                let items = self.expression_list()?;
                Node::In { operand: Box::new(left), items }
            } else {
                let right = self.primary()?;
                Node::Binary { op: "has".to_string(), left: Box::new(left), right: Box::new(right) }
            };
            let height = left_height.max(self.height);
            left = self.composite(node, height, op.pos)?;
            left_height = self.height;
        }
        Ok(left)
    }

    /// `Первичное`: скобки, литерал, строка, true/false/null, вызов функции, путь.
    fn primary(&mut self) -> R<Node> {
        let t = self.current().clone();
        if is_sign(&t, b'(') {
            self.next();
            self.enter(&t)?;
            let node = self.expression()?;
            self.expect_sign(b')')?;
            self.leave();
            return Ok(node);
        }
        match (&t.kind, &t.value) {
            (Kind::Literal, Value::Literal(lit)) => {
                self.next();
                return Ok(self.leaf(Node::Literal(lit.clone())));
            }
            (Kind::Str, Value::Str(content)) => {
                self.next();
                return Ok(self.leaf(Node::Literal(Literal::new("Edm.String", string(content)))));
            }
            (Kind::Name, _) => {}
            _ => return Err(self.expected("выражение")),
        }
        if is_name(&t, "true") || is_name(&t, "false") {
            self.next();
            return Ok(self.leaf(Node::Literal(Literal::new("Edm.Boolean", string(&t.text)))));
        }
        if is_name(&t, "null") {
            self.next();
            return Ok(self.leaf(Node::Literal(Literal::null())));
        }
        if is_name(&t, "INF") || is_name(&t, "NaN") {
            return Err(self.not_supported(&t));
        }
        if self.tokens.get(self.index + 1).is_some_and(|n| is_sign(n, b'(')) {
            return self.call();
        }
        self.path()
    }

    /// `Путь`: Имя {/ Имя}; первое имя может быть переменной лямбды; `/any(…)`, `/all(…)` завершают путь.
    fn path(&mut self) -> R<Node> {
        let first = self.next();
        if starts_with_dollar(&first.text) {
            return Err(self.not_supported(&first));
        }
        let mut variable = String::new();
        let mut segments = Vec::new();
        if self.variables.contains(&first.text) {
            variable = string(&first.text);
        } else {
            segments.push(string(&first.text));
        }
        while is_sign(self.current(), b'/') {
            self.next();
            let t = self.current().clone();
            if t.kind != Kind::Name {
                return Err(self.expected("имя"));
            }
            if starts_with_dollar(&t.text) {
                return Err(self.not_supported(&t));
            }
            self.next();
            if (eq_ascii(&t.text, "any") || eq_ascii(&t.text, "all")) && is_sign(self.current(), b'(') {
                return self.lambda(&t, Node::Path { variable, segments });
            }
            segments.push(string(&t.text));
        }
        Ok(self.leaf(Node::Path { variable, segments }))
    }

    /// `Лямбда`: `any()` или `any|all(переменная: условие)`.
    fn lambda(&mut self, t: &Token, collection: Node) -> R<Node> {
        self.expect_sign(b'(')?;
        self.enter(t)?;
        let mut variable = String::new();
        let mut predicate = None;
        self.height = 1;
        if eq_ascii(&t.text, "any") && is_sign(self.current(), b')') {
            self.next();
        } else {
            let var = self.current().clone();
            if var.kind != Kind::Name {
                return Err(self.expected("имя переменной"));
            }
            self.next();
            self.expect_sign(b':')?;
            variable = string(&var.text);
            self.variables.push(var.text.clone());
            let body = self.expression()?;
            self.variables.pop();
            predicate = Some(Box::new(body));
            self.expect_sign(b')')?;
        }
        self.leave();
        let node = Node::Lambda { op: string(&t.text), collection: Box::new(collection), variable, predicate };
        let height = self.height;
        self.composite(node, height, t.pos)
    }

    /// `ВызовФункции`: известная функция OData с проверкой числа аргументов; v3 `substringof(a, b)` → `contains(b, a)`.
    fn call(&mut self) -> R<Node> {
        let t = self.next();
        let mut name = string(&t.text);
        if name == "isof" {
            return Err(self.not_supported(&t));
        }
        if name == "cast" {
            return self.cast(&t);
        }
        let Some(allowed) = arity(&name) else {
            return Err(ParseError::bad_request(format!(
                "{}: неизвестная функция {} в позиции {}",
                self.param, name, t.pos
            )));
        };
        self.expect_sign(b'(')?;
        self.enter(&t)?;
        let mut args = Vec::new();
        let mut args_height = 0;
        if !is_sign(self.current(), b')') {
            args.push(self.expression()?);
            args_height = self.height;
            while is_sign(self.current(), b',') {
                self.next();
                args.push(self.expression()?);
                args_height = args_height.max(self.height);
            }
        }
        self.expect_sign(b')')?;
        self.leave();
        if !allowed.contains(&args.len()) {
            return Err(ParseError::bad_request(format!(
                "{}: неверное число аргументов функции {} в позиции {}",
                self.param, name, t.pos
            )));
        }
        if name == "substringof" && args.len() == 2 {
            args.swap(0, 1);
            name = "contains".to_string();
        }
        self.composite(Node::Call { name, args }, args_height, t.pos)
    }

    /// `Приведение`: `cast(Выражение, Тип)` или `cast(Тип)`; тип — строка (`'Catalog_X'`, как у standard.odata)
    /// или имя (`StandardODATA.Catalog_X`, OData v4). Поддержку приведения решает генератор.
    fn cast(&mut self, t: &Token) -> R<Node> {
        self.expect_sign(b'(')?;
        self.enter(t)?;
        let first = self.current().clone();
        let one_arg = matches!(first.kind, Kind::Str | Kind::Name)
            && self.tokens.get(self.index + 1).is_some_and(|n| is_sign(n, b')'));
        let mut operand = None;
        let mut height = 0;
        if !one_arg {
            operand = Some(Box::new(self.expression()?));
            height = self.height;
            self.expect_sign(b',')?;
        }
        let name = self.current().clone();
        let ty = match (&name.kind, &name.value) {
            (Kind::Str, Value::Str(content)) => string(content),
            (Kind::Name, _) => string(&name.text),
            _ => return Err(self.expected("имя типа")),
        };
        self.next();
        self.expect_sign(b')')?;
        self.leave();
        self.composite(Node::Cast { operand, ty }, height, t.pos)
    }

    /// `СписокВыражений`: «(Выражение {, Выражение})»; высота — наибольшая из элементов.
    fn expression_list(&mut self) -> R<Vec<Node>> {
        let open = self.current().clone();
        self.expect_sign(b'(')?;
        self.enter(&open)?;
        let mut items = vec![self.expression()?];
        let mut height = self.height;
        while is_sign(self.current(), b',') {
            self.next();
            items.push(self.expression()?);
            height = height.max(self.height);
        }
        self.expect_sign(b')')?;
        self.leave();
        self.height = height;
        Ok(items)
    }
}

/// `ОтрицательныйЛитерал`: литерал со знаком минус; целое без суффикса — диапазон Int64 со знаком.
fn negative_literal(t: &Token, lit: &Literal) -> Literal {
    let mut ty = lit.ty.clone();
    if count_digits(&t.text, 1) == t.text.len() {
        if let Some(checked) = integer_type(&t.text, true) {
            ty = checked.to_string();
        }
    }
    Literal { ty, value: Some(format!("-{}", lit.value.as_deref().unwrap_or(""))) }
}
