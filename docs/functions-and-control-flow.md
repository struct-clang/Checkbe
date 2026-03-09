# Functions and Control Flow

## Function Declarations

Syntax:

```checkbe
func name(param1: type, param2: type) -> returnType {
    // statements
}
```

If `-> returnType` is omitted, return type is `void`.

Examples:

```checkbe
func add(a: int, b: int) -> int {
    return a + b
}

func logMessage(msg: string) {
    Bridge.println(msg)
}
```

## Return Rules

- `void` function: `return` with no value, or no return.
- Non-void function: must return a compatible value on at least one analyzed path.

## If / Else

```checkbe
if (x > 0) {
    Bridge.println("positive")
} else if (x == 0) {
    Bridge.println("zero")
} else {
    Bridge.println("negative")
}
```

## While

```checkbe
while (i < 10) {
    i = i + 1
}
```

## Do / While

```checkbe
do {
    i = i + 1
} while (i < 10)
```

## For

Supported C-style form:

```checkbe
for (let int i = 0; i < 10; i = i + 1) {
    Bridge.println(i)
}
```

Notes:

- Init clause may contain `let` declaration or expression.
- Update clause cannot contain `let` declaration.
- Internally lowered to block + `while` during parsing.

## Statement Blocks

```checkbe
{
    let int x = 1
    x = x + 1
}
```

Local declarations are block-scoped in semantic analysis.

## Unsupported Statements

No parser support for `break` or `continue` in current implementation.
