# Syntax and Types

## Keywords

`import`, `body`, `let`, `func`, `return`, `if`, `else`, `while`, `do`, `for`, `new`, `true`, `false`

## Type System

Built-in value types:

- `int`
- `float`
- `string`
- `bool`
- arrays of any supported value type, e.g. `int[]`, `float[]`, `string[]`, `bool[]`

## Variable Declarations

Variables use `let` and currently require an explicit type in parser rules.

```checkbe
let int x = 10
let float y = 2.5
let string name = "Alice"
let bool ok = true
let int[] nums = [1, 2, 3]
```

Optional entitlement annotation:

```checkbe
let:roCap int secureValue = 1
```

## Literals

- Integer: `123`
- Float: `3.14`
- String: `"text"`
- Boolean: `true`, `false`
- Array: `[1, 2, 3]`

Empty array literals are not allowed without clear type inference and fail semantic checks.

## Expressions and Operators

### Unary

- Numeric negation: `-value`
- Boolean negation: `!flag`

### Binary

- Arithmetic: `+`, `-`, `*`, `/`, `%` (numeric only)
- Equality: `==`, `!=`
- Comparison: `>`, `>=`, `<`, `<=` (numeric only)
- Logical: `&&`, `||` (bool only)

### Ternary

```checkbe
let int n = ok ? 1 : 0
```

Condition must be `bool`. Branches must be compatible (numeric branches can promote to `float`).

## Assignment

Supported assignment targets:

- Variable: `x = 20`
- Array index: `arr[i] = 42`

## Indexing

```checkbe
let int first = arr[0]
```

Index must be `int`.

## String Interpolation

Interpolation syntax inside string literals:

```checkbe
"Hello, \(name)! Age: \(age)"
```

Interpolation expression is parsed as a real expression fragment.
Supported resulting types inside interpolation: `int`, `float`, `string`, `bool`, and `string[]`.

## Built-in Conversion

`Int(value)` converts from:

- `int`
- `float`
- `bool`
- `string`

Example:

```checkbe
let int n = Int("42")
```
