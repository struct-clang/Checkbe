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

## Math Module API

Use:

```checkbe
import Math
```

Core functions:

- `Math.max(int, int) -> int`
- `Math.max(float, float) -> float`
- `Math.min(int, int) -> int`
- `Math.min(float, float) -> float`
- `Math.euler() -> float`
- `Math.pi() -> float`
- `Math.sqrt(int|float) -> float`
- `Math.pow(int|float, int|float) -> float`

Additional standard helpers are also available:

- `Math.abs`, `Math.sign`, `Math.cbrt`
- `Math.exp`, `Math.log`, `Math.log10`, `Math.log2`
- `Math.floor`, `Math.ceil`, `Math.round`, `Math.trunc`
- `Math.sin`, `Math.cos`, `Math.tan`, `Math.asin`, `Math.acos`, `Math.atan`, `Math.atan2`
- `Math.hypot`, `Math.clamp`, `Math.deg2rad`, `Math.rad2deg`
- `Math.is_nan(float) -> bool`, `Math.is_inf(float) -> bool`

## Network Module API

Use:

```checkbe
import Network
```

Core functions:

- `Network.get(string url) -> string`
- `Network.post(string url, string body) -> string`
- `Network.request(string method, string url, string body) -> string`
- `Network.status(string url) -> int`
- `Network.resolve(string host) -> string`
- `Network.hostname() -> string`

Implementation note:

- HTTP calls are executed directly from `Network` via system sockets (`getaddrinfo`/`socket`/`connect`/`send`/`recv`), without external commands.
- Supported URL scheme is `http://` (plain HTTP, no TLS/`https://` yet).

Example:

```checkbe
import Bridge
import Network

body Application {
    func main() {
        let string url = "http://127.0.0.1:18080/hello"
        Bridge.println(Network.status(url))
        Bridge.println(Network.get(url))
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

The linker uses Boehm GC (`-lgc`) and system math library (`-lm`).
