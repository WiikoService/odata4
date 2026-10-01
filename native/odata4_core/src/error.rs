//! Ошибки разбора: статус HTTP (400 или 501) и текст — дословно как у `OData4_Разбор`.

use crate::text::{string, Unit};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub status: u16,
    pub message: String,
}

impl ParseError {
    pub fn bad_request(message: impl Into<String>) -> ParseError {
        ParseError { status: 400, message: message.into() }
    }

    pub fn not_implemented(message: impl Into<String>) -> ParseError {
        ParseError { status: 501, message: message.into() }
    }
}

/// «$параметр: неверный литерал СЫРОЙ в позиции N».
pub fn bad_literal(param: &str, raw: &[Unit], pos: usize) -> ParseError {
    ParseError::bad_request(format!("{param}: неверный литерал {} в позиции {pos}", string(raw)))
}
