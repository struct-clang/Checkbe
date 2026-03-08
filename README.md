# Checkbe Compiler

Компилятор языка `Checkbe` на Rust с LLVM (`inkwell`) и Boehm GC.
Сделано через ИИ. НЕ используйте в продакшене.

## Возможности

- Лексер, парсер, AST.
- Статический semantic analyzer:
  - проверка типов;
  - статическая проверка `capability` и `right` для операций `read/write`;
  - диагностика ошибок и предупреждений.
- Генерация LLVM IR и объектного файла через `inkwell`.
- Линковка в нативный бинарник через `clang`.
- Интеграция с Boehm GC (`checkbe_runtime_init`, `cb_gc_strdup`).
- Модульная система через `import` + `runtime/modules/<Module>/module.toml`.
- Массивы: `int[]`, `string[]`, литералы `[1, 2, 3]`, индексация `arr[i]`, присваивание `arr[i] = v`.
- Конвертация в `int`: `Int(value)` для `string/float/bool/int`.
- `Bridge.readln()` для чтения строки из stdin.
- `Bridge.print(...)` и `Bridge.println(...)` принимают 0..N аргументов (`int`, `float`, `string`, `bool`).
- Интерполяция строк в любом строковом литерале: `"Hello, \\(name)"`.

## Зависимости

- Rust stable
- LLVM 18 (`llvm-config` и библиотеки)
- `clang`
- Boehm GC (`bdw-gc`)

Пример для macOS (Homebrew):

```bash
brew install llvm@18 bdw-gc
```

Перед сборкой (для `inkwell`/`llvm-sys`):

```bash
export LLVM_SYS_181_PREFIX=/opt/homebrew/opt/llvm@18
```

## Сборка

```bash
cargo build --release
```

Бинарник:

```bash
./target/release/checkbe
```

Установка в системный префикс (бинарь + runtime/Bridge):

```bash
./target/release/checkbe --install --prefix /usr/local
```

После установки можно запускать:

```bash
checkbe input.checkbe -o output
```

## Использование

```bash
checkbe source.checkbe -o outputbinaryname
```

Пример:

```bash
./target/release/checkbe examples/hello.checkbe -o hello
./hello
```

## Runtime layout

```text
runtime/
  core/runtime.c
  modules/
    Bridge/
      module.toml
      bridge.c
```

- `Bridge` подключается через `import Bridge`.
- Функция `Bridge.println(...)` поддерживает типы `int`, `float`, `string`, `bool`.