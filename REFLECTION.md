# REFLECTION

## 1. The `.` and fractional parts

The check at `src/scanner.rs:142` — `if self.peek() == '.' && self.peek_next().is_ascii_digit()` — decides whether a `.` begins a fractional part. It only treats the dot as part of the number if the character *after* it is also a digit, using `peek_next()` to look ahead without consuming anything yet.

For the input `5.`, the first digit loop consumes `5`. Then the check at line 142 fires: `peek()` is `.`, but `peek_next()` is whatever comes after it (end of input or a following token), not a digit — so the condition is false. The function stops there and calls `add(TokenType::Number)`, producing `NUMBER '5'` with the dot left completely unconsumed. The next call to `scan_token` then reads that `.` fresh and falls into the `_` arm, reporting `Character is not part of any token.`, since Kobo has no DOT token type.

This matches section 1.4's rule exactly: "5. is not a number either: 5 is scanned, then the . reports the same error." A number is digits and at most one dot, never a trailing one with nothing after it.

## 2. The line counter and EOF

Two lines change the line counter: `src/scanner.rs:102`, inside `scan_token`'s `'\n'` match arm, handles every ordinary newline outside a string. `src/scanner.rs:122`, inside `string()`'s scanning loop, handles a newline found *while inside* a string literal, since strings may span lines per section 1.5.

For a file ending in two blank lines, each trailing newline is still consumed by line 102 as `scan_token` keeps running, incrementing `self.line` each time — even though no token gets produced for a blank line. By the time the file runs out, `self.line` has moved past the position of the last real token.

EOF doesn't use that live counter, though. In `run` (line 34), `eof_line` is computed as `self.tokens.last().map(|t| t.line).unwrap_or(1)` — the line recorded on the *last token actually pushed*, not wherever `self.line` currently sits. So EOF ends up carrying the line of the last real token, not the file's last line. Section 6.1 asks for this specifically: "a trailing newline left behind by an editor must not move a line number" — otherwise, two files that differ only in trailing whitespace would report different EOF lines for otherwise identical content.

## 3. Testing beyond the given suite

I didn't hit a bug during development that the given 14 tests caught — each function compiled and passed on the first implementation attempt, confirmed by `tool/run-golden phase-1` going from 0/14 to 14/14 across commits `d7a9009` and `b9c34dc`. Rather than invent a failure that didn't happen, I followed the brief's own advice to check rules "none of your tests happens to reach" by hand-testing inputs outside the given suite.

I tried `1e308;` — section 1.4 says there's no exponent notation, so this should scan as `NUMBER '1'` followed by `IDENTIFIER 'e308'`. It did, correctly, because `number()` (lines 137–151) only consumes digits and an optional single dot-plus-digits; it has no notion of `e` at all, so the scan simply stops after `1` and hands `e308` to `identifier()` on the next call.

I also tested a string spanning two lines: `var x = "line one\nline two";`. Section 6.1 says a multi-line token is reported "at the line it ends on." The output showed `STRING` and the following `SEMICOLON`/`EOF` all correctly reported at line 2, confirming `self.line += 1` at line 122 fires correctly for an embedded newline before the closing quote is found.

Both held up without changes, which told me the lookahead-based design (checking `peek`/`peek_next` before committing to consume) generalizes correctly beyond the given test cases rather than being tuned to pass them specifically.
