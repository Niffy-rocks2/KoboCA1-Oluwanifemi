# CA2 Reflection

## 1. The dangling else

The dangling else is resolved by `src/parser.rs:160` and `src/parser.rs:163`. At line 160, `if_statement` parses its body with `self.statement()`, which can itself be an `if`, and that inner call runs to completion before control comes back. At line 163, `self.matches(&[TokenType::Else])` consumes an `else` the moment it is the next token. So when the `else` arrives, the inner `if` is the function that is running, and it takes it. The grammar cannot say this, because `if (c) stmt ( else stmt )?` is ambiguous on its own; it is the order of the calls plus eager matching that decide it. For `if (false) if (true) print "a"; else print "b";` my parser prints `(if false (if true (print "a") (print "b")))`. If the `else` bound to the outer `if`, it would print `(if false (if true (print "a")) (print "b"))`, and the program would behave differently: with a false condition it would print "b".

## 2. Assignment

The check is at `src/parser.rs:220`, where `match expr` tests whether the left side turned out to be an `Expr::Variable`. Line 216 first parses the left side as an ordinary expression with `self.or()`. The check cannot happen earlier, because when the parser reads `a` it cannot tell whether it is the start of an assignment (`a = 1`) or just an expression (`a + 1`, `f(a)`). They begin with the same token, and one token of lookahead does not separate them. Only after the whole left side is built, and an `=` follows, can I look at what I built. That is why assignment is not LL(1). If it is not a variable, line 229 reports the error and returns the left side instead of failing, since the parser has not lost its place. For `1 = 2;` the message from section 5.1 is `[line 1] Error at '=': Cannot assign to this expression.`

## 3. synchronise

`synchronise` starts at `src/parser.rs:422`. It steps over one token first (`src/parser.rs:426`), so it can never stand still and hang. Then it stops in two situations: just past a `;` (`src/parser.rs:427`), because that is where a statement ends, or when the next token is `fun`, `var`, `if`, `while`, `print` or `return` (`src/parser.rs:430`), because those keywords can only begin a declaration or statement. Stopping there means one mistake costs one message instead of a cascade. The test `tests/phase-2/invalid/expect_variable_name.kobo` (`var 123 = 1;`) changes if the call to `synchronise` in `parse` is deleted. The failing `consume` does not advance, so the parser restarts on `123`, reads `123 = 1;` as an assignment, and adds a second line, `[line 1] Error at '=': Cannot assign to this expression.`

## Anchored in my history

- **Still wrong:** commit `f48b79f` (29 Oct 2026). `synchronise` could only stop after a semicolon, and that was its only exit: `if self.previous().kind == TokenType::Semicolon {`
- **Fixed:** commit `3d19e28` (30 Oct 2026). I added the second exit, on the statement keywords: `if matches!(self.peek().kind, TokenType::Fun | TokenType::Var | TokenType::If | TokenType::While | TokenType::Print | TokenType::Return) {`
