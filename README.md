# forge-plugin-spell-check


## Install with use_package

Add this to `~/.config/forge/init.rhai`:

```rhai
use_package("spell-check", #{
  vc: "https://github.com/martial-plains/spell-check.forge",
  pin: "v0.1.0",
  commands: ["spell-check-mode", "spell-check-buffer", "spell-correct-word"],
  grant: #{ directories: ["/usr/share/dict"] },
});
```

Change the granted directory if your word list lives elsewhere. Forge asks
before running the package build command the first time.

## Building

This plugin is built against the Forge plugin SDK (`forge-plugin-sdk`),
taken from the Forge repository named in `Cargo.toml` (change the `git =`
URL if Forge lives elsewhere).

    rustup target add wasm32-wasip2
    cargo build --release --target wasm32-wasip2

Copy the resulting `.wasm` to `~/.config/forge/plugins/`.

## License

MIT OR Apache-2.0. See the license files in this directory.
