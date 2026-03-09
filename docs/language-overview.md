# Language Overview

Checkbe is a statically checked, compiled language with:

- Primitive types: `int`, `float`, `string`, `bool`
- Arrays: `int[]`, `string[]`, nested arrays like `int[][]`
- Functions with typed parameters and optional return types
- Control flow: `if`, `while`, `do/while`, `for`
- Imports for runtime modules (`import Bridge`)
- Optional access constraints via `capability` and `right`
- String interpolation using `\(expr)`

## Program Structure

A source file is organized as:

1. Zero or more `import` declarations
2. Exactly one `body <Name> { ... }` block

Example:

```checkbe
import Bridge

body Application {
    func main() {
        Bridge.println("Hello, world!")
    }
}
```

## Entry Point Rules

A program must define `main`.

Allowed signatures:

- `func main()`
- `func main(argc: int, argv: string[])`

`main` must return `void` (no `-> type`).

## Comments and Statement Terminators

- Line comments: `// comment`
- Semicolons are generally optional in current parser implementation.

## Multi-file Compilation

You can pass multiple `.checkbe` files to the compiler:

```bash
checkbe a.checkbe b.checkbe c.checkbe -o app
```

The compiler merges sources into one program model. Local function names from non-primary files are internally renamed to avoid symbol collisions.
