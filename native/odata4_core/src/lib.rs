//! Разбор параметров запроса OData v4 для расширения OData4 (1С). Дерево разбора, тексты и позиции
//! ошибок совпадают с `OData4_Разбор` (язык 1С); общие примеры — `tests/fixtures/*.json`.
//! Этап 8: выдача OData JSON по строкам во внутреннем формате 1С — `output` (разбор формата — `internal`), примеры
//! `tests/fixtures/output/*.json`; разбор тел запросов (JSON-тела записи, `$batch`) — `body`, `batch`, результат во
//! внутреннем формате 1С (`onec`), примеры `tests/fixtures/body/*.json`; вывод `$metadata` (CSDL XML и JSON)
//! побайтно как `OData4_Метаданные` — `csdl`, примеры `tests/fixtures/metadata/*`.
//! Без `unsafe` и без зависимостей от 1С.
#![forbid(unsafe_code)]

pub mod api;
pub mod ast;
pub mod batch;
pub mod body;
pub mod csdl;
pub mod csdl_json;
pub mod csdl_xml;
pub mod error;
pub mod internal;
mod letters;
pub mod lexer;
pub mod lists;
pub mod literal;
pub mod onec;
pub mod options;
pub mod output;
pub mod parser;
pub mod text;

pub use api::request_json;
pub use batch::batch_internal;
pub use body::body_internal;
pub use csdl::csdl_document;
pub use error::ParseError;
pub use lists::{parse_expand, parse_orderby, parse_select};
pub use options::parse_options;
pub use parser::parse_filter;
