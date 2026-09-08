# Theme parameters

This page documents the keys supported in a Surfer theme file.

You only need to include the keys you want to override. Any omitted value falls back to the selected base theme or, for a full theme file, the built-in defaults.

For a complete example, see the [default theme](https://gitlab.com/surfer-project/surfer/-/blob/main/default_theme.toml?ref_type=heads).

## Value formats

- Colors use RGB hex and may optionally start with `#`, for example `"d4d4d4"` or `"#d4d4d4"`.
- Three-digit hex like `"abc"` or `"#abc"` is also accepted and expanded to `"aabbcc"`.
- Widths, lengths, and similar numeric values are non-negative. Negative values are clamped to `0`.
- Opacity-like values are clamped to the range `0..1`.

## Example

```toml
foreground = "d4d4d4"
border_color = "282b2d"
highlight_background = "37485E"
cursor = { color = "b63935", width = 2 }

canvas_colors = { background = "0b151d", alt_background = "1b252d", foreground = "d4d4d4" }
primary_ui_color = { background = "171717", foreground = "d4d4d4" }
selected_elements_colors = { background = "444444", foreground = "d4d4d4" }

[colors]
Green = "6a9955"
Red = "f44747"

[ticks]
density = 1.0
style = { color = "222222", width = 2 }
```

## Top-level parameters

| Key | Type | Description |
| --- | --- | --- |
| `foreground` | color | Default foreground text color used across the UI. |
| `border_color` | color | Border color between UI elements. |
| `alt_text_color` | color | Alternate text color. Often when an explicit background color is used, the color with most contrast is chosen between `foreground` and this. |
| `canvas_colors` | [`ThemeColorTriple`](#themecolortriple) | Colors for the waveform canvas. |
| `primary_ui_color` | [`ThemeColorPair`](#themecolorpair) | Main UI background and foreground colors. |
| `secondary_ui_color` | [`ThemeColorPair`](#themecolorpair) | Secondary UI colors, used for panels, lists, and inputs. |
| `selected_elements_colors` | [`ThemeColorPair`](#themecolorpair) | Colors used for selected items in the UI. |
| `accent_info` | [`ThemeColorPair`](#themecolorpair) | Informational accent colors. |
| `accent_warn` | [`ThemeColorPair`](#themecolorpair) | Warning accent colors. |
| `accent_error` | [`ThemeColorPair`](#themecolorpair) | Error accent colors. |
| `cursor` | [`SurferLineStyle`](#surferlinestyle) | Style used to draw the main cursor and marker vertical lines. |
| `gesture` | [`SurferLineStyle`](#surferlinestyle) | Style used for mouse gesture lines. |
| `measure` | [`SurferLineStyle`](#surferlinestyle) | Style used for measurement lines. |
| `clock_highlight_line` | [`SurferLineStyle`](#surferlinestyle) | Line style used when clock highlighting is in `Line` mode. |
| `clock_highlight_line_colors` | list of colors | Additional per-clock colors used for clock highlighting in `Line` mode. In multi-clock views, the effective repeating cycle is `clock_highlight_line.color` followed by this list, then wrapped by clock index. |
| `clock_highlight_cycle` | color | Fill color used when clock highlighting is in `Cycle` mode. |
| `clock_highlight_cycle_colors` | list of colors | Additional per-clock colors used when clock highlighting is in `Cycle` mode. In multi-clock views, the effective repeating cycle is `clock_highlight_cycle` followed by this list, then wrapped by clock index. |
| `clock_rising_marker` | boolean | Draw arrows on rising clock edges. |
| `variable_default` | color | Default waveform color for regular signals. |
| `variable_highimp` | color | Color used for high-impedance (`Z`) signal segments. |
| `variable_undef` | color | Color used for undefined (`X`) signal segments. |
| `variable_dontcare` | color | Color used for don't-care signal segments. |
| `variable_weak` | color | Color used for weak-signal segments. |
| `variable_parameter` | color | Color used for parameter/constant values. |
| `variable_event` | color | Color used for event variables. |
| `transaction_default` | color | Default color for transaction streams. |
| `relation_arrow` | [`SurferRelationArrow`](#surferrelationarrow) | Style used for transaction relation arrows. |
| `waveform_opacity` | number `0..1` | Background opacity for waveform rows. |
| `wide_opacity` | number `0..1` | Background opacity for wide signals. |
| `colors` | table of named colors | Named colors exposed in UI commands such as `item_set_color`. |
| `highlight_background` | color | Background highlight color used for focused or emphasized rows/elements. |
| `linewidth` | non-negative number | Standard waveform line width. |
| `thick_linewidth` | non-negative number | Thicker waveform line width for emphasized signals. |
| `vector_transition_width` | non-negative number | Maximum width of vector transition markers. |
| `alt_frequency` | integer | Number of rows using the normal canvas background before alternating to `alt_background`. Set to `0` to disable alternating backgrounds. |
| `viewport_separator` | [`SurferLineStyle`](#surferlinestyle) | Style of the separator between viewports. |
| `drag_hint_color` | color | Color of drag hint guides. |
| `drag_hint_width` | non-negative number | Width of drag hint guides. |
| `drag_threshold` | non-negative number | Threshold before drag hints or drag behavior activate. |
| `ticks` | [`SurferTicks`](#surferticks) | Tick line density and style. |
| `scope_icons` | [`ScopeIcons`](#scopeicons) | Optional icon overrides for scope types in the hierarchy. |
| `variable_icons` | [`VariableIcons`](#variableicons) | Optional icon overrides for variable types in the hierarchy. |

`theme_names` exists internally in the theme data model but is populated by Surfer itself and should not be set in a theme file.

## Named colors

The `[colors]` table defines the color names available to commands like `item_set_color` and `item_set_background_color`.

Example:

```toml
[colors]
Green = "6a9955"
Red = "f44747"
Blue = "569cd6"
```

Each key is an arbitrary user-visible color name and each value is a hex RGB color (with or without a leading `#`).

## Structured types

### `ThemeColorPair`

Used by `primary_ui_color`, `secondary_ui_color`, `selected_elements_colors`, `accent_info`, `accent_warn`, and `accent_error`.

```toml
primary_ui_color = { background = "171717", foreground = "d4d4d4" }
```

| Field | Type | Description |
| --- | --- | --- |
| `background` | color | Background color. |
| `foreground` | color | Foreground/text color. |

### `ThemeColorTriple`

Used by `canvas_colors`.

```toml
canvas_colors = { background = "0b151d", alt_background = "1b252d", foreground = "d4d4d4" }
```

| Field | Type | Description |
| --- | --- | --- |
| `background` | color | Primary waveform canvas background. |
| `alt_background` | color | Alternating background used every `alt_frequency` rows. |
| `foreground` | color | Text color used on the canvas. |

### `SurferLineStyle`

Used by `cursor`, `gesture`, `measure`, `clock_highlight_line`, and `viewport_separator`.

```toml
cursor = { color = "b63935", width = 2 }
```

| Field | Type | Description |
| --- | --- | --- |
| `color` | color | Line color. |
| `width` | non-negative number | Line width. |

### `SurferTicks`

Used by `ticks`.

```toml
[ticks]
density = 1.0
style = { color = "222222", width = 2 }
```

| Field | Type | Description |
| --- | --- | --- |
| `density` | number `0..1` | Tick density. `1` means as many ticks as can fit without overlap. |
| `style` | [`SurferLineStyle`](#surferlinestyle) | Tick line style. |

### `SurferRelationArrow`

Used by `relation_arrow`.

```toml
[relation_arrow]
style = { color = "c61521", width = 1.3 }
head_angle = 25
head_length = 8
```

| Field | Type | Description |
| --- | --- | --- |
| `style` | [`SurferLineStyle`](#surferlinestyle) | Arrow line style. |
| `head_angle` | non-negative number | Arrowhead angle in degrees. |
| `head_length` | non-negative number | Arrowhead length. |

## Icon customization

Themes can override both the glyph and the color for scope and variable icons shown in the hierarchy view.

Icon values are Unicode strings, typically copied from the [Remix Icon](https://remixicon.com/) set used by Surfer.

### `ScopeIcons`

Use a `[scope_icons]` table to override glyphs for scope-like hierarchy nodes.

Supported keys are:

- `module`
- `task`
- `function`
- `begin`
- `fork`
- `generate`
- `struct`
- `union`
- `class`
- `interface`
- `package`
- `program`
- `vhdl_architecture`
- `vhdl_procedure`
- `vhdl_function`
- `vhdl_record`
- `vhdl_process`
- `vhdl_block`
- `vhdl_for_generate`
- `vhdl_if_generate`
- `vhdl_generate`
- `vhdl_package`
- `ghw_generic`
- `vhdl_array`
- `unknown`
- `clocking`
- `sv_array`

Example:

```toml
[scope_icons]
module = "\ued52"
function = "\ued9e"
package = "\ued88"
```

#### `scope_icons.colors`

The nested `[scope_icons.colors]` table accepts the same keys as `[scope_icons]`, but each value is a color instead of a glyph.

```toml
[scope_icons.colors]
module = "4FC3F7"
function = "BA68C8"
package = "FFD54F"
```

### `VariableIcons`

Use a `[variable_icons]` table to override glyphs for variable categories.

Supported keys are:

- `wire`
- `bus`
- `string`
- `event`
- `other`

Example:

```toml
[variable_icons]
wire = "\uf035"
bus = "\uebad"
string = "\uf201"
event = "\ueea8"
other = "\uedfc"
```

#### `variable_icons.colors`

The nested `[variable_icons.colors]` table accepts the same keys as `[variable_icons]`, but each value is a color.

```toml
[variable_icons.colors]
wire = "81C784"
bus = "64B5F6"
string = "FFB74D"
event = "F06292"
other = "BA68C8"
```

## Notes

- A theme file may define only a subset of keys when it is intended to override an existing theme.
- The `colors` table is separate from UI palette settings; it exists so commands can refer to stable names like `Green` or `Orange`.
- Icon color tables are optional. If omitted, Surfer uses built-in defaults for icon colors.

## Widget palette (`[ui]`)

The optional `[ui]` table separates application widgets from waveform colors.
All existing themes continue to load without it. `dark_mode` selects egui's light
or dark base (otherwise inferred from the secondary background); `muted`,
`accent`, `subtle`, and `hover` are hex colors. Omitted colors derive from the
existing foreground, accent, canvas, and background tokens. Inactive widgets
use the primary surface; hovered widgets use `hover` with an accent border;
pressed and open widgets use the selected-elements background. Widget corners
are six points. Text fields use the primary surface; links and focus use the
accent. See the [interactive appearance chapter](../../html/appearance.html).
