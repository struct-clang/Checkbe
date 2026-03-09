# Compiler Usage

## Build Compiler

```bash
cargo build --release
```

Compiler binary:

```bash
./target/release/checkbe
```

## Compile Programs

Single file:

```bash
checkbe source.checkbe -o app
```

Multiple files:

```bash
checkbe main.checkbe util.checkbe math.checkbe -o app
```

Current CLI requires `.checkbe` input extensions.

If `-o` is omitted, output defaults to the first source file stem.

## Install Compiler + Runtime

```bash
./target/release/checkbe --install --prefix /usr/local
```

Default install prefix is `/usr/local`.

## Help

```bash
checkbe --help
```

## Diagnostics

Compiler emits warnings/errors with source location:

- `<file>:<line>:<column>: <message>`

Compilation stops on lexical, parse, semantic, runtime-prep, or linker failures.

## Runtime and Environment

Required tools/libraries:

- `clang`
- `ar`
- Boehm GC (`bdw-gc`)

GC lookup strategy:

1. `CHECKBE_GC_INCLUDE` + `CHECKBE_GC_LIB`
2. `pkg-config --cflags --libs bdw-gc`
3. Homebrew fallback paths (`/opt/homebrew/opt/bdw-gc/...`)

Runtime root can be overridden with:

```bash
export CHECKBE_RUNTIME=/path/to/runtime
```
