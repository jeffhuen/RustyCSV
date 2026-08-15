defmodule RustyCSV.Native do
  @moduledoc """
  Internal low-level NIF bindings for CSV parsing and encoding.

  This module provides direct access to the Rust NIF functions. It is documented
  for maintainers, diagnostics, and advanced debugging, but it is not part of
  RustyCSV's stable public API. Function names, argument shapes, return values,
  and error terms may change between patch releases.

  For normal use, prefer the higher-level `RustyCSV.RFC4180` or custom parsers
  defined with `RustyCSV.define/2`. RustyCSV's SemVer compatibility guarantee
  applies to those high-level parser modules and the `RustyCSV` behaviour, not
  to direct calls into `RustyCSV.Native`.

  ## Separator Format

  The `_with_config` functions accept the separator in three forms:

    * **Integer** — a single-byte separator: `44` (comma), `9` (tab)
    * **Binary** — a single separator, possibly multi-byte: `<<44>>` (comma), `"::"` (double colon)
    * **List of binaries** — multiple separators: `[<<44>>, <<59>>]` (comma or semicolon),
      `[",", "::"]` (comma or double colon)

  ## Escape Format

  The escape (quote character) accepts:

    * **Integer** — a single-byte escape: `34` (double quote)
    * **Binary** — possibly multi-byte: `<<34>>` (double quote), `"$$"` (dollar-dollar)

  ## Strategies

  The module exposes three distinct parsing strategies:

    * **Batch** — `parse_string/1`, `parse_string_fast/1`, `parse_string_indexed/1`,
      and `parse_string_zero_copy/1` are all equivalent. They use the same SIMD
      structural boundary scan and hybrid sub-binary term builder. Multiple names
      are retained for backward compatibility.
    * **Parallel** — `parse_string_parallel/1` uses the same SIMD scan but extracts
      fields in parallel via a rayon thread pool. Best for very large files (500 MB+).
    * **Streaming** — `streaming_*` functions process data in bounded-memory chunks.
      Use for files that exceed available memory or 4 GiB.

  ## Strategy Selection

  | Strategy | Use Case | Memory Model |
  |----------|----------|--------------|
  | `parse_string/1` et al. | Default, most files | Sub-binary (hybrid) |
  | `parse_string_parallel/1` | Large files 500MB+ | Owned copy |
  | `streaming_*` | Unbounded files, >4 GiB | Owned copy (per chunk) |

  ## Scheduling

  All parsing NIFs run on BEAM dirty CPU schedulers to avoid blocking
  normal schedulers. This includes all `parse_string*` functions,
  `streaming_feed/2`, `streaming_next_rows/2`, and `streaming_finalize/1`.

  Parallel parsing runs on a dedicated named `rustycsv-*` rayon thread pool
  (capped at 8 threads) rather than the global rayon pool.

  ## Concurrency

  Streaming parser references (`t:parser_ref/0`) are safe to share across
  BEAM processes — the underlying Rust state is protected by a mutex. If a
  NIF panics while holding the lock, subsequent calls return `:mutex_poisoned`
  instead of crashing the VM.

  ## Memory Tracking (Optional)

  Memory tracking functions are available but require the `memory_tracking`
  Cargo feature to be enabled. Without the feature, they return `0` with
  no runtime overhead.

  See `get_rust_memory/0`, `get_rust_memory_peak/0`, `reset_rust_memory_stats/0`.

  """

  version = Mix.Project.config()[:version]

  avx2_detect = fn _config ->
    try do
      cond do
        File.exists?("/proc/cpuinfo") ->
          case File.read("/proc/cpuinfo") do
            {:ok, info} -> String.contains?(info, "avx2")
            _ -> false
          end

        match?({:win32, _}, :os.type()) ->
          script =
            ~S/Add-Type -Namespace RustyCSV -Name CpuFeatures -MemberDefinition '[DllImport("kernel32.dll")] public static extern bool IsProcessorFeaturePresent(uint feature);'; [RustyCSV.CpuFeatures]::IsProcessorFeaturePresent(40)/

          case System.cmd("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script],
                 stderr_to_stdout: true
               ) do
            {result, 0} -> String.trim(result) == "True"
            _ -> false
          end

        true ->
          # macOS x86_64: check via sysctl
          case System.cmd("sysctl", ["-n", "hw.optional.avx2_0"], stderr_to_stdout: true) do
            {"1\n", 0} -> true
            _ -> false
          end
      end
    rescue
      _ -> false
    end
  end

  x86_64_variants = [avx2: avx2_detect]

  use RustlerPrecompiled,
    otp_app: :rusty_csv,
    crate: "rustycsv",
    base_url: "https://github.com/jeffhuen/rustycsv/releases/download/v#{version}",
    force_build: System.get_env("FORCE_RUSTYCSV_BUILD") in ["1", "true"],
    nif_versions: ["2.15", "2.16", "2.17"],
    targets:
      Enum.uniq(
        ["aarch64-apple-darwin", "x86_64-apple-darwin"] ++
          RustlerPrecompiled.Config.default_targets()
      ),
    variants: %{
      "x86_64-unknown-linux-gnu" => x86_64_variants,
      "x86_64-apple-darwin" => x86_64_variants,
      "x86_64-pc-windows-msvc" => x86_64_variants,
      "x86_64-pc-windows-gnu" => x86_64_variants,
      "x86_64-unknown-linux-musl" => x86_64_variants
    },
    version: version

  # ==========================================================================
  # Types
  # ==========================================================================

  @typedoc "Opaque reference to a streaming parser"
  @opaque parser_ref :: reference()

  @typedoc "A parsed row (list of field binaries)"
  @type row :: [binary()]

  @typedoc "Multiple parsed rows"
  @type rows :: [row()]

  @typedoc "Native parse error"
  @type parse_error ::
          {:error, :input_too_large}
          | {:error,
             {:malformed_csv, :unexpected_quote | :trailing_garbage | :unterminated_quote,
              non_neg_integer()}}

  @typedoc "Native encoding retry signal for wrappers that accept non-binary fields"
  @type encode_error :: {:error, :non_binary_field}

  @typedoc """
  Field separator(s). Accepts:
    * Integer byte (e.g., `44` for comma) — single separator
    * Binary (e.g., `<<44>>` or `<<58, 58>>`) — single separator (possibly multi-byte)
    * List of binaries (e.g., `[<<44>>, <<59>>]`) — multiple separators
  """
  @type separator :: binary() | non_neg_integer() | [binary()]

  @typedoc """
  Quote/escape sequence. Accepts:
    * Integer byte (e.g., `34` for double-quote)
    * Binary (e.g., `<<34>>` or `<<36, 36>>` for `$$`)
  """
  @type escape :: binary() | non_neg_integer()

  # ==========================================================================
  # Batch Parsing (A/B/C/F are equivalent — same SIMD boundary scan)
  # ==========================================================================

  @doc """
  Parse CSV using the SIMD structural boundary scanner. Runs on a dirty CPU scheduler.

  Uses a portable-SIMD scanner to find all field and row boundaries in a
  single pass, then builds Elixir terms using sub-binary references where
  possible (hybrid Cow approach — zero-copy for clean fields, copy only
  when unescaping is needed).

  Functionally equivalent to `parse_string_fast/1`, `parse_string_indexed/1`,
  and `parse_string_zero_copy/1` — all use the same code path. Multiple
  function names are retained for backward API compatibility.

  Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.

  ## Examples

      iex> RustyCSV.Native.parse_string("a,b\\n1,2\\n")
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string(binary()) :: rows() | parse_error()
  def parse_string(_csv), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV with configurable separator(s) and escape characters.

  ## Parameters

    * `csv` - The CSV binary to parse
    * `separator` - Integer byte, binary, or list of binaries (see "Separator Format" above)
    * `escape` - Integer byte or binary (see "Escape Format" above)

  ## Examples

      # TSV parsing with integer separator
      iex> RustyCSV.Native.parse_string_with_config("a\\tb\\n1\\t2\\n", 9, 34)
      [["a", "b"], ["1", "2"]]

      # TSV parsing with binary separator
      iex> RustyCSV.Native.parse_string_with_config("a\\tb\\n1\\t2\\n", <<9>>, 34)
      [["a", "b"], ["1", "2"]]

      # Multi-separator parsing (comma or semicolon)
      iex> RustyCSV.Native.parse_string_with_config("a,b;c\\n1;2,3\\n", [<<44>>, <<59>>], 34)
      [["a", "b", "c"], ["1", "2", "3"]]

      # Multi-byte separator
      iex> RustyCSV.Native.parse_string_with_config("a::b::c\\n", "::", 34)
      [["a", "b", "c"]]

      # Multi-byte escape
      iex> RustyCSV.Native.parse_string_with_config("$$hello$$,world\\n", 44, "$$")
      [["hello", "world"]]

  """
  @spec parse_string_with_config(binary(), separator(), escape(), term()) ::
          rows() | parse_error()
  def parse_string_with_config(csv, separator, escape, newlines),
    do: parse_string_with_config(csv, separator, escape, newlines, true)

  @doc false
  @spec parse_string_with_config(binary(), separator(), escape(), term(), boolean()) ::
          rows() | parse_error()
  def parse_string_with_config(_csv, _separator, _escape, _newlines, _strict),
    do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV using the SIMD structural boundary scanner. Runs on a dirty CPU scheduler.

  Equivalent to `parse_string/1` — same code path, retained for backward
  API compatibility.

  Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.

  ## Examples

      iex> RustyCSV.Native.parse_string_fast("a,b\\n1,2\\n")
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_fast(binary()) :: rows() | parse_error()
  def parse_string_fast(_csv), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV with configurable separator(s) and escape. Equivalent to
  `parse_string_with_config/4`.

  ## Examples

      # TSV parsing with integer separator
      iex> RustyCSV.Native.parse_string_fast_with_config("a\\tb\\n1\\t2\\n", 9, 34)
      [["a", "b"], ["1", "2"]]

      # TSV parsing with binary separator
      iex> RustyCSV.Native.parse_string_fast_with_config("a\\tb\\n1\\t2\\n", <<9>>, 34)
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_fast_with_config(binary(), separator(), escape(), term()) ::
          rows() | parse_error()
  def parse_string_fast_with_config(csv, separator, escape, newlines),
    do: parse_string_fast_with_config(csv, separator, escape, newlines, true)

  @doc false
  @spec parse_string_fast_with_config(binary(), separator(), escape(), term(), boolean()) ::
          rows() | parse_error()
  def parse_string_fast_with_config(_csv, _separator, _escape, _newlines, _strict),
    do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV using the SIMD structural boundary scanner. Runs on a dirty CPU scheduler.

  Equivalent to `parse_string/1` — same code path, retained for backward
  API compatibility.

  Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.

  ## Examples

      iex> RustyCSV.Native.parse_string_indexed("a,b\\n1,2\\n")
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_indexed(binary()) :: rows() | parse_error()
  def parse_string_indexed(_csv), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV with configurable separator(s) and escape. Equivalent to
  `parse_string_with_config/4`.

  ## Examples

      # TSV parsing with integer separator
      iex> RustyCSV.Native.parse_string_indexed_with_config("a\\tb\\n1\\t2\\n", 9, 34)
      [["a", "b"], ["1", "2"]]

      # TSV parsing with binary separator
      iex> RustyCSV.Native.parse_string_indexed_with_config("a\\tb\\n1\\t2\\n", <<9>>, 34)
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_indexed_with_config(binary(), separator(), escape(), term()) ::
          rows() | parse_error()
  def parse_string_indexed_with_config(csv, separator, escape, newlines),
    do: parse_string_indexed_with_config(csv, separator, escape, newlines, true)

  @doc false
  @spec parse_string_indexed_with_config(binary(), separator(), escape(), term(), boolean()) ::
          rows() | parse_error()
  def parse_string_indexed_with_config(_csv, _separator, _escape, _newlines, _strict),
    do: :erlang.nif_error(:nif_not_loaded)

  # ==========================================================================
  # Streaming Parser
  # ==========================================================================

  @doc """
  Create a new streaming parser instance.

  The streaming parser maintains internal state and can process CSV data
  in chunks, making it suitable for large files with bounded memory usage.

  The returned reference is safe to share across BEAM processes — the
  underlying Rust state is protected by a mutex.

  ## Examples

      parser = RustyCSV.Native.streaming_new()
      RustyCSV.Native.streaming_feed(parser, "a,b\\n")
      RustyCSV.Native.streaming_feed(parser, "1,2\\n")
      rows = RustyCSV.Native.streaming_next_rows(parser, 100)

  """
  @spec streaming_new() :: parser_ref()
  def streaming_new, do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Create a new streaming parser with configurable separator(s) and escape.

  ## Parameters

    * `separator` - Integer byte, binary, or list of binaries (see "Separator Format" above)
    * `escape` - Integer byte or binary (see "Escape Format" above)

  ## Examples

      # TSV streaming parser with integer separator
      parser = RustyCSV.Native.streaming_new_with_config(9, 34)

      # TSV streaming parser with binary separator
      parser = RustyCSV.Native.streaming_new_with_config(<<9>>, 34)

      # Multi-separator streaming parser
      parser = RustyCSV.Native.streaming_new_with_config([<<44>>, <<59>>], 34)

      # Multi-byte separator streaming parser
      parser = RustyCSV.Native.streaming_new_with_config("::", 34)

  """
  @spec streaming_new_with_config(separator(), escape(), term()) :: parser_ref()
  def streaming_new_with_config(separator, escape, newlines),
    do: streaming_new_with_config(separator, escape, newlines, true)

  @doc false
  @spec streaming_new_with_config(separator(), escape(), term(), boolean()) :: parser_ref()
  def streaming_new_with_config(_separator, _escape, _newlines, _strict),
    do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Feed a chunk of CSV data to the streaming parser. Runs on a dirty CPU scheduler.

  Returns `{available_rows, buffer_size}` indicating the number of complete
  rows ready to be taken and the current buffer size.

  ## Raises

    * `:buffer_overflow` — the chunk would push the internal buffer past
      the maximum size (default 256 MB). Use `streaming_set_max_buffer/2`
      to adjust the limit.
    * `:mutex_poisoned` — a previous NIF call panicked while holding the
      parser lock. The parser should be discarded.

  ## Examples

      {available, buffer_size} = RustyCSV.Native.streaming_feed(parser, chunk)

  """
  @spec streaming_feed(parser_ref(), binary()) ::
          {non_neg_integer(), non_neg_integer()} | parse_error()
  def streaming_feed(_parser, _chunk), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Take up to `max` complete rows from the streaming parser. Runs on a dirty CPU scheduler.

  Returns the rows as a list of lists of binaries.

  ## Examples

      rows = RustyCSV.Native.streaming_next_rows(parser, 100)

  """
  @spec streaming_next_rows(parser_ref(), non_neg_integer()) :: rows() | parse_error()
  def streaming_next_rows(_parser, _max), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Finalize the streaming parser and get any remaining rows. Runs on a dirty CPU scheduler.

  This should be called after all data has been fed to get any partial
  row that was waiting for a terminating newline.

  ## Examples

      final_rows = RustyCSV.Native.streaming_finalize(parser)

  """
  @spec streaming_finalize(parser_ref()) :: rows() | parse_error()
  def streaming_finalize(_parser), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Get the current status of the streaming parser.

  Returns `{available_rows, buffer_size, has_partial}`:
    * `available_rows` - Number of complete rows ready to be taken
    * `buffer_size` - Current size of the internal buffer in bytes
    * `has_partial` - Whether there's an incomplete row in the buffer

  ## Examples

      {available, buffer, has_partial} = RustyCSV.Native.streaming_status(parser)

  """
  @spec streaming_status(parser_ref()) :: {non_neg_integer(), non_neg_integer(), boolean()}
  def streaming_status(_parser), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Set the maximum buffer size (in bytes) for the streaming parser.
  Default is 256 MB. Raises on overflow during `streaming_feed/2`.
  """
  @spec streaming_set_max_buffer(parser_ref(), non_neg_integer()) :: :ok
  def streaming_set_max_buffer(_parser, _max), do: :erlang.nif_error(:nif_not_loaded)

  # ==========================================================================
  # Parallel Parsing
  # ==========================================================================

  @doc """
  Parse CSV in parallel using multiple threads.

  Uses the rayon thread pool to parse rows in parallel. This is most
  beneficial for very large files (500MB+) where the parallelization
  overhead is outweighed by the parsing speedup.

  This function runs on a dirty CPU scheduler to avoid blocking the
  normal BEAM schedulers.

  ## Examples

      iex> RustyCSV.Native.parse_string_parallel("a,b\\n1,2\\n")
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_parallel(binary()) :: rows() | parse_error()
  def parse_string_parallel(_csv), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV in parallel with configurable separator(s) and escape.

  ## Examples

      # TSV parallel parsing with integer separator
      iex> RustyCSV.Native.parse_string_parallel_with_config("a\\tb\\n1\\t2\\n", 9, 34)
      [["a", "b"], ["1", "2"]]

      # TSV parallel parsing with binary separator
      iex> RustyCSV.Native.parse_string_parallel_with_config("a\\tb\\n1\\t2\\n", <<9>>, 34)
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_parallel_with_config(binary(), separator(), escape(), term()) ::
          rows() | parse_error()
  def parse_string_parallel_with_config(csv, separator, escape, newlines),
    do: parse_string_parallel_with_config(csv, separator, escape, newlines, true)

  @doc false
  @spec parse_string_parallel_with_config(binary(), separator(), escape(), term(), boolean()) ::
          rows() | parse_error()
  def parse_string_parallel_with_config(_csv, _separator, _escape, _newlines, _strict),
    do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV using the SIMD structural boundary scanner. Runs on a dirty CPU scheduler.

  Equivalent to `parse_string/1` — same code path, retained for backward
  API compatibility.

  Returns `{:error, :input_too_large}` if the input exceeds 4 GiB.

  ## Examples

      iex> RustyCSV.Native.parse_string_zero_copy("a,b\\n1,2\\n")
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_zero_copy(binary()) :: rows() | parse_error()
  def parse_string_zero_copy(_csv), do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV with configurable separator(s) and escape. Equivalent to
  `parse_string_with_config/4`.

  ## Examples

      # TSV parsing with integer separator
      iex> RustyCSV.Native.parse_string_zero_copy_with_config("a\\tb\\n1\\t2\\n", 9, 34)
      [["a", "b"], ["1", "2"]]

      # TSV parsing with binary separator
      iex> RustyCSV.Native.parse_string_zero_copy_with_config("a\\tb\\n1\\t2\\n", <<9>>, 34)
      [["a", "b"], ["1", "2"]]

  """
  @spec parse_string_zero_copy_with_config(binary(), separator(), escape(), term()) ::
          rows() | parse_error()
  def parse_string_zero_copy_with_config(csv, separator, escape, newlines),
    do: parse_string_zero_copy_with_config(csv, separator, escape, newlines, true)

  @doc false
  @spec parse_string_zero_copy_with_config(binary(), separator(), escape(), term(), boolean()) ::
          rows() | parse_error()
  def parse_string_zero_copy_with_config(_csv, _separator, _escape, _newlines, _strict),
    do: :erlang.nif_error(:nif_not_loaded)

  # ==========================================================================
  # Headers-to-Maps Extension
  # ==========================================================================

  @doc """
  Parse CSV and return list of maps for the non-parallel batch strategy atoms.

  ## Parameters

    * `input` - The CSV binary to parse
    * `separator` - Separator(s) (see "Separator Format" above)
    * `escape` - Escape sequence (see "Escape Format" above)
    * `strategy` - Non-parallel batch atom: `:basic`, `:simd`, `:indexed`, or
      `:zero_copy`. For `:parallel`, use `parse_to_maps_parallel/6`.
    * `header_mode` - Atom `:true` (first row = keys) or list of key terms
    * `skip_first` - Whether to skip the first row when using explicit keys

  """
  @spec parse_to_maps(binary(), separator(), escape(), term(), atom(), atom() | list(), boolean()) ::
          [map()] | parse_error()
  def parse_to_maps(input, separator, escape, newlines, strategy, header_mode, skip_first),
    do: parse_to_maps(input, separator, escape, newlines, strategy, header_mode, skip_first, true)

  @doc false
  @spec parse_to_maps(
          binary(),
          separator(),
          escape(),
          term(),
          atom(),
          atom() | list(),
          boolean(),
          boolean()
        ) :: [map()] | parse_error()
  def parse_to_maps(
        _input,
        _separator,
        _escape,
        _newlines,
        _strategy,
        _header_mode,
        _skip_first,
        _strict
      ),
      do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Parse CSV in parallel and return list of maps.

  Uses the rayon thread pool on a dirty CPU scheduler.

  ## Parameters

    * `input` - The CSV binary to parse
    * `separator` - Separator(s) (see "Separator Format" above)
    * `escape` - Escape sequence (see "Escape Format" above)
    * `header_mode` - Atom `:true` (first row = keys) or list of key terms
    * `skip_first` - Whether to skip the first row when using explicit keys

  """
  @spec parse_to_maps_parallel(
          binary(),
          separator(),
          escape(),
          term(),
          atom() | list(),
          boolean()
        ) ::
          [map()] | parse_error()
  def parse_to_maps_parallel(input, separator, escape, newlines, header_mode, skip_first),
    do:
      parse_to_maps_parallel(
        input,
        separator,
        escape,
        newlines,
        header_mode,
        skip_first,
        true
      )

  @doc false
  @spec parse_to_maps_parallel(
          binary(),
          separator(),
          escape(),
          term(),
          atom() | list(),
          boolean(),
          boolean()
        ) :: [map()] | parse_error()
  def parse_to_maps_parallel(
        _input,
        _separator,
        _escape,
        _newlines,
        _header_mode,
        _skip_first,
        _strict
      ),
      do: :erlang.nif_error(:nif_not_loaded)

  # ==========================================================================
  # Encoding NIF
  # ==========================================================================

  @doc """
  Encode rows to CSV using SIMD-accelerated scanning.

  Uses portable SIMD to scan 16-32 bytes at a time for characters that need
  escaping. On platforms without SIMD hardware, portable_simd automatically
  degrades to scalar operations. Falls back to a general encoder for
  multi-byte separator/escape sequences.

  Accepts a list of rows, where each row is a list of binary fields.
  Returns one binary per row as iodata. If any field is not a binary, returns
  `{:error, :non_binary_field}` so higher-level wrappers can coerce fields
  without masking unrelated `ArgumentError` failures.

  ## Parameters

    * `rows` - List of rows (list of lists of binaries)
    * `separator` - Field separator (see "Separator Format" above)
    * `escape` - Escape character (see "Escape Format" above)
    * `line_separator` - Line separator binary or `:default` for `"\\n"`

  ## Examples

      iex> RustyCSV.Native.encode_string(
      ...>   [["a", "b"], ["1", "2"]], [","], <<34>>, "\\n", nil, :utf8, []
      ...> )
      ["a,b\\n", "1,2\\n"]

  """
  @spec encode_string(
          [[binary()]],
          separator(),
          escape(),
          binary() | atom(),
          term(),
          term(),
          [binary()]
        ) ::
          iodata() | encode_error()
  def encode_string(_rows, _separator, _escape, _line_separator, _formula, _encoding, _reserved),
    do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Encode rows to CSV in parallel using rayon, returning iodata (list of binaries).

  Uses multiple threads to encode rows simultaneously. Copies all
  field data into Rust-owned memory before dispatching to worker threads.
  If any field is not a binary, returns `{:error, :non_binary_field}` so
  higher-level wrappers can coerce fields without masking unrelated
  `ArgumentError` failures.

  Best for **quoting-heavy data** — fields that frequently contain commas,
  quotes, or newlines (e.g., user-generated content, free-text descriptions).
  The per-field quoting work parallelizes well and outweighs the copy overhead.

  For typical/clean data where most fields pass through unquoted, prefer
  `encode_string/7` which avoids the input copy via term references.

  Only supports single-byte separator/escape. Raises `ArgumentError` for
  multi-byte configurations — use `encode_string/7` instead.

  ## Parameters

    * `rows` - List of rows (list of lists of binaries)
    * `separator` - Field separator (see "Separator Format" above)
    * `escape` - Escape character (see "Escape Format" above)
    * `line_separator` - Line separator binary or `:default` for `"\\n"`

  ## Examples

      iex> RustyCSV.Native.encode_string_parallel(
      ...>   [["a", "b"], ["1", "2"]], [","], <<34>>, "\\r\\n", nil, :utf8, []
      ...> )
      ["a,b\\r\\n", "1,2\\r\\n"]

  """
  @spec encode_string_parallel(
          [[binary()]],
          separator(),
          escape(),
          binary() | atom(),
          term(),
          term(),
          [binary()]
        ) :: iodata() | encode_error()
  def encode_string_parallel(
        _rows,
        _separator,
        _escape,
        _line_separator,
        _formula,
        _encoding,
        _reserved
      ),
      do: :erlang.nif_error(:nif_not_loaded)

  # ==========================================================================
  # Memory Tracking (requires `memory_tracking` feature flag)
  # ==========================================================================

  @doc """
  Get current Rust heap allocation in bytes.

  **Note**: Requires the `memory_tracking` Cargo feature to be enabled.
  Without the feature, this returns `0` with no overhead.

  To enable memory tracking, set the feature in `native/rustycsv/Cargo.toml`:

      [features]
      default = ["mimalloc", "memory_tracking"]

  ## Examples

      bytes = RustyCSV.Native.get_rust_memory()

  """
  @spec get_rust_memory() :: non_neg_integer()
  def get_rust_memory, do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Get peak Rust heap allocation since last reset, in bytes.

  **Note**: Requires the `memory_tracking` Cargo feature. Returns `0` otherwise.

  ## Examples

      peak_bytes = RustyCSV.Native.get_rust_memory_peak()

  """
  @spec get_rust_memory_peak() :: non_neg_integer()
  def get_rust_memory_peak, do: :erlang.nif_error(:nif_not_loaded)

  @doc """
  Reset memory tracking statistics.

  **Note**: Requires the `memory_tracking` Cargo feature. Returns `{0, 0}` otherwise.

  Returns `{current_bytes, previous_peak_bytes}`.

  ## Examples

      {current, peak} = RustyCSV.Native.reset_rust_memory_stats()

  """
  @spec reset_rust_memory_stats() :: {non_neg_integer(), non_neg_integer()}
  def reset_rust_memory_stats, do: :erlang.nif_error(:nif_not_loaded)
end
