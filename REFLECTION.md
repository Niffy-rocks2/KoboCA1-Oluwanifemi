# CA3 Reflection — Compiler Construction

## Question 1: Short-Circuit Evaluation

Short-circuit evaluation allows logical expressions to return a result without unnecessarily evaluating both operands. The implementation in `src/interpreter.rs:179` handles logical expressions, while lines `src/interpreter.rs:182` and `src/interpreter.rs:183` determine whether evaluation should stop based on the logical operator and the truthiness of the left operand.

For the expression `nil or "fallback"`, nil is falsey, so the interpreter evaluates the right operand and returns the string `"fallback"`.

For `0 and 1`, zero is truthy in Kobo. Therefore, the interpreter evaluates the right operand and returns the number `1`.

The important distinction is that logical operators return one of their original operands rather than converting the result into a Boolean. Returning only true or false might pass tests that use logical expressions as conditions. However, it would fail tests that print or store the actual value returned by a logical expression. Short-circuit evaluation also prevents unnecessary operations and unwanted side effects.

## Question 2: Variable Resolution and Stack Management

The lookup function in `src/resolver.rs:96` determines which declaration a local variable refers to before the program executes. This allows the bytecode compiler to associate local variables with stack slots.

Instead of repeatedly searching for a variable by its name, the virtual machine can access the corresponding stack slot directly. This reduces the work required during execution. The tree-walking interpreter also benefits from resolution because it can identify the appropriate environment depth.

The `end_scope` function in `src/compiler.rs:310` is responsible for removing local variables from the stack when their scope ends.

If the compiler emits one Pop instruction too few, an expired local variable remains on the stack. This can disrupt stack discipline and cause incorrect local variable access during subsequent operations.

According to the assignment brief, the phase-4 suite detects this problem through tests such as `jumps_while.kobo`, whereas the phase-3 marking programs do not expose that particular VM defect.

## Question 3: Conditional Expression Extension

The extension is the conditional expression, written as `condition ? a : b`. It evaluates the condition and executes only the selected branch.

The implementation affects the following source locations:

- `src/token.rs:12–13` and `src/token.rs:61–62` — conditional tokens.
- `src/scanner.rs:53–54` — recognition of the new symbols.
- `src/ast.rs:92` — conditional expression node.
- `src/parser.rs:216` and `src/parser.rs:237–254` — precedence and right-associativity.
- `src/printer.rs:88` — expression representation.
- `src/resolver.rs:192` — variable resolution.
- `src/interpreter.rs:169` — conditional evaluation.
- `src/compiler.rs:234` — bytecode generation.

The bytecode implementation is more complicated because it requires conditional jumps, jump patching and correct stack management. Both execution paths must remove the condition and leave exactly one result. The existing VM instruction at `src/vm.rs:147` supports conditional jumps without requiring a new instruction.

The extension was implemented with AI assistance and tested locally on both execution engines. All five extension tests passed on each engine.

### Development History

- `f3605c5` — core VM implementation.
- `faf1090` — conditional expression implementation and golden tests.

The available commit history does not contain a separate incorrect extension implementation followed by its correction. Therefore, I cannot provide a verified before-and-after correction for this requirement.