use crate::ast::{Expr, Function, Program, Resolved, Stmt};
use crate::token::{one_line, Token, TokenType};
use crate::value::Value;

/// Recursive descent. Every rule in the grammar is one function, and the call order of
/// those functions *is* the precedence table.
pub fn parse(tokens: Vec<Token>) -> (Program, Vec<String>) {
    let mut p = Parser {
        tokens,
        current: 0,
        depth: 0,
        program: Program::default(),
        errors: Vec::new(),
    };
    while !p.at_end() {
        match p.declaration() {
            Ok(s) => p.program.stmts.push(s),
            Err(()) => p.synchronise(),
        }
    }
    (p.program, p.errors)
}

struct Parser {
    tokens: Vec<Token>,
    current: usize,
    /// How many grammar rules are currently on the Rust call stack. Recursive descent
    /// borrows the host language's stack, so a deeply nested source file would overflow
    /// it and abort the process. A table-driven parser has no such limit; this is the
    /// price of the technique, and the reason for `MAX_NESTING`.
    depth: usize,
    /// Built as the parse runs: `funDecl` appends to `program.functions` and hands back
    /// the index. Only `stmts` is filled in at the end, by `parse`.
    program: Program,
    errors: Vec<String>,
}

/// Deep enough that no honest program reaches it, shallow enough that the Rust stack
/// does not: measured overflow is past a thousand levels on this toolchain.
const MAX_NESTING: usize = 250;

type Parsed<T> = Result<T, ()>;

impl Parser {
    // --- declarations and statements ------------------------------------------------

    fn declaration(&mut self) -> Parsed<Stmt> {
        // spec 3.1: declaration -> funDecl | varDecl | statement
        if self.matches(&[TokenType::Fun]) {
            self.fun_declaration()
        } else if self.matches(&[TokenType::Var]) {
            self.var_declaration()
        } else {
            self.statement()
        }
    }

    fn fun_declaration(&mut self) -> Parsed<Stmt> {
        // spec 3.1: 'fun' IDENTIFIER '(' parameters? ')' block   ('fun' already matched)
        // ASSUMED: Function { name, params, body } and Stmt::Function(index).
        let name = self.consume(TokenType::Identifier, "Expect function name.")?;
        self.consume(TokenType::LParen, "Expect '(' after function name.")?;
        let mut params: Vec<Token> = Vec::new();
        if !self.check(TokenType::RParen) {
            loop {
                params.push(self.consume(TokenType::Identifier, "Expect parameter name.")?);
                if !self.matches(&[TokenType::Comma]) {
                    break;
                }
            }
        }
        self.consume(TokenType::RParen, "Expect ')' after parameters.")?;
        self.consume(TokenType::LBrace, "Expect '{' before function body.")?;
        let body = self.block()?;
        self.program.functions.push(Function { name, params, body });
        let index = self.program.functions.len() - 1;
        Ok(Stmt::Function(index))
    }

    fn var_declaration(&mut self) -> Parsed<Stmt> {
        // spec 3.1: 'var' name ( '=' expression )? ';'   ('var' already matched)
        let name = self.consume(TokenType::Identifier, "Expect variable name.")?;
        let initialiser = if self.matches(&[TokenType::Equal]) {
            Some(self.expression()?)
        } else {
            None
        };
        self.consume(
            TokenType::Semicolon,
            "Expect ';' after variable declaration.",
        )?;
        Ok(Stmt::Var {
            name,
            initialiser,
            slot: None,
        })
    }

    fn statement(&mut self) -> Parsed<Stmt> {
        self.depth += 1;
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }

    fn statement_inner(&mut self) -> Parsed<Stmt> {
        // spec 3.1: pick the statement by its leading token.
        // Keywords cannot begin an expression, so these arms come before print and the
        // expression statement. No two arms match the same token.
        if self.matches(&[TokenType::If]) {
            self.if_statement()
        } else if self.matches(&[TokenType::While]) {
            self.while_statement()
        } else if self.matches(&[TokenType::Return]) {
            self.return_statement()
        } else if self.matches(&[TokenType::LBrace]) {
            let stmts = self.block()?;
            Ok(Stmt::Block { body: stmts, locals: 0 })
        } else if self.matches(&[TokenType::Print]) {
            let value = self.expression()?;
            self.consume(TokenType::Semicolon, "Expect ';' after value.")?;
            Ok(Stmt::Print(value))
        } else {
            let expr = self.expression()?;
            self.consume(TokenType::Semicolon, "Expect ';' after expression.")?;
            Ok(Stmt::Expression(expr))
        }
    }

    fn return_statement(&mut self) -> Parsed<Stmt> {
        // spec 3.1: 'return' expression? ';'   ('return' already matched)
        // ASSUMED: Stmt::Return { keyword, value }.
        let keyword = self.previous();
        let value = if !self.check(TokenType::Semicolon) {
            Some(self.expression()?)
        } else {
            None
        };
        self.consume(TokenType::Semicolon, "Expect ';' after return value.")?;
        Ok(Stmt::Return { keyword, value })
    }

    fn if_statement(&mut self) -> Parsed<Stmt> {
        // spec 3.1: 'if' '(' expression ')' statement ( 'else' statement )?
        // ('if' already matched)
        self.consume(TokenType::LParen, "Expect '(' after 'if'.")?;
        let condition = self.expression()?;
        self.consume(TokenType::RParen, "Expect ')' after if condition.")?;
        // statement(), not block(): the body of an if is any statement.
        let then_branch = Box::new(self.statement()?);
        // The else is taken the moment it is seen. Whichever `if` is running when the
        // `else` token arrives takes it, which is the nearest unmatched if.
        let else_branch = if self.matches(&[TokenType::Else]) {
            Some(Box::new(self.statement()?))
        } else {
            None
        };
        Ok(Stmt::If {
            condition,
            then_branch,
            else_branch,
        })
    }

    fn while_statement(&mut self) -> Parsed<Stmt> {
        // spec 3.1: 'while' '(' expression ')' statement   ('while' already matched)
        self.consume(TokenType::LParen, "Expect '(' after 'while'.")?;
        let condition = self.expression()?;
        self.consume(TokenType::RParen, "Expect ')' after while condition.")?;
        let body = Box::new(self.statement()?);
        Ok(Stmt::While { condition, body })
    }

    /// The `{` is already consumed. Declarations, not statements: a block is where a
    /// `var` most often appears, and the whole point of one is that the name dies at `}`.
    fn block(&mut self) -> Parsed<Vec<Stmt>> {
        // spec 3.1: '{' declaration* '}'
        let mut stmts = Vec::new();
        while !self.check(TokenType::RBrace) && !self.at_end() {
            stmts.push(self.declaration()?);
        }
        self.consume(TokenType::RBrace, "Expect '}' after block.")?;
        Ok(stmts)
    }

    // --- expressions, lowest precedence first --------------------------------------

    fn expression(&mut self) -> Parsed<Expr> {
        self.depth += 1;
        let result = if self.depth > MAX_NESTING {
            self.error("Too deeply nested.");
            Err(())
        } else {
            self.assignment()
        };
        self.depth -= 1;
        result
    }

    /// Assignment is right-associative and its left side must be a bare name, but that is
    /// not knowable until the `=` is reached: `a` and `a.b` and `f()` all start the same
    /// way. So the left side is parsed as an ordinary expression and *then* checked, which
    /// is the standard answer to a rule that is not LL(1).
    fn assignment(&mut self) -> Parsed<Expr> {
        // spec 3.2: assignment -> IDENTIFIER '=' assignment | logic_or
        let expr = self.or()?;
        if self.matches(&[TokenType::Equal]) {
            let equals = self.previous();
            let value = self.assignment()?; // right-associative: a = b = 3
            return match expr {
                Expr::Variable { name, .. } => Ok(Expr::Assign {
                    name,
                    value: Box::new(value),
                    at: Resolved::default(),
                }),
                other => {
                    // Reported, not thrown: the parser knows exactly where it is, so there
                    // is nothing to recover from.
                    self.error_at(&equals, "Cannot assign to this expression.");
                    Ok(other)
                }
            };
        }
        Ok(expr)
    }

    fn or(&mut self) -> Parsed<Expr> {
        // spec 3.2: logic_or -> logic_and ( 'or' logic_and )*
        let mut expr = self.and()?;
        while self.matches(&[TokenType::Or]) {
            let op = self.previous();
            let right = self.and()?;
            expr = Expr::Logical {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn and(&mut self) -> Parsed<Expr> {
        // spec 3.2: logic_and -> equality ( 'and' equality )*
        let mut expr = self.equality()?;
        while self.matches(&[TokenType::And]) {
            let op = self.previous();
            let right = self.equality()?;
            expr = Expr::Logical {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn equality(&mut self) -> Parsed<Expr> {
        // spec 3.2: equality -> comparison ( ( '!=' | '==' ) comparison )*
        let mut expr = self.comparison()?;
        while self.matches(&[TokenType::BangEqual, TokenType::EqualEqual]) {
            let op = self.previous();
            let right = self.comparison()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn comparison(&mut self) -> Parsed<Expr> {
        // spec 3.2: comparison -> term ( ( '>' | '>=' | '<' | '<=' ) term )*
        let mut expr = self.term()?;
        while self.matches(&[
            TokenType::Greater,
            TokenType::GreaterEqual,
            TokenType::Less,
            TokenType::LessEqual,
        ]) {
            let op = self.previous();
            let right = self.term()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn term(&mut self) -> Parsed<Expr> {
        // spec 3.2: term -> factor ( ( '-' | '+' ) factor )*
        let mut expr = self.factor()?;
        while self.matches(&[TokenType::Minus, TokenType::Plus]) {
            let op = self.previous();
            let right = self.factor()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn factor(&mut self) -> Parsed<Expr> {
        // spec 3.2: factor -> unary ( ( '/' | '*' ) unary )*
        let mut expr = self.unary()?;
        while self.matches(&[TokenType::Slash, TokenType::Star]) {
            let op = self.previous();
            let right = self.unary()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn unary(&mut self) -> Parsed<Expr> {
        // spec 3.2: unary -> ( '!' | '-' ) unary | call   (right-associative: !!x)
        if self.matches(&[TokenType::Bang, TokenType::Minus]) {
            let op = self.previous();
            let right = self.unary()?;
            return Ok(Expr::Unary {
                op,
                right: Box::new(right),
            });
        }
        self.call()
    }

    /// A loop, not recursion: `f(1)(2)` is one callee called twice, and the grammar says
    /// `call → primary ( "(" arguments? ")" )*` for exactly that reason.
    fn call(&mut self) -> Parsed<Expr> {
        // spec 3.2: call -> primary ( '(' arguments? ')' )*
        // ASSUMED: Expr::Call { callee, paren, arguments } and the message below.
        let mut expr = self.primary()?;
        while self.matches(&[TokenType::LParen]) {
            let mut arguments = Vec::new();
            if !self.check(TokenType::RParen) {
                loop {
                    arguments.push(self.expression()?);
                    if !self.matches(&[TokenType::Comma]) {
                        break;
                    }
                }
            }
            let paren = self.consume(TokenType::RParen, "Expect ')' after arguments.")?;
            expr = Expr::Call {
                callee: Box::new(expr),
                paren,
                args: arguments,
            };
        }
        Ok(expr)
    }

    fn primary(&mut self) -> Parsed<Expr> {
        // spec 3.2: NUMBER | STRING | 'true' | 'false' | 'nil' | IDENTIFIER | '(' expression ')'
        if self.matches(&[TokenType::Number]) {
            let n = self.previous().lexeme.parse::<f64>().unwrap();
            return Ok(self.literal(Value::Number(n)));
        }
        // TODO: string literals. The variant names in token.rs / value.rs are not what I
        //       guessed (TokenType::String and Value::String do not exist). Added next.
        if self.matches(&[TokenType::True]) {
            return Ok(self.literal(Value::Bool(true)));
        }
        if self.matches(&[TokenType::False]) {
            return Ok(self.literal(Value::Bool(false)));
        }
        if self.matches(&[TokenType::Nil]) {
            return Ok(self.literal(Value::Nil));
        }
        if self.matches(&[TokenType::Identifier]) {
            // ASSUMED: Expr::Variable { name, at }
            return Ok(Expr::Variable {
                name: self.previous(),
                at: Resolved::default(),
            });
        }
        if self.matches(&[TokenType::LParen]) {
            let expr = self.expression()?;
            self.consume(TokenType::RParen, "Expect ')' after expression.")?;
            return Ok(Expr::Grouping(Box::new(expr)));
        }
        self.error("Expect expression.");
        Err(())
    }

    // --- error recovery ---------------------------------------------------------------

    /// Discard tokens until the parser is plausibly at the start of a statement again:
    /// just past a `;`, or sitting on a keyword that can only begin a declaration or
    /// statement. This is why one missing semicolon costs one error and not forty
    /// (spec section 5.2).
    ///
    /// It always steps over at least one token before it looks, which is what guarantees
    /// the parse makes progress; a recovery that could stand still would hang the
    /// compiler on the file that triggered it.
    fn synchronise(&mut self) {
        // spec 5.2: stop just past a ';' or on fun var if while print return.
        self.depth = 0;
        while !self.at_end() {
            self.current += 1; // step first, always: this is what prevents a hang
            if self.previous().kind == TokenType::Semicolon {
                return;
            }
            if matches!(
                self.peek().kind,
                TokenType::Fun
                    | TokenType::Var
                    | TokenType::If
                    | TokenType::While
                    | TokenType::Print
                    | TokenType::Return
            ) {
                return;
            }
        }
    }

    /// Wraps a value as a literal node carrying the line of the token just consumed.
    fn literal(&self, value: Value) -> Expr {
        Expr::Literal {
            value,
            line: self.previous().line,
        }
    }

    // --- primitives ---------------------------------------------------------------

    fn matches(&mut self, kinds: &[TokenType]) -> bool {
        for k in kinds {
            if self.check(*k) {
                self.current += 1;
                return true;
            }
        }
        false
    }

    fn check(&self, kind: TokenType) -> bool {
        !self.at_end() && self.peek().kind == kind
    }

    fn at_end(&self) -> bool {
        self.peek().kind == TokenType::Eof
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> Token {
        self.tokens[self.current - 1].clone()
    }

    fn consume(&mut self, kind: TokenType, message: &str) -> Parsed<Token> {
        if self.check(kind) {
            self.current += 1;
            return Ok(self.previous());
        }
        self.error(message);
        Err(())
    }

    fn error(&mut self, message: &str) {
        let t = self.peek().clone();
        self.error_at(&t, message);
    }

    fn error_at(&mut self, t: &Token, message: &str) {
        let at = if t.kind == TokenType::Eof {
            "end".to_string()
        } else {
            format!("'{}'", one_line(&t.lexeme))
        };
        self.errors
            .push(format!("[line {}] Error at {}: {}", t.line, at, message));
    }
}