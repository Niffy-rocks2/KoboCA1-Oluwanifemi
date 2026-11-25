use crate::ast::{Expr, Program, Resolved, Stmt};
use crate::environment::Environment;
use crate::token::{Token, TokenType};
use crate::value::Value;

pub struct RuntimeError {
    pub line: usize,
    pub message: String,
}

/// Why a statement stopped early. An error unwinds to `main`; a `return` unwinds only as
/// far as the call that started the function, which is why both travel the same way. Rust
/// gives `?` for free on a `Result`, and a `return` is exactly a non-local exit.
enum Flow {
    Error(RuntimeError),
    Return(Value),
}

type Exec<T> = Result<T, Flow>;

/// Kobo's call stack is the Rust call stack: `call` calls `execute` calls `evaluate`
/// calls `call`. Running off the end of it aborts the process with no diagnostic, so the
/// depth is capped instead. Measured on this toolchain, the real limit is near 1,500
/// frames; 500 leaves room for a deeply nested expression inside every one of them.
pub const MAX_FRAMES: usize = 500;

pub fn interpret(program: &Program) -> Result<(), RuntimeError> {
    let mut env = Environment::new();
    for stmt in &program.stmts {
        match execute(stmt, &mut env, program) {
            Ok(()) => {}
            // `return` outside any function ends the program (spec section 3.1).
            Err(Flow::Return(_)) => return Ok(()),
            Err(Flow::Error(e)) => return Err(e),
        }
    }
    Ok(())
}

fn execute(stmt: &Stmt, env: &mut Environment, program: &Program) -> Exec<()> {
    // spec 4: run one statement.
    match stmt {
        Stmt::Expression(e) => {
            evaluate(e, env, program)?;
            Ok(())
        }

        Stmt::Print(e) => {
            let value = evaluate(e, env, program)?;
            println!("{}", value);
            Ok(())
        }

        Stmt::Var {
            name, initialiser, ..
        } => {
            // The initialiser runs BEFORE the name is bound (spec 4.2), so `var a = a + 1;`
            // reads the previous `a`. A `var` with no initialiser holds nil.
            let value = match initialiser {
                Some(e) => evaluate(e, env, program)?,
                None => Value::Nil,
            };
            env.declare(&name.lexeme, value);
            Ok(())
        }

        Stmt::Block { body, .. } => {
            env.push_scope();
            let mut result = Ok(());
            for s in body {
                result = execute(s, env, program);
                if result.is_err() {
                    break;
                }
            }
            env.pop_scope();
            result
        }

        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            if evaluate(condition, env, program)?.is_truthy() {
                execute(then_branch, env, program)
            } else if let Some(alt) = else_branch {
                execute(alt, env, program)
            } else {
                Ok(())
            }
        }

        Stmt::While { condition, body } => {
            while evaluate(condition, env, program)?.is_truthy() {
                execute(body, env, program)?;
            }
            Ok(())
        }

        // Binds the function's name to a value that remembers which entry of
        // Program::functions it is. The body is not run here.
        Stmt::Function { index, name, .. } => {
            env.declare(
                name,
                Value::Function {
                    index: *index,
                    name: name.clone(),
                },
            );
            Ok(())
        }

        // `return` is not an error, but it unwinds the same way: up to the call.
        Stmt::Return(value) => {
            let v = match value {
                Some(e) => evaluate(e, env, program)?,
                None => Value::Nil,
            };
            Err(Flow::Return(v))
        }
    }
}

fn evaluate(expr: &Expr, env: &mut Environment, program: &Program) -> Exec<Value> {
    // spec 2 and 4.1: evaluate one expression to a Value.
    match expr {
        Expr::Literal { value, .. } => Ok(value.clone()),

        Expr::Grouping(inner) => evaluate(inner, env, program),

        // The resolver already decided which declaration this name is. A local is read
        // by its depth, with no search; a global stays a lookup by name (spec 7).
        Expr::Variable { name, at } => {
            let found = match at {
                Resolved::Local { depth, .. } => env.get_at(*depth, &name.lexeme),
                Resolved::Global => env.get_global(&name.lexeme),
            };
            found.cloned().ok_or_else(|| undefined(name))
        }

        Expr::Assign { name, value, at } => {
            let v = evaluate(value, env, program)?;
            let assigned = match at {
                Resolved::Local { depth, .. } => {
                    env.assign_at(*depth, &name.lexeme, v.clone())
                }
                Resolved::Global => env.assign_global(&name.lexeme, v.clone()),
            };
            if assigned {
                Ok(v)
            } else {
                Err(undefined(name))
            }
        }

        Expr::Unary { op, right } => {
            let v = evaluate(right, env, program)?;
            match op.kind {
                TokenType::Minus => match v {
                    Value::Number(n) => Ok(Value::Number(-n)),
                    _ => Err(err(op.line, "Operand must be a number.")),
                },
                // `!` works on any value: it asks only whether it is falsey (spec 4.1).
                _ => Ok(Value::Bool(!v.is_truthy())),
            }
        }

        // `and` / `or` evaluate to one of their operands, not to a boolean, and the right
        // side runs only if the left did not already settle it (spec 4.1).
        Expr::Logical { left, op, right } => {
            let l = evaluate(left, env, program)?;
            let settled = match op.kind {
                TokenType::Or => l.is_truthy(),
                _ => !l.is_truthy(),
            };
            if settled {
                Ok(l)
            } else {
                evaluate(right, env, program)
            }
        }

        Expr::Binary { left, op, right } => {
            let l = evaluate(left, env, program)?;
            let r = evaluate(right, env, program)?;
            let line = op.line;
            match op.kind {
                TokenType::Plus => match (l, r) {
                    (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
                    (Value::Str(a), Value::Str(b)) => Ok(Value::Str(a + &b)),
                    _ => Err(err(
                        line,
                        "Operands must be two numbers or two strings.",
                    )),
                },
                TokenType::Minus => arith(l, r, line, |a, b| a - b),
                TokenType::Star => arith(l, r, line, |a, b| a * b),
                TokenType::Slash => {
                    let (a, b) = numbers(l, r, line)?;
                    // `b == 0.0` is also true for -0, which is what the spec wants.
                    if b == 0.0 {
                        Err(err(line, "Division by zero."))
                    } else {
                        Ok(Value::Number(a / b))
                    }
                }
                TokenType::Greater => compare(l, r, line, |a, b| a > b),
                TokenType::GreaterEqual => compare(l, r, line, |a, b| a >= b),
                TokenType::Less => compare(l, r, line, |a, b| a < b),
                TokenType::LessEqual => compare(l, r, line, |a, b| a <= b),
                // Different kinds are never equal and never an error (spec 4.1).
                TokenType::EqualEqual => Ok(Value::Bool(l == r)),
                _ => Ok(Value::Bool(l != r)),
            }
        }

        // Callee first, then the arguments left to right, and only then the checks, so
        // the VM can do exactly the same in the same order.
        Expr::Call { callee, paren, args } => {
            let callee = evaluate(callee, env, program)?;
            let mut values = Vec::new();
            for a in args {
                values.push(evaluate(a, env, program)?);
            }
            match callee {
                Value::Function { index, .. } => call(index, values, paren.line, env, program),
                _ => Err(err(paren.line, "This value is not a function.")),
            }
        }
    }
}

/// Run a function body in a fresh frame. Everything about Kobo's scope rules is in these
/// twenty lines: the parameters are declared in the frame's own scope, the body can reach
/// past it only to the globals, and the frame is popped on every path out: normal
/// completion, `return`, or a runtime error.
fn call(
    index: usize,
    args: Vec<Value>,
    line: usize,
    env: &mut Environment,
    program: &Program,
) -> Exec<Value> {
    let function = &program.functions[index];
    if args.len() != function.params.len() {
        return Err(err(
            line,
            &format!(
                "Expected {} arguments but got {}.",
                function.params.len(),
                args.len()
            ),
        ));
    }
    if env.frame_depth() >= MAX_FRAMES {
        return Err(err(line, "Stack overflow."));
    }

    env.push_frame();
    for (param, arg) in function.params.iter().zip(args) {
        env.declare(param, arg);
    }

    // Three ways out: the body ends (nil), a `return` (its value), or an error.
    let mut result = Ok(Value::Nil);
    for stmt in &function.body {
        match execute(stmt, env, program) {
            Ok(()) => {}
            Err(Flow::Return(v)) => {
                result = Ok(v);
                break;
            }
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    env.pop_frame();
    result
}

fn numbers(l: Value, r: Value, line: usize) -> Exec<(f64, f64)> {
    match (l, r) {
        (Value::Number(a), Value::Number(b)) => Ok((a, b)),
        _ => Err(err(line, "Operands must be numbers.")),
    }
}

fn arith(l: Value, r: Value, line: usize, f: fn(f64, f64) -> f64) -> Exec<Value> {
    let (a, b) = numbers(l, r, line)?;
    Ok(Value::Number(f(a, b)))
}

fn compare(l: Value, r: Value, line: usize, f: fn(f64, f64) -> bool) -> Exec<Value> {
    let (a, b) = numbers(l, r, line)?;
    Ok(Value::Bool(f(a, b)))
}

fn undefined(name: &Token) -> Flow {
    err(name.line, &format!("Variable '{}' is not defined.", name.lexeme))
}

fn err(line: usize, message: &str) -> Flow {
    Flow::Error(RuntimeError {
        line,
        message: message.to_string(),
    })
}