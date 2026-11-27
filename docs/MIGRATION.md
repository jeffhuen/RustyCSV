# Migrate and upgrade RustyCSV

Use this guide to upgrade between RustyCSV versions or to move a codebase from NimbleCSV to RustyCSV.

## Upgrade from 0.4.x to 0.5.0

Version 0.5.0 keeps all public Elixir function signatures, return shapes, default options, and strategy atoms compatible with 0.4.x. Review these behavior changes before you deploy.

### 1. Update the dependency

Update the version requirement in `mix.exs`:

```elixir
def deps do
  [
    {:rusty_csv, "~> 0.5.0"}
  ]
end
```

Fetch and compile the update:

```bash
mix deps.update rusty_csv
```

If you run Erlang/OTP 29, `rustler_precompiled` downloads the precompiled NIF 2.18 artifact automatically. You do not need to change your configuration.

If you compile from source with `FORCE_RUSTYCSV_BUILD=1`, use Rust 1.91 or newer because `rustler 0.39.0` requires Rust 1.91. The `nightly-2026-07-27` toolchain pinned in `native/rustycsv/rust-toolchain.toml` meets this requirement.

### 2. Update code that parses `RustyCSV.ParseError.message`

Version 0.5.0 changes the `RustyCSV.ParseError` message text to include 1-based line and column numbers:

- Before: `"unexpected escape character at byte 13"`
- After: `"unexpected escape character at line 1, column 14 (byte 13)"`

If your code matches the full message string or parses `"at byte N"` with a regular expression, read the structured exception fields:

```elixir
try do
  CSV.parse_string(input)
rescue
  e in RustyCSV.ParseError ->
    Logger.warning("CSV error",
      reason: e.reason,
      line: e.line,
      column: e.column,
      byte_offset: e.byte_offset
    )
end
```

The `:reason` field is one of `:unexpected_quote`, `:trailing_garbage`, `:unterminated_quote`, `:invalid_encoding`, `:incomplete_encoding`, or `:truncated_encoding`. For encoding errors, `:line` and `:column` are `nil`.

### 3. Fix invalid option values

Version 0.5.0 raises `ArgumentError` for invalid options that 0.4.x accepted silently or rejected with `FunctionClauseError`:

- `parse_stream/2` with `headers: []` now raises `ArgumentError`, as `parse_string/2` already does. Pass `headers: false` for list rows, `headers: true` for header-row keys, or a non-empty list of keys.
- `parse_string/2` and `parse_stream/2` with a non-boolean `:skip_headers` value now raise `ArgumentError`. Pass `true` or `false`.
- `parse_string/2` with an unknown `:strategy` atom now raises `ArgumentError` instead of `FunctionClauseError`.
- `parse_stream/2` and `RustyCSV.Streaming` functions with a non-positive `:chunk_size` or `:max_buffer_size` now raise `ArgumentError` when you build the stream. Pass a positive integer.
- `RustyCSV.define/2` now validates `:strategy`, `:newlines`, `:line_separator`, `:trim_bom`, `:dump_bom`, and `:escape_formula` at compile time. Pass a valid strategy atom, a non-empty list of non-empty binaries for `:newlines`, and a binary for `:line_separator`. Pass booleans for `:trim_bom` and `:dump_bom`, and pass a map or `nil` for `:escape_formula`.

### 4. Update direct `RustyCSV.Native` callers

`RustyCSV.Native` is an internal module and is not part of the public SemVer contract. If your code calls `RustyCSV.Native` directly and matches malformed-CSV error tuples, update the 3-element inner tuple to 5 elements:

```elixir
# Before (0.4.x)
{:error, {:malformed_csv, reason, byte_offset}}

# After (0.5.0)
{:error, {:malformed_csv, reason, byte_offset, line, column}}
```

## Upgrade from 0.3.x to 0.5.0

If you upgrade from 0.3.x, complete these steps from the 0.4.x releases in addition to the 0.4.x to 0.5.0 steps above:

1. **Require Elixir 1.18 or newer.** RustyCSV 0.4.1 raised the minimum Elixir version to `~> 1.18`.
2. **Opt out of strict parsing only when you accept malformed quotes.** Since 0.4.0, malformed quotes raise `RustyCSV.ParseError` by default. To keep pre-0.4 lenient behavior for dirty input, pass `strict: false` to `parse_string/2` or `parse_stream/2`.
3. **Flatten dumped iodata when you need a binary.** Since 0.4.0, `dump_to_iodata/2` returns one iodata list per row and `dump_to_stream/1` emits list-shaped row iodata. If a caller expects a binary, pass the result to `IO.iodata_to_binary/1`.
4. **Remove `:batch_size` from streaming calls.** RustyCSV 0.4.1 removed `:batch_size`. Streaming now drains every completed native row after each chunk.
5. **Audit bundled allocators before you re-enable mimalloc.** Since 0.4.5, precompiled artifacts use the system allocator to prevent VM crashes when two NIFs link mimalloc. If you need mimalloc for parallel encoding or heavy streaming, follow the audit in [Allocator safety](ALLOCATOR_SAFETY.md) before you set `RUSTYCSV_ALLOCATOR=mimalloc`.

## Migrate from NimbleCSV

Follow these steps to switch an existing codebase from NimbleCSV to RustyCSV:

1. Replace `:nimble_csv` with `{:rusty_csv, "~> 0.5.0"}` in `mix.exs` and run `mix deps.get`.
2. Replace `NimbleCSV.define/2` with `RustyCSV.define/2`, and swap built-in aliases:

   ```elixir
   # Before
   alias NimbleCSV.RFC4180, as: CSV

   # After
   alias RustyCSV.RFC4180, as: CSV
   ```

3. Check two intentional differences documented in [Compliance and validation](COMPLIANCE.md):
   - `RustyCSV.ParseError` omits raw CSV field contents and reports `:line`, `:column`, `:byte_offset`, and `:reason` instead.
   - When `:escape_formula` matches a field, RustyCSV always quotes the combined `replacement <> original` value. Both libraries parse the output back to the same field value, but the raw CSV bytes differ.
4. If you stream arbitrary binary chunks, you can remove `to_line_stream/1` calls before `parse_stream/2`. RustyCSV accepts arbitrary chunks directly, and `to_line_stream/1` remains available for compatibility.
