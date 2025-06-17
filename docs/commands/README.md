# Commands

To execute a command, press space and type the command. There is fuzzy match support, so it is enough to type parts of the command name and it will display options that matches.

It is also possible to create a command file, extension `.sucl`, and run that. Running a command file can be done from within Surfer using the menu option in the File menu, through the toolbar button, or by typing the command ``run_command_file``. It can also be done using the ``--command-file`` argument when starting Surfer.

Not all commands are available unless a file is loaded. Also, some commands are not available in the WASM-build (browser/VS Code extension).

## Waveform/transaction loading and reloading

* ``load_file <FILE_NAME>``

    Load a file. Note that it works to load a waveform file from a command file.

    <div class="warning">In WASM-builds (web browser/VS Code plugin) it is not possible to open a file due to file access restrictions. Use <tt>load_url</tt>.</div>


* ``switch_file <FILE_NAME>``

    Load file, but keep waveform view.

* ``load_url <URL>``

    Load a URL.

* ``reload``

    Reload the current file. Does not work in a web browser.

* ``remove_unavailable``

    Remove variables that are not longer present in the reloaded/switched file.

## State files

* ``load_state``
* ``save_state``
* ``save_state_as``

## Command files

* ``run_command_file <FILE_NAME>`` (not on WASM)

    Run the commands in the given file.

    <div class="warning">In WASM-builds (web browser/VS Code plugin) it is not possible to run another command file from a command file due to file access restrictions.</div>

* ``run_command_file_from_url <URL>``

## Add variable/transaction items

* ``scope_add <SCOPE_NAME>``, ``stream_add``

    Add all signals in the specified scope to the waveform display.

* ``scope_add_recursive <SCOPE_NAME>``

    Add all signals in the specified scope and from all sub-scopes to the waveform display.

    <div class="warning">Adding large hierarchies with a large number of signals can freeze surfer for a significant amount of time.</div>

* ``scope_add_as_group <SCOPE_NAME>``

    Add all signals in the specified scope to the waveform display in a newly created group of the same name.

* ``scope_add_as_group_recursive <SCOPE_NAME>``

    Add all signals in the specified scope and all sub-scopes to the waveform display in a newly created groups nested.

    <div class="warning">Adding large hierarchies with a large number of signals can freeze surfer for a significant amount of time.</div>

* ``scope_select``
* ``stream_select``
* ``variable_add <VARIABLE_NAME>``, ``generator_add  <GENERATOR_NAME>``
* ``variable_add_from_scope``
* ``generator_add_from_stream``

## Add other items

* ``divider_add <NAME>``
* ``timeline_add``

## Groups

* ``group_marked``
* ``group_dissolve``
* ``group_fold_recursive``
* ``group_unfold_recursive``
* ``group_fold_all``
* ``group_unfold_all``

## Controlling item appearance

* ``item_focus``
* ``item_set_color <COLOR_NAME>``
* ``item_set_background_color <COLOR_NAME>``
* ``item_set_format <FORMAT_NAME>``
* ``item_unset_color``

  Reset to default color.

* ``item_unset_background_color``

  Reset to default background color.

* ``item_unfocus``

  Remove focus from currently focused item.

* ``item_rename``
* ``theme_select <THEME_NAME>``

## Navigation

* ``zoom_fit``

  Zoom to display the full simulation.

* ``zoom_in``
* ``zoom_out``
* ``scroll_to_start``,  ``goto_start``
* ``scroll_to_end``, ``goto_end``
* ``transition_next``

  Move cursor to next transition of focused item. Scroll if not visible.

* ``transition_previous``

  Move cursor to previous transition of focused item. Scroll if not visible.

* ``transaction_next``
* ``transaction_prev``

## UI control

* ``show_controls``
* ``show_mouse_gestures``

  Show mouse gesture help window.

* ``show_quick_start``
* ``show_logs``

  Show log window.

* ``toggle_menu``

  Toggle visibility of menu. If not visible, there will be a burger menu in the toolbar.

* ``toggle_side_panel``

* ``toggle_fullscreen``

  Toggle fullscreen view.

* ``toggle_tick_lines``
* ``variable_set_name_type``
* ``variable_force_name_type``
* ``preference_set_clock_highlight``
* ``preference_set_hierarchy_style``
* ``preference_set_arrow_key_bindings``
* ``config_reload``

## Cursor and markers

* ``goto_cursor``

  Go to the location of the main cursor. If off screen, scroll to it.

* ``goto_marker <MARKER_NAME> | #<MARKER_NUMBER>``

  Go to the location of the main cursor. If off screen, scroll to it.

* ``cursor_set <TIME>``

  Move cursor to given time.

* ``marker_set  <MARKER_NAME> | #<MARKER_NUMBER>``

  Add/set marker to location of cursor.

* ``marker_remove <MARKER_NAME> | #<MARKER_NUMBER>``

  Remove marker.

* ``show_marker_window``

  Display window with markers and differences between markers

## Interactive simulation

* ``pause_simulation``
* ``unpause_simulation``

## Viewports

* ``viewport_add``
* ``viewport_remove``

## Waveform control protocol (WCP)

* ``wcp_server_stop`` (not WASM)
* ``wcp_server_start`` (not WASM)

## Other

* ``copy_value``
* ``undo``
* ``redo``
* ``exit`` (not WASM)
