# Examples and Gotchas

## Minimal Program

```checkbe
import Bridge

body Application {
    func main() {
        Bridge.println("Hello")
    }
}
```

## argc/argv Entry Point

```checkbe
import Bridge

body Application {
    func main(argc: int, argv: string[]) {
        Bridge.println("argc = \(argc)")
        if (argc > 1) {
            Bridge.println("argv[1] = \(argv[1])")
        }
    }
}
```

## Arrays

```checkbe
import Bridge

body Application {
    func main() {
        let int[] values = [1, 2, 3]
        values[1] = 99
        Bridge.println(values[1])
    }
}
```

## Control Flow with For

```checkbe
import Bridge

body Application {
    func main() {
        for (let int i = 0; i < 3; i = i + 1) {
            Bridge.println(i)
        }
    }
}
```

## Capability / Right Example

```checkbe
import Bridge

body Application {
    let capability roCap = new Capability(CAP_READ)
    let right roRight = new Right(RIGHT_READ)

    let:roCap int secret = 10

    let:roRight func reader() {
        Bridge.println(secret)
        // secret = 20   // write violation
    }

    func main() {
        reader()
    }
}
```

## Frequent Errors

- Missing `main` function.
- Wrong `main` signature or non-void return type.
- Using undeclared capability/right names.
- Type mismatch in assignment, return, or function arguments.
- Non-`bool` conditions in `if`/`while`/ternary.
- Indexing non-array values or using non-`int` index.
- Calling module functions with unsupported signatures.
- Empty array literals without resolvable element type.

## Practical Tips

- Prefer explicit types in declarations.
- Keep module calls aligned with `module.toml` overloads.
- Use interpolation for readable output: `"value=\(x)"`.
- Start from the examples in `/examples` and evolve incrementally.
