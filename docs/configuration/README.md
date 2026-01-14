# Configuration

Surfer can be customized by modifying configuration files.

Note that it is enough to only add the configuration parameters that are changed to the file. All other will have the default values.

For a list of all possible configuration options, please look at the [default configuration](https://gitlab.com/surfer-project/surfer/-/blob/main/default_config.toml?ref_type=heads).
To replace Surfer's default configuration, add your configuration to a file called `config.toml` and place it in Surfer configuration directory. The location of the configuration directory depends on your OS.

| Os      | Path                                                                  |
|---------|-----------------------------------------------------------------------|
| Linux   | `~/.config/surfer/config.toml`.                                       |
| Windows | `C:\Users\<Name>\AppData\Roaming\surfer-project\surfer\config\config.toml`.  |
| macOS   | `/Users/<Name>/Library/Application Support/org.surfer-project.surfer/config.toml` |

Surfer also allows having custom configs per directory.
To use a configuration in just a single directory, create a `.surfer` subdirectory and add a file called `config.toml` inside this subdirectory.
If you now start Surfer from within the directory containing `.surfer`, the configuration is loaded.

The load order of these configurations is `default->config.toml->project specific`.
All these configuration options can be layered, this means that configurations that are loaded later only overwrite the options they provide.

After changing the configuration, run the `config_reload` command to update the running Sufer instance.

## Themes

To add additional themes to Surfer, create a `themes` directory in Surfer's config directory and add your themes inside there. That is

| Os      | Path                                                                  |
|---------|-----------------------------------------------------------------------|
| Linux   | `~/.config/surfer/themes/`                                     |
| Windows | `C:\Users\<Name>\AppData\Roaming\surfer-project\surfer\config\themes\`  |
| macOS   | `/Users/<Name>/Library/Application Support/org.surfer-project.surfer/themes/` |

You can also add project-specific themes to `.surfer/themes` directories.
Additionally, configurations can be loaded using the Menubar option `View/Theme` or using the `theme_select` command.

For a list of all possible style options, please look at the [default theme](https://gitlab.com/surfer-project/surfer/-/blob/main/default_theme.toml?ref_type=heads).
For example of existing themes [look here](https://gitlab.com/surfer-project/surfer/-/tree/main/themes?ref_type=heads).

### Customizing Icons

Themes can customize the icons displayed in the hierarchy view for scopes and variables. Icons are specified using Unicode code points from the [Remix Icon](https://remixicon.com/) font.
To find the Unicode code point for an icon, search the [egui-remixicon source](https://github.com/get200/egui-remixicon).


#### Scope Icons

Scope icons appear next to hierarchy items like modules, functions, and packages. Add a `[scope_icons]` section to your theme file:

```toml
[scope_icons]
module = "\ued52"        # FOLDER_2_LINE
function = "\ued9e"      # FUNCTION_LINE
package = "\ued88"       # FOLDER_ZIP_LINE
#...
```

This is useful for adapting icons to HDL languages other than Verilog/VHDL. For example, if your simulator maps a language-specific construct to one of the existing scope types,
you can customize its icon to better represent its meaning in your workflow.

#### Variable Icons

Variable icons appear next to signals in the hierarchy. Add a `[variable_icons]` section to your theme file:

```toml
[variable_icons]
wire = "\uf035"          # PULSE_LINE
bus = "\uebad"           # CODE_S_SLASH_LINE
string = "\uf201"        # TEXT
event = "\ueea8"         # LIGHTBULB_FLASH_LINE
other = "\uedfc"         # HASHTAG
```

#### Icon Colors

Icons can be colored using 6-digit hex RGB values (without `#`). Add `[scope_icons.colors]` or `[variable_icons.colors]` sections:

```toml
[scope_icons.colors]
module = "4FC3F7"        # Light Blue
function = "BA68C8"      # Purple
package = "FFD54F"       # Yellow
# ... other scope types: task, begin, fork, generate, struct, union,
# class, interface, program, vhdl_*, ghw_generic, unknown

[variable_icons.colors]
wire = "81C784"          # Green (1-bit)
bus = "64B5F6"           # Blue (multi-bit)
string = "FFB74D"        # Orange
event = "F06292"         # Pink
other = "BA68C8"         # Purple
```

Light themes include darker colors for better contrast. Only specify colors you want to override.
