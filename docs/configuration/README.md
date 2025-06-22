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
