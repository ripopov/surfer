# Compile Features

There are a number of compile time features that can be enabled or disabled.

| Feature | Description | Default |
| ------- | ----------- | ------- |
| `accesskit` |  Accessibility support. | No`*`|
| `f128` | 128-bit floating-point translator. Requires building with gcc as underlying C-compiler. | No |
| `performance_plot` | The `show_performance` command and the drawing performance plot window. | Yes |
| `python` | Python translators. | No |
| `wasm_plugins` | WASM translator plugins. | Yes |

`*` Included in pre-built binaries.
