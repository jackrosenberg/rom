# ROM

Visual build monitor for Nix that transforms cryptic build logs into a clean,
real-time dependency graph. Think of it as `NOM`, but written in Rust with a
focus on speed and showing you exactly what Nix is doing with
your builds.

Built with a modular parser under [`crates/cognos`](crates/cognos) that handles
the ATerm and internal-json log formats from Nix.

> [!NOTE]
> ROM is still under active development. Things may break, output formats may
> change, and bugs are to be expected. If you encounter any issues, please
> report them!

## Usage

ROM is primarily designed to wrap the Nix installation on your system. As such,
the _recommended_ interface is using `rom build`, `rom shell` and `rom develop`
for their Nix counterparts.

<!--markdownlint-disable MD013-->

```terminal
$ rom -h
ROM - A Nix build output monitor

Usage: rom [OPTIONS] [COMMAND]

Commands:
  build    Run nix build with monitoring
  shell    Run nix shell with monitoring
  develop  Run nix develop with monitoring
  help     Print this message or the help of the given subcommand(s)

Options:
      --json                     Parse unprefixed Nix internal-json records from stdin
      --silent                   Minimal output
      --style <STYLE>            Presentation style: connected, compact, verbose, plain, dashboard, table-summary, or full-summary [default: connected]
      --log-prefix <LOG_PREFIX>  Log prefix style: short, full, none [default: short]
      --platform <PLATFORM>      Nix-family evaluator to use. Auto-detected by default
  -v...                          Increase verbosity; controls nix log level and rom diagnostic output. Repeatable: -v (info), -vv (debug), -vvv (trace)
  -h, --help                     Print help
  -V, --version                  Print version
```

<!--markdownlint-enable MD013-->

To build a package with Nix, let's say `pkgs.hello`, you can do:

```terminal
$ rom build nixpkgs#hello
⢄ Building hello-2.12.2  configurePhase  2s
  Building 1 │ Waiting 4 │ 2s
```

and the live operations console will appear below the build logs. Each active
package appears as a node, with status, phase, and timing information. The final
graph is retained after the command exits.

ROM can also monitor an existing stream. With no subcommand it reads standard
input, preserves log output once, and appends the final operations graph:

```sh
nix build nixpkgs#hello 2>&1 | rom
nix build nixpkgs#hello --log-format internal-json 2>&1 | rom --json
```

`@nix `-prefixed internal-json records are detected automatically; `--json`
also accepts unprefixed records. Stream output is append-only, making this mode
safe for redirection and library writers.

### Presentation styles

Use `--style <STYLE>` with any ROM invocation, including stdin monitoring and
Nix-wrapper subcommands. Seven presets are available:

- `connected`: default connected activity tree with the standard build/cache
  footer. The alias `tree` is also accepted.
- `compact`: connected activity tree with a single-line summary footer for
  narrow terminals or dense logs.
- `verbose`: connected activity tree with expanded status, cache, and outcome
  details.
- `plain`: minimal flat text view without graph chrome.
- `dashboard`: dashboard-oriented live status view.
- `table-summary`: connected live/final graph plus a tabular final summary.
  The alias `table` is also accepted.
- `full-summary`: connected live/final graph plus the most complete final
  summary. The alias `full` is also accepted.

For example:

```sh
rom --style compact build nixpkgs#hello
nix build nixpkgs#hello --log-format internal-json 2>&1 | rom --json --style full-summary
```

### Library API

The top-level `rom` crate re-exports the stream API from `rom-core`:
`Config`, `InputMode`, `Monitor<W>`, `create_monitor`, and `monitor_stream`.
Generic writers do not take terminal ownership; output width is selected with
`Config::width`, and the historical `Config::piping = true` setting disables
ANSI color for redirected output.

Presentation selection is available through additive APIs:
`PresentationStyle`, `RenderOptions`, `MonitorOptions`,
`create_monitor_with_options`, and `monitor_stream_with_options`. Existing
`Config` struct literals and callers of `create_monitor` and `monitor_stream`
remain source-compatible; they use `PresentationStyle::Connected` by default.
New callers can opt into any preset:

```rust
use rom::{Config, MonitorOptions, PresentationStyle, monitor_stream_with_options};

let config = Config::default();
let options = MonitorOptions::from(PresentationStyle::Compact);
let input = std::io::Cursor::new(Vec::<u8>::new());
let mut output = Vec::new();

monitor_stream_with_options(config, options, input, &mut output)?;
# Ok::<(), rom::RomError>(())
```

### Migrating to 0.3

The `rom`, `rom-core`, and `rom-cli` crates are now version 0.3.0, and the
independently versioned `cognos` parser is 2.0.0. The stream monitor no longer
accepts legacy or arbitrary `.drv` path forms: embedding callers must provide
canonical `/nix/store/<32-character-nix-base32-hash>-<name>.drv` paths. Public
`Monitor::process_action` now emits message and build-log actions through the
same exact-once writer path as JSON input.

The old `--format`, `--legend`, `--summary`, and `--log-lines` flags were
removed. Use the preset mapping `tree` → `connected`, `plain` → `plain`,
`dashboard` → `dashboard`, compact/table/verbose legends → the matching
`compact`/`connected`/`verbose` presets, and table/full summaries →
`table-summary`/`full-summary`. Logs now stream above the bounded graph, so
there is no retained log-line cap.

The former `cache`, `display`, and `icons` modules and their renderer APIs were
removed. The legacy `DisplayFormat`, `LegendStyle`, and `SummaryStyle` names
remain as `Config` field types so old struct literals compile, but new rendering
code should use `PresentationStyle`/`RenderOptions`. New presentation choices
belong in `MonitorOptions`.
The old parsing helpers `Monitor::extract_path_from_message` and
`Monitor::extract_byte_size` were implementation details exposed accidentally;
embedding code should parse its own unstructured messages or submit decoded
Cognos actions through `Monitor::process_action`.

### Argument Passthrough

At times, especially while you're calling ROM as a standalone executable, you
might need to pass additional flags to the Nix command being invoked. ROM allows
for this behaviour by accepting `--` as a delimiter and passing any arguments
that come after to Nix. For example:

```terminal
$ rom develop nixpkgs#hello -- --substituters ""
fetching git input 'git+file:///home/notashelf/Dev/notashelf/rom'
┗━ ⏵ 0 │ ✔ 2 │ ✗ 0 │ ⏸ 0 │ ⏱ 1s


notashelf@enyo ~/Dev/notashelf/rom [git:(9e83f57...) *]
i $ hello
Hello, world!
```

## FAQ

**Q**: If "NOM" is nix-output-monitor, what does "ROM stand for"?

**A**: It doesn't stand for anything, I named it _rom_ because it sounds like
_rum_. I like rum. However you may choose to name it "rusty output monitor" or
"raf's output monitor" at your convenience. I don't know, be creative.

## Attributions

This project is clearly inspired by the famous
<https://github.com/maralorn/nix-output-monitor>. I am a huge fan of NOM's
design, and built ROM as a fast Rust implementation centered on a compact operations
console.

The ATerm and internal-json log parser was inspired, and mostly copied from
<https://git.atagen.co/atagen/nous> with consolidation, cleaner repo layout, and
a better separation of concerns. rom builds on the ideas previously pondered by
nous, and provides a subcrate under [`crates/cognos`](crates/cognos) for easy
parsing. Thank you Atagen for letting me play with the idea.

## License

<!--markdownlint-disable MD059-->

This project is made available under Mozilla Public License (MPL) version 2.0.
See [LICENSE](LICENSE) for more details on the exact conditions. An online copy
is provided [here](https://www.mozilla.org/en-US/MPL/2.0/).

<!--markdownlint-enable MD059-->
