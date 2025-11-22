# Mnenomic Translators

A simpler variant of decoders is the mnemonic translator. This can convert bit-vector values into text using a simple configuration file.

The configuration files are located in the following directories depending on platform:

| Os      | Path                                                                  |
|---------|-----------------------------------------------------------------------|
| Linux   | `~/.config/surfer/mnemonic/`                                        |
| Windows | `C:\Users\<Name>\AppData\Roaming\surfer-project\surfer\config\mnemonic\`  |
| macOS   | `/Users/<Name>/Library/Application Support/org.surfer-project.surfer/mnemonic/` |

The file format is in its simplest form just pairs of vector values and text strings, one per line. For example,

``` text
0000 Start
0001 State1
0010 State2
```

will map the variable value `0000` into the text `Start` and the variable value `0001` into the text `State2` and so on.

The values can be written either as binary, decimal or hex and different formats can be used for each row. It is possible to use `_` as part of the numbers to obtain clearer formatting.

Hence, the example above can be written as

``` text
00_00 Start
0x1 State1
2 State2
```

It is also possible to write values using 4-state or 9-state logic.

The wordlength is determined by the values, but can also be specified using a line as the first or second line (see below about naming) as

``` text
Bits: 4
0 Start
1 State1
2 State2
```

It is also possible to name the translator, which then is used as the Format to select, by having a first line as

``` text
Name: My statemachine
Bits: 4
0 Start
1 State1
2 State2
```

If no name is provided, the filename without extension is used as Format name.

It is also possible to have spaces in the strings by enclosing them in "

``` text
Name: My statemachine
Bits: 4
0 Start
1 "State 1"
2 "State 2"
```

Finally, it is possible to supply a variable kind or color as a third argument for each line. The kinds are:

* `default`, `normal`: Use the `variable_default` theme color
* `undef`: Use the `variable_undef` theme color
* `highimp`: Use the `variable_highimp` theme color
* `dontcare`: Use the `variable_dontcare` theme color
* `weak`: Use the `variable_weak` theme color

For colors, any named [Color32](https://docs.rs/ecolor/latest/ecolor/struct.Color32.html) can be used (case insensitive, so `RED`, `red`, and `Red` will all work). It is also possible to specify an EGD hex color, either with or without a leading `#` so both `aa7035`  and `#aa7035` are valid options.

If no kind/color is given, the waveform is drawn as a default/normal variable.

Comments starts with `//` and spans the rest of the line.

A final example is:

``` text
// The statemachine in block 2
Name: Block 2 statemachine
Bits: 4
// Make the start state pink
0 Start pink
// Often better to use the kinds as they follow the themes
// We also want to highlight State 1
1 "State 1" undef
// Empty lines are OK

2 "State 2" // Just keep normal

```
