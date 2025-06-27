/*!
    # Writing a Surfer Translator Plugin

    Surfer translators are web-asssembly binaries that are loaded at runtime by Surfer.
    They can be written in any language that has an `extism` plugin
    development kit
    [https://extism.org/docs/concepts/pdk/](https://extism.org/docs/concepts/pdk/).

    For this example we will use Rust since that is what the rest of Surfer is written in, which allows us to reuse type definitions between Surfer itself and the plugin.

    To create a plugin, create a new project
    ```bash
    cargo init --lib cool_surfer_translator
    ```
    then modify the `Cargo.toml` to set the library type to "cdylib", and add the `extism_pdk` and `surfer-translation-types` library as dependencies
    ```toml
    [lib]
    crate-type = ["cdylib"]

    [dependencies]
    extism-pdk = "1.4.1"
    surfer-translation-types.git = "https://gitlab.com/surfer-project/surfer.git"
    ```

    In your new project, you now need to define a few functions which must all
    be annotated with `#[plugin_fn]` and have the right type signature. Click on each function to learn more

    - [name]: sets the name of the plugin in the format selection list
    - [translates]: allows the plugin to opt in or out of translating certain signals
    - [variable_info]: specifies the hierarchical structure of the signal
    - [translate]: does the actual translation of bit vectors to new values

    In addition, there are a few [optional] functions that can be implemented for additional
    functionality
    - [new]: Called once on plugin load
    - [reload]: Called when Surfer reloads the waveform
    - [set_wave_source]: Called when the current waveform changes
    - [variable_name_info]: Translate signal names

    ## Accessing Files

    Surfer plugins are sandboxed and are in general not allowed _any_ access to the external
    world. Translators may need to read the file system however, and for that, "host functions"
    are provided. To use them, define them in your plugin using

    ```rust
    use extism_pdk::host_fn;

    #[host_fn]
    extern "ExtismHost" {
        pub fn read_file(filename: String) -> Vec<u8>;
        pub fn file_exists(filename: String) -> bool;
    }
    ```

    ## Maintaining State

    Plugins may need to maintain state between calls. This can be done by
    simply using static variables in the plugin.
    ```
    static STATE: Mutex<bool> = Mutex::new(false)
    ```

    > NOTE: The static variables are shared between all "instances" of the
    > translator, i.e. if you want to maintain different state for different
    > variables, this must currently be handled on the plugin side.

    ## Testing and Installation

    To build your plugin, call
    ```bash
    cargo build --debug --target wasm32-unknown-unknown
    ```
    which will create `target/debug/cool_surfer_translator.wasm`

    This file can then be copied to the local or global plugin translator directories in order to be found and automatically loaded by Surfer

    Local:
    ```
    .surfer/translators/
    ```

    Global
    | Os      | Path                                                                  |
    |---------|-----------------------------------------------------------------------|
    | Linux   | `~/.config/surfer/translators`.                                       |
    | Windows | `C:\Users\<Name>\AppData\Roaming\surfer-project\surfer\config\translators`.  |
    | macOS   | `/Users/<Name>/Library/Application Support/org.surfer-project.surfer/translators` |
*/

use extism_pdk::FnResult;
use surfer_translation_types::plugin_types::TranslateParams;
use surfer_translation_types::{
    TranslationPreference, TranslationResult, ValueKind, VariableInfo, VariableMeta,
};

/// Returns the name of the plugin as shown to the user. This needs to be unique, so
/// do not set it to a translator name that is already present in Surfer.
///
/// While it is possible to change the name between calls, doing so will cause
/// unexpected behaviour.
pub fn name() -> FnResult<&'static str> {
    Ok("Docs Plugin")
}

/// Returns a translation preference for the specified variable, which allows
/// the translator to opt out of translating certain signals which it does not
/// support.
///
/// For example, a translator which translates 32 bit floating point values should return
/// [TranslationPreference::Yes] for bit variables with `num_bits == 32` and
/// [TranslationPreference::No] for other signals.
///
/// Translators also have the option of returning [TranslationPreference::Prefer] to
/// not only allow their use on a signal, but make it the _default_ translator for that signal.
/// This should be used with caution and only in cases where the translator is _sure_ that the
/// translator is a sane default. A prototypical example is translators for custom HDLs where
/// it is known that the signal came from the custom HDL.
pub fn translates(_variable: VariableMeta<(), ()>) -> FnResult<TranslationPreference> {
    Ok(TranslationPreference::Yes)
}

/// Returns information about the hierarchical structure of the signal. For translators
/// which simply want to do bit vector to string and/or color translation, returning
/// [VariableInfo::Bits] is sufficient.
///
/// For compound signals, [VariableInfo::Compound] is used, which allows the user to
/// expand the signal into its available subfields. If subfields specified here
/// are omitted by the [translate] function, they will be left empty during the corresponding
/// clock cycles.
pub fn variable_info(variable: VariableMeta<(), ()>) -> FnResult<VariableInfo> {
    Ok(VariableInfo::Compound {
        subfields: (0..(variable.num_bits.unwrap_or_default() / 4 + 1))
            .map(|i| (format!("[{i}]"), VariableInfo::Bits))
            .collect(),
    })
}

/// Gets called once for every value of every signal being rendered, and
/// returns the corresponding translated value.
///
/// For non-hierarchical values, returning
/// ```notest
/// Ok(TranslationResult {
///     val: surfer_translation_types::ValueRepr::String(/* value here */),
///     kind: ValueKind::Normal,
///     subfields: vec![],
/// })
/// ```
/// works, for hierarchical values, see [TranslationResult]
///
/// It is often helpful to destructure the params like this to not have to perform field
/// access on the values
/// ```notest
/// pub fn translate(
///     TranslateParams { variable, value }: TranslateParams,
/// ) -> FnResult<TranslationResult> {}
/// ```
///
pub fn translate(
    TranslateParams {
        variable: _,
        value: _,
    }: TranslateParams,
) -> FnResult<TranslationResult> {
    Ok(TranslationResult {
        val: surfer_translation_types::ValueRepr::Tuple,
        kind: ValueKind::Normal,
        subfields: vec![],
    })
}

/// Documentation for functions which are not necessary for a basic translator but can do more
/// advanced things.
pub mod optional {
    use extism_pdk::Json;
    use surfer_translation_types::translator::{TrueName, VariableNameInfo};

    use super::*;

    /// The new function is used to initialize a plugin. It is called once when the
    /// plugin is loaded
    pub fn new() -> FnResult<()> {
        Ok(())
    }

    /// Called every time Surfer reloads the waveform. This can be used to
    /// re-run any initialization that depends on which waveform is loaded.
    ///
    /// Note that `set_wave_source` is also called when reloading, so if the state
    /// depends on the currently loaded waveform, `reload` is not necessary.
    pub fn reload() -> FnResult<()> {
        Ok(())
    }

    /// This is called whenever the wave source changes and can be used by the plugin to change
    /// its behaviour depending on the currently loaded waveform.
    pub fn set_wave_source(
        Json(_wave_source): Json<Option<surfer_translation_types::WaveSource>>,
    ) -> FnResult<()> {
        Ok(())
    }

    /// Can be used to convert a variable name into a name that is more descriptive.
    /// See [VariableNameInfo] and [TrueName] for details on the possible output.
    ///
    /// **NOTE** The user has no way to opt out of a translator that specifies a true name,
    /// which means this feature should be used with caution and only on signals which
    /// are likely to mean very little to the user in their original form. The original use
    /// case for the feature is for translators for HDLs to translate temporary variables
    /// into something more descriptive.
    pub fn variable_name_info(
        Json(_variable): Json<VariableMeta<(), ()>>,
    ) -> FnResult<Option<VariableNameInfo>> {
        let _ = TrueName::SourceCode {
            line_number: 0,
            before: String::new(),
            this: String::new(),
            after: String::new(),
        };
        Ok(None)
    }
}

#[doc(hidden)]
pub use optional::*;
