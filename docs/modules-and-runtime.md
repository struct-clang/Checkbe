# Modules and Runtime

## Importing Modules

Use:

```checkbe
import Bridge
```

External modules are discovered from runtime roots in `runtime/modules/<ModuleName>/module.toml`.

## Bridge Module API

The compiler has special semantic handling for common Bridge calls and also validates against `module.toml` overloads.

Common functions:

- `Bridge.println(...)`
- `Bridge.print(...)`
- `Bridge.readln() -> string`
- `Bridge.sleep(int)`
- `Bridge.usleep(int)`
- `Bridge.system(string, ...)`

Example:

```checkbe
import Bridge

body Application {
    func main() {
        let string line = Bridge.readln()
        Bridge.println("You typed: \(line)")
        Bridge.system("echo", "done")
    }
}
```

## Capability and Right Model

Top-level declarations:

```checkbe
let capability roCap = new Capability(CAP_READ)
let right roRgt = new Right(RIGHT_READ)
```

Attach them with `let:<name>`:

```checkbe
let:roCap int guarded = 10
let:roRgt func onlyRead() {
    Bridge.println(guarded)
}
```

Semantics:

- `capability` constrains access to annotated variables.
- `right` constrains operations inside annotated function.
- Operations are checked as `read` and `write`.
- `main` ignores attached rights and is treated as full-rights entrypoint.

Permission atoms are interpreted by name heuristics:

- Contains `read` -> read allow/deny
- Contains `write` -> write allow/deny
- Contains `all`/`full` -> all allow/deny
- Contains `forbid` / `deny` / `no_` / leading `!` -> deny semantics

Unknown atoms produce warnings.

## Runtime Discovery

Runtime root is searched in this order (first valid wins for core runtime):

1. `CHECKBE_RUNTIME`
2. `<source_dir>/runtime`
3. `<current_workdir>/runtime`
4. `~/.local/share/checkbe/runtime`
5. `~/.checkbe/runtime`
6. `/usr/local/lib/checkbe/runtime`
7. `/opt/homebrew/lib/checkbe/runtime`
8. Near compiler executable (`.../runtime`)

The linker expects Boehm GC (`-lgc`).
