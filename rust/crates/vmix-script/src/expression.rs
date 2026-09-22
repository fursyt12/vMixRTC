//! Выражения в параметрах команд.
//!
//! Оригинал считает параметры через NCalc (`CalculateExpression`), а состояние читает
//! функцией `_('путь')` — например `_('Overlays[0].ActiveInput') != _('Inputs[{0}].Number')`.
//! Здесь свой маленький вычислитель: литералы, переменные, `_(...)`, арифметика,
//! сравнения и логика. Он покрывает всё, что встречается в реальных контроллерах.

use anyhow::{anyhow, Result};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Number(f64),
    Text(String),
    Bool(bool),
}

impl Value {
    pub fn number(value: f64) -> Self {
        Self::Number(value)
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn as_number(&self) -> f64 {
        match self {
            Self::Number(value) => *value,
            Self::Bool(value) => {
                if *value {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Text(text) => text.trim().parse().unwrap_or(0.0),
            Self::Null => 0.0,
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Number(value) => format_number(*value),
            Self::Text(text) => text.clone(),
            Self::Bool(value) => if *value { "true" } else { "false" }.to_string(),
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Number(value) => *value != 0.0,
            Self::Text(text) => {
                let trimmed = text.trim();
                !trimmed.is_empty()
                    && !trimmed.eq_ignore_ascii_case("false")
                    && trimmed != "0"
            }
            Self::Null => false,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_text())
    }
}

/// Доступ к состоянию и переменным — то, что вычислитель ожидает от окружения.
pub trait ExpressionContext {
    /// Значение из состояния vMix по пути (`_('Overlays[0].ActiveInput')`).
    fn state_value(&self, path: &str) -> Option<String>;

    /// Переменная: `_var12` — локальная, иначе глобальная по имени.
    fn variable(&self, name: &str) -> Option<Value>;
}

/// Окружение без состояния — для тестов и простых случаев.
#[derive(Default)]
pub struct EmptyContext;

impl ExpressionContext for EmptyContext {
    fn state_value(&self, _path: &str) -> Option<String> {
        None
    }

    fn variable(&self, _name: &str) -> Option<Value> {
        None
    }
}

pub fn evaluate(expression: &str, context: &dyn ExpressionContext) -> Result<Value> {
    let mut parser = Parser::new(expression, context)?;
    let value = parser.parse_expression(0)?;
    parser.expect_end()?;
    Ok(value)
}

/// Число → строка как в C# (`InvariantCulture`): 5, а не 5.0.
pub fn format_number(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        let text = format!("{value}");
        text
    }
}

// --------------------------------------------------------------- разбор

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Ident(String),
    Operator(String),
    Open,
    Close,
    Comma,
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let symbol = chars[index];
        match symbol {
            ' ' | '\t' | '\r' | '\n' => index += 1,
            '(' => {
                tokens.push(Token::Open);
                index += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                index += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                index += 1;
            }
            '\'' | '"' => {
                let quote = symbol;
                index += 1;
                let mut text = String::new();
                while index < chars.len() && chars[index] != quote {
                    text.push(chars[index]);
                    index += 1;
                }
                if index >= chars.len() {
                    return Err(anyhow!("незакрытая кавычка в «{input}»"));
                }
                index += 1;
                tokens.push(Token::Text(text));
            }
            '0'..='9' | '.' => {
                let start = index;
                while index < chars.len()
                    && (chars[index].is_ascii_digit() || chars[index] == '.')
                {
                    index += 1;
                }
                let text: String = chars[start..index].iter().collect();
                tokens.push(Token::Number(
                    text.parse().map_err(|_| anyhow!("не число: «{text}»"))?,
                ));
            }
            _ if symbol.is_alphabetic() || symbol == '_' => {
                let start = index;
                while index < chars.len()
                    && (chars[index].is_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                tokens.push(Token::Ident(chars[start..index].iter().collect()));
            }
            _ => {
                // двухсимвольные операторы
                let two: String = chars[index..(index + 2).min(chars.len())]
                    .iter()
                    .collect();
                if matches!(two.as_str(), "!=" | "==" | "<=" | ">=" | "&&" | "||") {
                    tokens.push(Token::Operator(two));
                    index += 2;
                } else if "=<>+-*/%!".contains(symbol) {
                    tokens.push(Token::Operator(symbol.to_string()));
                    index += 1;
                } else {
                    return Err(anyhow!("непонятный символ «{symbol}» в «{input}»"));
                }
            }
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    position: usize,
    context: &'a dyn ExpressionContext,
}

impl<'a> Parser<'a> {
    fn new(expression: &str, context: &'a dyn ExpressionContext) -> Result<Self> {
        Ok(Self {
            tokens: tokenize(expression)?,
            position: 0,
            context,
        })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn expect_end(&self) -> Result<()> {
        if self.position < self.tokens.len() {
            return Err(anyhow!("лишние токены в выражении"));
        }
        Ok(())
    }

    /// Приоритеты: || < && < сравнения < + − < * / % < унарные.
    fn parse_expression(&mut self, min_precedence: u8) -> Result<Value> {
        let mut left = self.parse_unary()?;
        while let Some(Token::Operator(operator)) = self.peek().cloned() {
            let precedence = precedence(&operator);
            if precedence < min_precedence {
                break;
            }
            self.next();
            let right = self.parse_expression(precedence + 1)?;
            left = apply_binary(&operator, left, right)?;
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Value> {
        match self.peek().cloned() {
            Some(Token::Operator(operator)) if operator == "!" => {
                self.next();
                let value = self.parse_unary()?;
                Ok(Value::Bool(!value.as_bool()))
            }
            Some(Token::Operator(operator)) if operator == "-" => {
                self.next();
                let value = self.parse_unary()?;
                Ok(Value::number(-value.as_number()))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Value> {
        match self.next() {
            Some(Token::Number(value)) => Ok(Value::number(value)),
            Some(Token::Text(value)) => Ok(Value::text(value)),
            Some(Token::Open) => {
                let value = self.parse_expression(0)?;
                match self.next() {
                    Some(Token::Close) => Ok(value),
                    _ => Err(anyhow!("ожидалась «)»")),
                }
            }
            Some(Token::Ident(name)) => {
                if matches!(self.peek(), Some(Token::Open)) {
                    self.next();
                    let mut arguments = Vec::new();
                    if !matches!(self.peek(), Some(Token::Close)) {
                        loop {
                            arguments.push(self.parse_expression(0)?);
                            match self.peek() {
                                Some(Token::Comma) => {
                                    self.next();
                                }
                                _ => break,
                            }
                        }
                    }
                    match self.next() {
                        Some(Token::Close) => {}
                        _ => return Err(anyhow!("ожидалась «)» после аргументов {name}")),
                    }
                    call_function(&name, &arguments, self.context)
                } else {
                    match self.context.variable(&name) {
                        Some(value) => Ok(value),
                        // неизвестное имя в оригинале даёт пустое значение
                        None => Ok(Value::Null),
                    }
                }
            }
            other => Err(anyhow!("неожиданный токен: {other:?}")),
        }
    }
}

fn precedence(operator: &str) -> u8 {
    match operator {
        "||" => 1,
        "&&" => 2,
        "=" | "==" | "!=" | "<" | ">" | "<=" | ">=" => 3,
        "+" | "-" => 4,
        "*" | "/" | "%" => 5,
        _ => 0,
    }
}

fn apply_binary(operator: &str, left: Value, right: Value) -> Result<Value> {
    let result = match operator {
        "+" => {
            // «+» склеивает строки, если хотя бы один операнд текст
            if matches!(left, Value::Text(_)) || matches!(right, Value::Text(_)) {
                Value::text(format!("{}{}", left.as_text(), right.as_text()))
            } else {
                Value::number(left.as_number() + right.as_number())
            }
        }
        "-" => Value::number(left.as_number() - right.as_number()),
        "*" => Value::number(left.as_number() * right.as_number()),
        "/" => {
            let divisor = right.as_number();
            if divisor == 0.0 {
                Value::Null
            } else {
                Value::number(left.as_number() / divisor)
            }
        }
        "%" => {
            let divisor = right.as_number();
            if divisor == 0.0 {
                Value::Null
            } else {
                Value::number(left.as_number() % divisor)
            }
        }
        "&&" => Value::Bool(left.as_bool() && right.as_bool()),
        "||" => Value::Bool(left.as_bool() || right.as_bool()),
        "=" | "==" => Value::Bool(compare(&left, &right) == std::cmp::Ordering::Equal),
        "!=" => Value::Bool(compare(&left, &right) != std::cmp::Ordering::Equal),
        "<" => Value::Bool(compare(&left, &right) == std::cmp::Ordering::Less),
        ">" => Value::Bool(compare(&left, &right) == std::cmp::Ordering::Greater),
        "<=" => Value::Bool(compare(&left, &right) != std::cmp::Ordering::Greater),
        ">=" => Value::Bool(compare(&left, &right) != std::cmp::Ordering::Less),
        other => return Err(anyhow!("неизвестный оператор «{other}»")),
    };
    Ok(result)
}

/// Числа сравниваются как числа, остальное — как строки (как в C# при `object`-сравнении).
fn compare(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (left, right) {
        (Value::Number(_), Value::Number(_)) | (Value::Bool(_), Value::Number(_))
        | (Value::Number(_), Value::Bool(_)) => left
            .as_number()
            .partial_cmp(&right.as_number())
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => left.as_text().cmp(&right.as_text()),
    }
}

fn call_function(name: &str, arguments: &[Value], context: &dyn ExpressionContext) -> Result<Value> {
    match name {
        // _('путь') — чтение значения из состояния vMix
        "_" => {
            let path = arguments.first().map(Value::as_text).unwrap_or_default();
            Ok(match context.state_value(&path) {
                Some(value) => Value::text(value),
                None => Value::Null,
            })
        }
        "len" | "Len" => Ok(Value::number(
            arguments.first().map(|value| value.as_text().chars().count()).unwrap_or(0) as f64,
        )),
        other => Err(anyhow!("функция «{other}» пока не поддержана")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Ctx {
        state: BTreeMap<String, String>,
        variables: BTreeMap<String, Value>,
    }

    impl ExpressionContext for Ctx {
        fn state_value(&self, path: &str) -> Option<String> {
            self.state.get(path).cloned()
        }

        fn variable(&self, name: &str) -> Option<Value> {
            self.variables.get(name).cloned()
        }
    }

    fn context() -> Ctx {
        let mut ctx = Ctx::default();
        ctx.state.insert("Recording".into(), "True".into());
        ctx.state.insert("Overlays[0].ActiveInput".into(), "keyA".into());
        ctx.state.insert("Inputs[keyA].Number".into(), "1".into());
        ctx.state.insert("Inputs[keyB].Number".into(), "2".into());
        ctx.variables.insert("_var5".into(), Value::number(42.0));
        ctx
    }

    #[test]
    fn arithmetic_and_precedence() {
        let ctx = context();
        assert_eq!(evaluate("1 + 2 * 3", &ctx).unwrap(), Value::number(7.0));
        assert_eq!(evaluate("(1 + 2) * 3", &ctx).unwrap(), Value::number(9.0));
        assert_eq!(evaluate("-4 + 10 / 2", &ctx).unwrap(), Value::number(1.0));
        assert_eq!(evaluate("10 % 3", &ctx).unwrap(), Value::number(1.0));
    }

    #[test]
    fn comparisons_and_logic() {
        let ctx = context();
        assert_eq!(evaluate("1 < 2", &ctx).unwrap(), Value::Bool(true));
        assert_eq!(evaluate("2 <= 2", &ctx).unwrap(), Value::Bool(true));
        assert_eq!(evaluate("'a' != 'b'", &ctx).unwrap(), Value::Bool(true));
        assert_eq!(evaluate("1 = 1 && 2 > 1", &ctx).unwrap(), Value::Bool(true));
        assert_eq!(evaluate("1 > 2 || 3 > 2", &ctx).unwrap(), Value::Bool(true));
        assert_eq!(evaluate("!0", &ctx).unwrap(), Value::Bool(true));
    }

    #[test]
    fn state_accessor_reads_values() {
        let ctx = context();
        assert_eq!(
            evaluate("_('Recording')", &ctx).unwrap(),
            Value::text("True")
        );
        // как в реальном Scoreboard.vmc: Overlay != Number
        assert_eq!(
            evaluate(
                "_('Overlays[0].ActiveInput') != _('Inputs[keyA].Number')",
                &ctx
            )
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(evaluate("_('НетТакого')", &ctx).unwrap(), Value::Null);
    }

    #[test]
    fn variables_and_strings() {
        let ctx = context();
        assert_eq!(evaluate("_var5 + 1", &ctx).unwrap(), Value::number(43.0));
        assert_eq!(evaluate("'a' + 'b'", &ctx).unwrap(), Value::text("ab"));
        assert_eq!(evaluate("'x' + 5", &ctx).unwrap(), Value::text("x5"));
        assert_eq!(evaluate("len('abcd')", &ctx).unwrap(), Value::number(4.0));
        assert_eq!(evaluate("Неизвестная", &ctx).unwrap(), Value::Null);
    }

    #[test]
    fn division_by_zero_is_null_instead_of_panic() {
        let ctx = context();
        assert_eq!(evaluate("1 / 0", &ctx).unwrap(), Value::Null);
        assert!(!evaluate("1 / 0", &ctx).unwrap().as_bool());
    }

    #[test]
    fn numbers_print_like_c_sharp() {
        assert_eq!(Value::number(5.0).as_text(), "5");
        assert_eq!(Value::number(2.5).as_text(), "2.5");
    }

    #[test]
    fn broken_expressions_are_reported() {
        let ctx = context();
        assert!(evaluate("1 +", &ctx).is_err());
        assert!(evaluate("'незакрытая", &ctx).is_err());
        assert!(evaluate("1 2", &ctx).is_err());
    }
}
