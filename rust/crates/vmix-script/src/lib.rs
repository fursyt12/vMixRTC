//! Скриптовый слой vMixUTC: выражения в параметрах и интерпретатор команд кнопки.
//!
//! Соответствие оригиналу:
//! * выражения — `vMixControlExpressionHelper.CalculateExpression` (мини-вычислитель
//!   вместо NCalc, с функцией `_('путь')` для чтения состояния vMix);
//! * интерпретатор — `vMixControlButton.ExecutionThread` (стек условий, `GoTo`,
//!   `Timer`, переменные, нативные функции UTC).

mod expression;
mod interpreter;

pub use expression::{evaluate, format_number, EmptyContext, ExpressionContext, Value};
pub use interpreter::{
    commands_of_widget, resolve_state_path, state_by_xpath, FunctionSender, ScriptOutcome,
    ScriptRunner,
};
