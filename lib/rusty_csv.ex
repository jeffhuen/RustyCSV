defmodule RustyCSV do
  @moduledoc ~S"""
  RustyCSV is an ultra-fast CSV parsing and dumping library powered by purpose-built Rust NIFs.

  It provides a drop-in replacement for NimbleCSV with the same API, while offering
  multiple parsing strategies optimized for different use cases.

  ## Quick Start

  Use the pre-defined `RustyCSV.RFC4180` parser:

      alias RustyCSV.RFC4180, as: CSV

      CSV.parse_string("name,age\njohn,27\n")
      #=> [["john", "27"]]

      CSV.parse_string("name,age\njohn,27\n", skip_headers: false)
      #=> [["name", "age"], ["john", "27"]]

  ## Defining Custom Parsers

  You can define custom CSV parsers with `define/2`:

      RustyCSV.define(MyParser,
        separator: ",",
        escape: "\"",
        line_separator: "\n"
      )

      MyParser.parse_string("a,b\n1,2\n")
      #=> [["1", "2"]]

  ## Parsing Strategies

  RustyCSV supports the `:strategy` option for backward compatibility, but
  `:simd`, `:basic`, `:indexed`, and `:zero_copy` are all equivalent — they
  use the same SIMD structural boundary scanner and hybrid sub-binary term
  builder. The only meaningfully distinct strategies are:

    * `:simd` (default) — SIMD structural boundary scan, single-threaded
    * `:parallel` — same SIMD scan, multi-threaded field extraction via rayon
      (best for very large files 500 MB+)

  The streaming parser (used automatically with `parse_stream/2`) is a
  separate stateful approach for bounded-memory processing.

  Example:

      CSV.parse_string(large_csv, strategy: :parallel)

  ## Scheduling

  All parsing NIFs run on BEAM dirty CPU schedulers, so they never block
  normal schedulers. Parallel parsing (`:parallel` strategy) additionally
  runs on a dedicated `rustycsv-*` rayon thread pool to avoid contention
  with other Rayon users in the same VM.

  ## Streaming

  For large files, use `parse_stream/2` which uses a bounded-memory streaming parser:

      "huge.csv"
      |> File.stream!()
      |> CSV.parse_stream()
      |> Stream.each(&process_row/1)
      |> Stream.run()

  Streaming parsers are safe to share across processes — the underlying
  Rust resource is protected by a mutex. However, concurrent access is
  serialized, so for maximum throughput use one parser per process.

  ## Encoding (Dumping)

  Convert rows back to CSV format:

      CSV.dump_to_iodata([["name", "age"], ["john", "27"]])
      #=> "name,age\njohn,27\n"

  Encoding uses SIMD-accelerated Rust NIFs. The default encoder writes CSV bytes
  into a single flat binary. The parallel encoder and BOM-enabled parsers return
  iodata, which can be flattened with `IO.iodata_to_binary/1` when a binary is
  required. The NIF handles four modes: plain UTF-8, UTF-8 with formula escaping,
  non-UTF-8 encoding, and both combined.

  > **Difference from NimbleCSV:** NimbleCSV's `dump_to_iodata/1` returns an
  > iodata list (a nested list of small binaries) that callers typically flatten
  > back into a single binary via `IO.iodata_to_binary/1` before writing to a
  > file, sending as a download, or passing to an API. RustyCSV's default encoder
  > skips that roundtrip and returns the final binary directly, ready for use with
  > `IO.binwrite/2`, `Conn.send_resp/3`, `:gen_tcp.send/2`, `File.write/2`,
  > etc. The output bytes are identical.
  >
  > Code that pattern-matches on the return value expecting a list will need
  > adjustment. This is a deliberate trade-off: building an iodata list across
  > the NIF boundary requires allocating one Erlang term per field, separator,
  > and newline, which is 18–63% slower and uses 3–6x more NIF memory than
  > returning the bytes directly.

  ### Encoding Strategies

  `dump_to_iodata/2` accepts a `:strategy` option:

    * *default* (no option) — Single-threaded SIMD-accelerated encoder.
      Writes all CSV bytes into a single flat binary. Best for most workloads.

    * `:parallel` — Multi-threaded encoding via rayon. Copies all field data
      into Rust-owned memory, splits rows into chunks, and encodes each chunk
      on a separate thread. Returns a short list of large binaries. Best for
      quoting-heavy data (user-generated content with embedded commas/quotes/newlines).

  Example:

      # Default (recommended for most cases)
      CSV.dump_to_iodata(rows)

      # Parallel (opt in for quoting-heavy data)
      CSV.dump_to_iodata(rows, strategy: :parallel)

  ### High-Throughput Concurrent Exports

  The encoding NIF runs on dirty CPU schedulers with per-thread mimalloc
  arenas, making it suitable for concurrent export workloads — e.g.,
  thousands of users downloading CSV reports simultaneously:

      # Phoenix controller — each request encodes independently
      rows = MyApp.Reports.fetch_rows(user_id)
      csv = MyCSV.dump_to_iodata(rows)
      send_download(conn, {:binary, csv}, filename: "report.csv")

  For very large exports, use chunked NIF encoding for bounded memory:

      MyApp.Reports.stream_rows(user_id)
      |> Stream.chunk_every(5_000)
      |> Stream.map(&MyCSV.dump_to_iodata/1)
      |> Enum.each(&Conn.chunk(conn, &1))

  ## NimbleCSV Compatibility

  RustyCSV is designed as a drop-in replacement for NimbleCSV. The API is identical:

    * `parse_string/2` - Parse CSV string to list of rows
    * `parse_stream/2` - Lazily parse a stream
    * `parse_enumerable/2` - Parse any enumerable
    * `dump_to_iodata/2` - Convert rows to iodata (the default encoder returns a flat binary — see "Encoding" section)
    * `dump_to_stream/1` - Lazily convert rows to iodata stream
    * `to_line_stream/1` - Convert arbitrary chunks to lines
    * `options/0` - Return module configuration

  RustyCSV extends NimbleCSV with additional options:

    * `:strategy` on `parse_string/2` - Select the parsing approach (`:simd`,
      `:basic`, `:indexed`, `:parallel`, `:zero_copy`)
    * `:strategy` on `dump_to_iodata/2` - Select the encoding approach
      (default or `:parallel`)
    * `:headers` - Return rows as maps instead of lists

  ## Headers-to-Maps

  Use the `:headers` option to get maps instead of lists:

      CSV.parse_string("name,age\njohn,27\n", headers: true)
      #=> [%{"name" => "john", "age" => "27"}]

      CSV.parse_string("name,age\njohn,27\n", headers: [:name, :age])
      #=> [%{name: "john", age: "27"}]

      CSV.parse_string("name,age\njohn,27\n", headers: ["n", "a"])
      #=> [%{"n" => "john", "a" => "27"}]

  Streaming also supports headers:

      "huge.csv"
      |> File.stream!()
      |> CSV.parse_stream(headers: true)
      |> Stream.each(&process_map/1)
      |> Stream.run()

  ### How `:headers` interacts with `:skip_headers`

  With `headers: true`, the first row is always consumed as keys — `:skip_headers`
  has no effect.

  With `headers: [keys]`, the `:skip_headers` option controls whether the first
  row is skipped (default: `true`). Most CSV files have a header row, so skipping
  it avoids mapping the header row itself into a map. If your file has no header
  row, pass `skip_headers: false`:

      # File with header row (typical) — first row skipped by default
      CSV.parse_string("name,age\njohn,27\n", headers: [:n, :a])
      #=> [%{n: "john", a: "27"}]

      # File without header row — include all rows
      CSV.parse_string("john,27\njane,30\n", headers: [:n, :a], skip_headers: false)
      #=> [%{n: "john", a: "27"}, %{n: "jane", a: "30"}]

  ### Edge cases

    * Fewer columns than keys — missing values are `nil`
    * More columns than keys — extra columns are ignored
    * Duplicate headers — last column wins
    * Empty header field — key is `""`

  ## Multi-Separator Support

  Like NimbleCSV, RustyCSV supports multiple separator characters. Separators
  can be single-byte or multi-byte:

      RustyCSV.define(MyParser,
        separator: [",", ";"],
        escape: "\""
      )

      # Any separator in the list is recognized when parsing
      MyParser.parse_string("a,b;c\\n1;2,3\\n", skip_headers: false)
      #=> [["a", "b", "c"], ["1", "2", "3"]]

      # Only the FIRST separator is used when dumping
      MyParser.dump_to_iodata([["a", "b", "c"]]) |> IO.iodata_to_binary()
      #=> "a,b,c\\n"

  Multi-byte separators are supported:

      RustyCSV.define(MyParser,
        separator: "::",
        escape: "\""
      )

      MyParser.parse_string("a::b::c\\n", skip_headers: false)
      #=> [["a", "b", "c"]]

  You can also mix single-byte and multi-byte separators:

      RustyCSV.define(MyParser,
        separator: [",", "::"],
        escape: "\""
      )

  ## Multi-Byte Escape Support

  Escape sequences can also be multi-byte:

      RustyCSV.define(MyParser,
        separator: ",",
        escape: "$$"
      )

      MyParser.parse_string("$$hello$$,world\\n", skip_headers: false)
      #=> [["hello", "world"]]

  ## Encoding Support

  RustyCSV supports character encoding conversion via the `:encoding` option.
  This is useful when exporting CSVs with non-ASCII characters (accents, CJK,
  emoji) that need to open correctly in spreadsheet applications:

      alias RustyCSV.Spreadsheet

      # Export data with international characters for Excel/Google Sheets/Numbers
      rows = [["名前", "年齢"], ["田中", "27"], ["Müller", "35"]]
      csv = Spreadsheet.dump_to_iodata(rows) |> IO.iodata_to_binary()
      File.write!("export.csv", csv)

  The pre-defined `RustyCSV.Spreadsheet` module outputs UTF-16 LE with BOM,
  which spreadsheet applications auto-detect correctly. You can also define
  custom encodings:

      RustyCSV.define(MySpreadsheet,
        separator: "\t",
        encoding: {:utf16, :little},
        trim_bom: true,
        dump_bom: true
      )

  Supported encodings:
    * `:utf8` - UTF-8 (default, no conversion overhead)
    * `:latin1` - ISO-8859-1 / Latin-1
    * `{:utf16, :little}` - UTF-16 Little Endian
    * `{:utf16, :big}` - UTF-16 Big Endian
    * `{:utf32, :little}` - UTF-32 Little Endian
    * `{:utf32, :big}` - UTF-32 Big Endian

  """

  # ==========================================================================
  # Types
  # ==========================================================================

  @typedoc """
  A single row of CSV data, represented as a list of field binaries.
  """
  @type row :: [binary()]

  @typedoc """
  Multiple rows of CSV data.
  """
  @type rows :: [row()]

  @typedoc """
  Error returned by batch parsers when the input exceeds the native index limit.
  """
  @type parse_error :: {:error, :input_too_large}

  @typedoc """
  Result returned by parsing functions.
  """
  @type parse_result :: rows() | [map()] | parse_error()

  @typedoc """
  Parsing strategy to use.

  These strategies apply to `parse_string/2` and other parsing functions.
  For encoding strategies, see `t:dump_options/0`.

  ## Available Strategies

    * `:simd` (default) — SIMD structural boundary scan, single-threaded.
    * `:basic` — alias for `:simd` (retained for backward compatibility).
    * `:indexed` — alias for `:simd` (retained for backward compatibility).
    * `:parallel` — same SIMD scan, multi-threaded field extraction via rayon.
      Best for very large files (500 MB+).
    * `:zero_copy` — alias for `:simd` (retained for backward compatibility).

  > #### Strategy equivalence {: .info}
  >
  > `:simd`, `:basic`, `:indexed`, and `:zero_copy` all execute the same
  > code path: a portable-SIMD structural scanner followed by a hybrid
  > sub-binary term builder. They produce identical results with identical
  > performance. The names are kept for API stability.

  ## Memory Model

  All batch strategies use boundary-based parsing: the NIF scans the input
  to find field boundaries, then returns sub-binary references for clean
  fields (zero-copy) and only allocates new binaries for fields that
  require unescaping. The input binary is kept alive while any sub-binary
  references it.

  | Strategy | Best When |
  |----------|-----------|
  | `:simd` | Default, fastest for most files |
  | `:parallel` | Large files 500 MB+, complex quoting |

  ## Examples

      # Default strategy
      CSV.parse_string(data)

      # Parallel for large files
      CSV.parse_string(large_data, strategy: :parallel)

  """
  @type strategy :: :simd | :basic | :indexed | :parallel | :zero_copy

  @typedoc """
  Options for parsing functions.

  ## Common Options

    * `:skip_headers` - When `true`, skips the first row. Defaults to `true`.
    * `:strategy` - The parsing strategy to use. One of:
      * `:simd` - SIMD structural boundary scan (default)
      * `:basic` - Alias for `:simd`
      * `:indexed` - Alias for `:simd`
      * `:parallel` - Multi-threaded via rayon
      * `:zero_copy` - Alias for `:simd`
    * `:headers` - Controls header handling. Defaults to `false`.
      * `false` - Return rows as lists (default behavior)
      * `true` - Use first row as string keys, return list of maps.
        `:skip_headers` is ignored (first row is always consumed as keys).
      * list of atoms or strings - Use as explicit keys, return list of maps.
        The first row is skipped by default (`:skip_headers` applies). Pass
        `skip_headers: false` if the file has no header row.

  ## Limits

    * **Input size** - Batch parsing (all strategies except streaming) is limited
      to inputs of at most 4 GiB (`u32::MAX` bytes) because the SIMD structural
      scanner uses 32-bit positions. Passing a larger binary returns
      `{:error, :input_too_large}`. Use the streaming parser for files exceeding
      this limit.

  ## Streaming Options

    * `:chunk_size` - Bytes per IO read for streaming. Defaults to `65536`.
    * `:batch_size` - Rows per batch for streaming. Defaults to `1000`.
    * `:max_buffer_size` - Maximum streaming buffer size in bytes. Defaults to
      `268_435_456` (256 MB). If the internal buffer exceeds this limit during
      `streaming_feed/2`, a `:buffer_overflow` exception is raised. Increase
      this if your data contains rows longer than 256 MB. Decrease it to fail
      faster on malformed input that lacks newlines.

  """
  @type parse_options :: [
          skip_headers: boolean(),
          strategy: strategy(),
          headers: boolean() | [atom() | String.t()],
          chunk_size: pos_integer(),
          batch_size: pos_integer(),
          max_buffer_size: pos_integer()
        ]

  @typedoc """
  Options for `dump_to_iodata/2`.

  ## Options

    * `:strategy` - Encoding strategy to use. Defaults to the single-threaded
      SIMD-accelerated encoder (no option needed). Pass `:parallel` for
      multi-threaded encoding via rayon, which is faster for quoting-heavy data.

  """
  @type dump_options :: [strategy: :parallel]

  @typedoc """
  Encoding for CSV data.

  Supported encodings:
    * `:utf8` - UTF-8 (default, no conversion)
    * `:latin1` - ISO-8859-1 / Latin-1
    * `{:utf16, :little}` - UTF-16 Little Endian
    * `{:utf16, :big}` - UTF-16 Big Endian
    * `{:utf32, :little}` - UTF-32 Little Endian
    * `{:utf32, :big}` - UTF-32 Big Endian
  """
  @type encoding :: :utf8 | :latin1 | {:utf16, :little | :big} | {:utf32, :little | :big}

  @typedoc """
  Options for `define/2`.

  ## Parsing Options

    * `:separator` - Field separator character(s). Can be a single string (e.g., `","`)
      or a list of strings for multi-separator support (e.g., `[",", ";"]`).
      When parsing, any separator in the list is recognized as a field delimiter.
      When dumping, only the **first** separator is used for output.
      Defaults to `","`.
    * `:escape` - Escape/quote character. Defaults to `"\""`.
    * `:newlines` - List of recognized line endings. Defaults to `["\r\n", "\n"]`.
    * `:trim_bom` - Remove BOM when parsing strings. Defaults to `false`.
    * `:encoding` - Character encoding. Defaults to `:utf8`. See `t:encoding/0`.

  ## Dumping Options

    * `:line_separator` - Line separator for output. Defaults to `"\n"`.
    * `:dump_bom` - Include BOM in output. Defaults to `false`.
    * `:reserved` - Additional characters requiring escaping.
    * `:escape_formula` - Map for formula injection prevention. Defaults to `nil`.
      When set, fields starting with trigger characters are prefixed with a
      replacement string inside quotes. Handled natively in the Rust NIF.

  ## Other Options

    * `:strategy` - Default parsing strategy. Defaults to `:simd`.
    * `:moduledoc` - Documentation for the generated module.

  """
  @type define_options :: [
          separator: String.t() | [String.t()],
          escape: String.t(),
          newlines: [String.t()],
          line_separator: String.t(),
          trim_bom: boolean(),
          dump_bom: boolean(),
          reserved: [String.t()],
          escape_formula: map() | nil,
          encoding: encoding(),
          strategy: strategy(),
          moduledoc: String.t() | false | nil
        ]

  # ==========================================================================
  # Exceptions
  # ==========================================================================

  defmodule ParseError do
    @moduledoc """
    Exception raised when CSV parsing fails.

    ## Fields

      * `:message` - Human-readable error description

    """
    defexception [:message]

    @impl true
    def message(%{message: message}), do: message
  end

  # ==========================================================================
  # Callbacks (Behaviour)
  # ==========================================================================

  @doc """
  Returns the options used to define this CSV module.
  """
  @callback options() :: keyword()

  @doc """
  Parses a CSV string into a list of rows.
  """
  @callback parse_string(binary()) :: rows() | parse_error()

  @doc """
  Parses a CSV string into a list of rows with options.
  """
  @callback parse_string(binary(), parse_options()) :: parse_result()

  @doc """
  Lazily parses a stream of CSV data into a stream of rows.
  """
  @callback parse_stream(Enumerable.t()) :: Enumerable.t()

  @doc """
  Lazily parses a stream of CSV data into a stream of rows with options.
  """
  @callback parse_stream(Enumerable.t(), parse_options()) :: Enumerable.t()

  @doc """
  Eagerly parses an enumerable of CSV data into a list of rows.
  """
  @callback parse_enumerable(Enumerable.t()) :: rows() | [map()]

  @doc """
  Eagerly parses an enumerable of CSV data into a list of rows with options.
  """
  @callback parse_enumerable(Enumerable.t(), parse_options()) :: rows() | [map()]

  @doc """
  Converts rows to iodata in CSV format.

  Returns iodata. For the default encoder without a BOM this is a single flat
  binary. `:parallel` encoding and BOM-enabled modules may return list-shaped
  iodata. Use `IO.iodata_to_binary/1` when a binary is required. See
  "Encoding (Dumping)" in the module doc for details on how this differs from
  NimbleCSV.

  ## Options

    * `:strategy` - Encoding strategy. Defaults to the single-threaded
      SIMD-accelerated encoder. Pass `:parallel` for multi-threaded encoding
      via rayon, which is faster for quoting-heavy data.

  """
  @callback dump_to_iodata(Enumerable.t()) :: iodata()
  @callback dump_to_iodata(Enumerable.t(), dump_options()) :: iodata()

  @doc """
  Lazily converts rows to a stream of iodata in CSV format.
  """
  @callback dump_to_stream(Enumerable.t()) :: Enumerable.t()

  @doc """
  Converts a stream of arbitrary binary chunks into a line-oriented stream.
  """
  @callback to_line_stream(Enumerable.t()) :: Enumerable.t()

  # ==========================================================================
  # Module Definition
  # ==========================================================================

  @doc ~S"""
  Defines a new CSV parser/dumper module.

  ## Options

  ### Parsing Options

    * `:separator` - The field separator(s). Can be a single string
      (e.g., `","`, `"::"`) or a list of strings for multi-separator support
      (e.g., `[",", ";"]`, `[",", "::"]`). Separators can be multi-byte.
      Defaults to `","`.

      When multiple separators are specified:
      - **Parsing**: Any separator in the list is recognized as a field delimiter
      - **Dumping**: Only the **first** separator is used for output

      This is useful for parsing files with inconsistent delimiters or mixed
      comma/semicolon separators (common in European locales).

    * `:escape` - The escape/quote sequence. Can be multi-byte (e.g., `"$$"`).
      Defaults to `"\""`.

    * `:newlines` - List of recognized line endings for parsing.
      Defaults to `["\r\n", "\n"]`. Both CRLF and LF are always recognized.

    * `:trim_bom` - When `true`, removes the BOM (byte order marker)
      from the beginning of strings before parsing. Defaults to `false`.

    * `:encoding` - Character encoding for input/output. Defaults to `:utf8`.
      Supported encodings:
      * `:utf8` - UTF-8 (default, no conversion overhead)
      * `:latin1` - ISO-8859-1 / Latin-1
      * `{:utf16, :little}` - UTF-16 Little Endian
      * `{:utf16, :big}` - UTF-16 Big Endian
      * `{:utf32, :little}` - UTF-32 Little Endian
      * `{:utf32, :big}` - UTF-32 Big Endian

      When encoding is not `:utf8`, input data is converted to UTF-8 for
      parsing, and output is converted back to the target encoding.

  ### Dumping Options

    * `:line_separator` - The line separator for dumped output.
      Defaults to `"\n"`.

    * `:dump_bom` - When `true`, includes the appropriate BOM at the start of
      dumped output. Defaults to `false`.

    * `:reserved` - Additional characters that should trigger field escaping
      when dumping. By default, fields containing the separator, escape
      character, or newlines are escaped.

    * `:escape_formula` - A map of characters to their escaped versions
      for preventing CSV formula injection. When set, fields starting with
      these characters will be prefixed with a tab. Defaults to `nil`.

      Example: `%{"=" => true, "+" => true, "-" => true, "@" => true}`

  ### Strategy Options

    * `:strategy` - The default parsing strategy. One of:
      * `:simd` - SIMD structural boundary scan (default)
      * `:basic` - Alias for `:simd`
      * `:indexed` - Alias for `:simd`
      * `:parallel` - Multi-threaded via rayon
      * `:zero_copy` - Alias for `:simd`

  ### Documentation

    * `:moduledoc` - The `@moduledoc` for the generated module.
      Set to `false` to disable documentation.

  ## Examples

      # Define a standard CSV parser
      RustyCSV.define(MyApp.CSV,
        separator: ",",
        escape: "\"",
        line_separator: "\n"
      )

      # Use it
      MyApp.CSV.parse_string("a,b\n1,2\n")
      #=> [["1", "2"]]

      # Define a UTF-16 spreadsheet parser
      RustyCSV.define(MyApp.Spreadsheet,
        separator: "\t",
        encoding: {:utf16, :little},
        trim_bom: true,
        dump_bom: true
      )

      # Define a multi-separator parser (comma or semicolon)
      RustyCSV.define(MyApp.FlexibleCSV,
        separator: [",", ";"],
        escape: "\""
      )

      # Parse files with mixed delimiters
      MyApp.FlexibleCSV.parse_string("a,b;c\n1;2,3\n", skip_headers: false)
      #=> [["a", "b", "c"], ["1", "2", "3"]]

      # Dumping uses the first separator (comma)
      MyApp.FlexibleCSV.dump_to_iodata([["x", "y"]]) |> IO.iodata_to_binary()
      #=> "x,y\n"

      # Get the configuration
      MyApp.CSV.options()
      #=> [separator: ",", escape: "\"", ...]

  """
  @spec define(module(), define_options()) :: :ok
  def define(module, options \\ []) do
    config = extract_and_validate_options(options)
    compile_module(module, config)
    :ok
  end

  # ==========================================================================
  # Private: Option Extraction and Validation
  # ==========================================================================

  defp extract_and_validate_options(options) do
    separator = Keyword.get(options, :separator, ",") |> normalize_codepoint()
    escape = Keyword.get(options, :escape, "\"") |> normalize_codepoint()

    # Validate and normalize separator(s)
    {separator_list, separator_binaries} = validate_and_normalize_separator!(separator)
    # First separator is used for dumping (NimbleCSV compatibility)
    first_separator = hd(separator_list)

    validate_non_empty!(:escape, escape)
    escape_binary = escape

    line_separator = Keyword.get(options, :line_separator, "\n")
    newlines = Keyword.get(options, :newlines, ["\r\n", "\n"])
    trim_bom = Keyword.get(options, :trim_bom, false)
    dump_bom = Keyword.get(options, :dump_bom, false)
    reserved = Keyword.get(options, :reserved, [])
    reserved_binaries = Enum.map(reserved, &normalize_codepoint/1)
    escape_formula = Keyword.get(options, :escape_formula, nil)
    default_strategy = Keyword.get(options, :strategy, :simd)
    moduledoc = Keyword.get(options, :moduledoc)

    # Encoding support
    encoding = Keyword.get(options, :encoding, :utf8)
    validate_encoding!(encoding)
    bom = :unicode.encoding_to_bom(encoding)

    stored_options = [
      separator: separator_list,
      escape: escape,
      line_separator: line_separator,
      newlines: newlines,
      trim_bom: trim_bom,
      dump_bom: dump_bom,
      reserved: reserved,
      escape_formula: escape_formula,
      encoding: encoding,
      strategy: default_strategy
    ]

    %{
      separator: first_separator,
      separator_binaries: separator_binaries,
      escape: escape,
      escape_binary: escape_binary,
      line_separator: line_separator,
      newlines: newlines,
      trim_bom: trim_bom,
      dump_bom: dump_bom,
      escape_formula: escape_formula,
      default_strategy: default_strategy,
      stored_options: stored_options,
      moduledoc: moduledoc,
      encoding: encoding,
      bom: bom,
      reserved_binaries: reserved_binaries
    }
  end

  defp normalize_codepoint(value) when is_integer(value), do: <<value::utf8>>
  defp normalize_codepoint(value), do: value

  defp validate_non_empty!(name, value) do
    unless is_binary(value) and byte_size(value) >= 1 do
      raise ArgumentError,
            "RustyCSV requires a non-empty binary #{name}, got: #{inspect(value)}"
    end
  end

  # Validates and normalizes separator option.
  # Accepts either a string or a list of strings (each can be multi-byte).
  # Returns {list_of_separator_strings, list_of_separator_binaries}
  defp validate_and_normalize_separator!(separator) when is_binary(separator) do
    validate_non_empty!(:separator, separator)
    {[separator], [separator]}
  end

  defp validate_and_normalize_separator!(separators) when is_list(separators) do
    if Enum.empty?(separators) do
      raise ArgumentError, "RustyCSV separator list cannot be empty"
    end

    # Normalize integer codepoints in lists
    separators = Enum.map(separators, &normalize_codepoint/1)

    Enum.each(separators, fn sep ->
      unless is_binary(sep) and byte_size(sep) >= 1 do
        raise ArgumentError,
              "RustyCSV requires each separator to be a non-empty string, got: #{inspect(sep)}"
      end
    end)

    {separators, separators}
  end

  defp validate_and_normalize_separator!(other) do
    raise ArgumentError,
          "RustyCSV separator must be a string or list of strings, got: #{inspect(other)}"
  end

  defp validate_encoding!(encoding) when encoding in [:utf8, :latin1], do: :ok
  defp validate_encoding!({:utf16, endian}) when endian in [:little, :big], do: :ok
  defp validate_encoding!({:utf32, endian}) when endian in [:little, :big], do: :ok

  defp validate_encoding!(encoding) do
    raise ArgumentError,
          "Invalid encoding: #{inspect(encoding)}. " <>
            "Supported: :utf8, :latin1, {:utf16, :little}, {:utf16, :big}, {:utf32, :little}, {:utf32, :big}"
  end

  # ==========================================================================
  # Private: Module Compilation
  # ==========================================================================

  defp compile_module(module, config) do
    quoted_ast =
      quote do
        defmodule unquote(module) do
          unquote(quoted_module_header(config))
          unquote(quoted_config_function(config))
          unquote_splicing(quoted_parsing_functions(config))
          unquote_splicing(quoted_dumping_functions())
        end
      end

    Code.compile_quoted(quoted_ast)
  end

  # ==========================================================================
  # Private: AST Generation Helpers
  # ==========================================================================

  defp build_formula_nif_config(nil), do: nil

  defp build_formula_nif_config(map) when is_map(map) do
    map
    |> Enum.flat_map(&expand_formula_entry/1)
  end

  defp expand_formula_entry({prefixes, replacement}) do
    prefixes = if is_list(prefixes), do: prefixes, else: [prefixes]
    replacement_bin = if is_binary(replacement), do: replacement, else: to_string(replacement)

    Enum.map(prefixes, fn prefix ->
      <<first_byte, _rest::binary>> = prefix
      {first_byte, replacement_bin}
    end)
  end

  defp quoted_module_header(config) do
    quote do
      @moduledoc unquote(Macro.escape(config.moduledoc))
      @behaviour RustyCSV

      @separator unquote(Macro.escape(config.separator))
      @separator_binaries unquote(Macro.escape(config.separator_binaries))
      @escape unquote(Macro.escape(config.escape))
      @escape_binary unquote(Macro.escape(config.escape_binary))
      @line_separator unquote(Macro.escape(config.line_separator))
      @newlines unquote(Macro.escape(config.newlines))
      @newlines_nif unquote(
                      Macro.escape(
                        if config.newlines == ["\r\n", "\n"] do
                          :default
                        else
                          config.newlines
                        end
                      )
                    )
      @trim_bom unquote(Macro.escape(config.trim_bom))
      @dump_bom unquote(Macro.escape(config.dump_bom))
      @formula_nif_config unquote(Macro.escape(build_formula_nif_config(config.escape_formula)))
      @default_strategy unquote(Macro.escape(config.default_strategy))
      @stored_options unquote(Macro.escape(config.stored_options))
      @encoding unquote(Macro.escape(config.encoding))
      @bom unquote(Macro.escape(config.bom))
      @reserved_binaries unquote(Macro.escape(config.reserved_binaries))
    end
  end

  defp quoted_config_function(config) do
    quote do
      @doc """
      Returns the options used to define this CSV module.
      """
      @impl RustyCSV
      @spec options() :: keyword()
      def options, do: unquote(Macro.escape(config.stored_options))
    end
  end

  defp quoted_parsing_functions(config) do
    List.flatten([
      quoted_parse_string_function(config),
      quoted_parse_stream_function(),
      quoted_parse_enumerable_function(),
      quoted_to_line_stream_function()
    ])
  end

  defp quoted_parse_string_function(config) do
    [
      quoted_parse_string_main(config.encoding),
      quoted_parse_string_headers_clauses(),
      quoted_parse_to_maps_clauses(),
      quoted_maybe_trim_bom(config.trim_bom),
      quoted_maybe_to_utf8(config.encoding),
      quoted_do_parse_string_clauses()
    ]
  end

  defp quoted_parse_string_main(encoding) do
    encoding_doc =
      if encoding == :utf8,
        do: "",
        else:
          "\n\n  Input is expected in #{inspect(encoding)} encoding and will be converted to UTF-8 for parsing."

    quote do
      @doc """
      Parses a CSV string into a list of rows.

      ## Options

        * `:skip_headers` - When `true`, skips the first row. Defaults to `true`.
        * `:strategy` - The parsing strategy. Defaults to `#{inspect(@default_strategy)}`.
        * `:headers` - Controls header handling. Defaults to `false`.
          * `false` - Return rows as lists (default behavior)
          * `true` - Use first row as string keys, return maps.
            `:skip_headers` is ignored.
          * `[atom | string, ...]` - Use explicit keys, return maps.
            First row skipped by default; pass `skip_headers: false` if no header row.
      #{unquote(encoding_doc)}
      """
      @impl RustyCSV
      @spec parse_string(binary(), RustyCSV.parse_options()) :: RustyCSV.parse_result()
      def parse_string(string, opts \\ [])

      def parse_string(string, opts) when is_binary(string) and is_list(opts) do
        headers = Keyword.get(opts, :headers, false)
        strategy = Keyword.get(opts, :strategy, @default_strategy)
        string = string |> maybe_trim_bom() |> maybe_to_utf8()
        do_parse_string_with_headers(string, strategy, headers, opts)
      end
    end
  end

  defp quoted_parse_string_headers_clauses do
    quote do
      defp do_parse_string_with_headers(string, strategy, false, opts) do
        skip_headers = Keyword.get(opts, :skip_headers, true)
        rows = do_parse_string(string, strategy)

        case {skip_headers, rows} do
          {true, [_ | tail]} -> tail
          _ -> rows
        end
      end

      defp do_parse_string_with_headers(string, strategy, true, _opts) do
        do_parse_to_maps(string, strategy, true, true)
      end

      defp do_parse_string_with_headers(string, strategy, header_list, opts)
           when is_list(header_list) do
        skip_headers = Keyword.get(opts, :skip_headers, true)
        do_parse_to_maps(string, strategy, header_list, skip_headers)
      end

      defp do_parse_string_with_headers(_string, _strategy, other, _opts) do
        raise ArgumentError,
              "invalid :headers option, expected false, true, or a list of keys, got: #{inspect(other)}"
      end
    end
  end

  defp quoted_parse_to_maps_clauses do
    quote do
      defp do_parse_to_maps(string, :parallel, header_mode, skip_first) do
        RustyCSV.Native.parse_to_maps_parallel(
          string,
          @separator_binaries,
          @escape_binary,
          @newlines_nif,
          header_mode,
          skip_first
        )
      end

      defp do_parse_to_maps(string, strategy, header_mode, skip_first) do
        RustyCSV.Native.parse_to_maps(
          string,
          @separator_binaries,
          @escape_binary,
          @newlines_nif,
          strategy,
          header_mode,
          skip_first
        )
      end
    end
  end

  defp quoted_maybe_trim_bom(true) do
    quote do
      defp maybe_trim_bom(<<@bom, rest::binary>>), do: rest
      defp maybe_trim_bom(string), do: string
    end
  end

  defp quoted_maybe_trim_bom(false) do
    quote do
      defp maybe_trim_bom(string), do: string
    end
  end

  # For UTF-8, encoding conversion is a no-op
  defp quoted_maybe_to_utf8(:utf8) do
    quote do
      defp maybe_to_utf8(data), do: data
    end
  end

  # For other encodings, convert to UTF-8 using :unicode module
  defp quoted_maybe_to_utf8(encoding) do
    quote do
      defp maybe_to_utf8(data) do
        case :unicode.characters_to_binary(data, unquote(Macro.escape(encoding)), :utf8) do
          binary when is_binary(binary) ->
            binary

          {:incomplete, converted, rest} ->
            raise RustyCSV.ParseError,
              message:
                "Incomplete #{inspect(unquote(Macro.escape(encoding)))} sequence: " <>
                  "converted #{byte_size(converted)} bytes, #{byte_size(rest)} bytes remaining"

          {:error, converted, rest} ->
            raise RustyCSV.ParseError,
              message:
                "Invalid #{inspect(unquote(Macro.escape(encoding)))} sequence at byte #{byte_size(converted)}: " <>
                  "#{inspect(binary_part(rest, 0, min(byte_size(rest), 10)))}"
        end
      end
    end
  end

  defp quoted_do_parse_string_clauses do
    quote do
      defp do_parse_string(string, :basic) do
        RustyCSV.Native.parse_string_with_config(
          string,
          @separator_binaries,
          @escape_binary,
          @newlines_nif
        )
      end

      defp do_parse_string(string, :simd) do
        RustyCSV.Native.parse_string_fast_with_config(
          string,
          @separator_binaries,
          @escape_binary,
          @newlines_nif
        )
      end

      defp do_parse_string(string, :indexed) do
        RustyCSV.Native.parse_string_indexed_with_config(
          string,
          @separator_binaries,
          @escape_binary,
          @newlines_nif
        )
      end

      defp do_parse_string(string, :parallel) do
        RustyCSV.Native.parse_string_parallel_with_config(
          string,
          @separator_binaries,
          @escape_binary,
          @newlines_nif
        )
      end

      defp do_parse_string(string, :zero_copy) do
        RustyCSV.Native.parse_string_zero_copy_with_config(
          string,
          @separator_binaries,
          @escape_binary,
          @newlines_nif
        )
      end
    end
  end

  defp quoted_parse_stream_function do
    [
      quoted_parse_stream_main(),
      quoted_stream_headers_clauses(),
      quoted_zip_to_map()
    ]
  end

  defp quoted_parse_stream_main do
    quote do
      @doc """
      Lazily parses a stream of CSV data into a stream of rows.

      ## Options

        * `:skip_headers` - When `true`, skips the first row. Defaults to `true`.
        * `:headers` - Controls header handling. Defaults to `false`.
          * `false` - Return rows as lists (default behavior)
          * `true` - Use first row as string keys, return maps.
            `:skip_headers` is ignored.
          * `[atom | string, ...]` - Use explicit keys, return maps.
            First row skipped by default; pass `skip_headers: false` if no header row.
        * `:chunk_size` - Bytes per IO read. Defaults to `65536`.
        * `:batch_size` - Rows per batch. Defaults to `1000`.
        * `:max_buffer_size` - Maximum streaming buffer size in bytes.
          Defaults to `268_435_456` (256 MB). Raises if exceeded during parsing.

      """
      @impl RustyCSV
      @spec parse_stream(Enumerable.t(), RustyCSV.parse_options()) :: Enumerable.t()
      def parse_stream(stream, opts \\ [])

      def parse_stream(stream, opts) when is_list(opts) do
        headers = Keyword.get(opts, :headers, false)
        chunk_size = Keyword.get(opts, :chunk_size, 64 * 1024)
        batch_size = Keyword.get(opts, :batch_size, 1000)

        stream_opts = [
          chunk_size: chunk_size,
          batch_size: batch_size,
          separator: @separator_binaries,
          escape: @escape_binary,
          newlines: @newlines_nif,
          encoding: @encoding,
          bom: @bom,
          trim_bom: @trim_bom
        ]

        stream_opts =
          case Keyword.fetch(opts, :max_buffer_size) do
            {:ok, max} -> Keyword.put(stream_opts, :max_buffer_size, max)
            :error -> stream_opts
          end

        result_stream = RustyCSV.Streaming.stream_enumerable(stream, stream_opts)

        do_stream_with_headers(result_stream, headers, opts)
      end
    end
  end

  defp quoted_stream_headers_clauses do
    quote do
      defp do_stream_with_headers(stream, false, opts) do
        if Keyword.get(opts, :skip_headers, true) do
          Stream.drop(stream, 1)
        else
          stream
        end
      end

      defp do_stream_with_headers(stream, true, _opts) do
        Stream.transform(stream, :no_header, fn
          row, :no_header ->
            {[], {:header, row, length(row)}}

          row, {:header, _keys, _num_keys} = state ->
            {[zip_to_map(state, row)], state}
        end)
      end

      defp do_stream_with_headers(stream, header_list, opts) when is_list(header_list) do
        num_keys = length(header_list)
        state = {:header, header_list, num_keys}
        base = if Keyword.get(opts, :skip_headers, true), do: Stream.drop(stream, 1), else: stream
        Stream.map(base, &zip_to_map(state, &1))
      end

      defp do_stream_with_headers(_stream, other, _opts) do
        raise ArgumentError,
              "invalid :headers option, expected false, true, or a list of keys, got: #{inspect(other)}"
      end
    end
  end

  defp quoted_zip_to_map do
    quote do
      defp zip_to_map({:header, keys, num_keys}, row) do
        row_len = length(row)

        padded =
          cond do
            row_len == num_keys -> row
            row_len < num_keys -> row ++ List.duplicate(nil, num_keys - row_len)
            true -> Enum.take(row, num_keys)
          end

        keys |> Enum.zip(padded) |> Map.new()
      end
    end
  end

  defp quoted_parse_enumerable_function do
    quote do
      @doc """
      Eagerly parses an enumerable of CSV data into a list of rows.
      """
      @impl RustyCSV
      @spec parse_enumerable(Enumerable.t(), RustyCSV.parse_options()) ::
              RustyCSV.rows() | [map()]
      def parse_enumerable(enumerable, opts \\ [])

      def parse_enumerable(enumerable, opts) when is_list(opts) do
        enumerable
        |> parse_stream(opts)
        |> Enum.to_list()
      end
    end
  end

  defp quoted_to_line_stream_function do
    quote do
      @doc """
      Converts a stream of arbitrary binary chunks into a line-oriented stream.
      """
      @impl RustyCSV
      @spec to_line_stream(Enumerable.t()) :: Enumerable.t()
      def to_line_stream(stream) do
        newline = :binary.compile_pattern(@newlines)

        stream
        |> Stream.chunk_while(
          "",
          fn element, acc ->
            to_try = acc <> element
            {elements, new_acc} = chunk_by_newline(to_try, newline, [], {0, byte_size(to_try)})
            {:cont, elements, new_acc}
          end,
          fn
            "" -> {:cont, []}
            acc -> {:cont, [acc], []}
          end
        )
        |> Stream.concat()
      end

      defp chunk_by_newline(_string, _newline, elements, {_offset, 0}) do
        {Enum.reverse(elements), ""}
      end

      defp chunk_by_newline(string, newline, elements, {offset, length}) do
        case :binary.match(string, newline, scope: {offset, length}) do
          {newline_offset, newline_length} ->
            difference = newline_length + newline_offset - offset
            element = binary_part(string, offset, difference)

            chunk_by_newline(
              string,
              newline,
              [element | elements],
              {newline_offset + newline_length, length - difference}
            )

          :nomatch ->
            {Enum.reverse(elements), binary_part(string, offset, length)}
        end
      end
    end
  end

  defp quoted_dumping_functions do
    [
      quoted_dump_to_iodata_function(),
      quoted_encode_rows_nif_helpers(),
      quoted_dump_retry_helpers(),
      quoted_dump_to_stream_function()
    ]
  end

  defp quoted_dump_to_iodata_function do
    quote do
      @doc """
      Converts an enumerable of rows to iodata in CSV format.

      Returns iodata. For the default encoder without a BOM this is a single
      flat binary. `:parallel` encoding and BOM-enabled modules may return
      list-shaped iodata.

      ## Options

        * `:strategy` - Encoding strategy. By default, uses a single-threaded
          SIMD-accelerated encoder. Pass `strategy: :parallel` for multi-threaded
          encoding via rayon, which is faster for quoting-heavy data.

      ## Examples

          # Default encoder (best for most data)
          #{inspect(__MODULE__)}.dump_to_iodata(rows)

          # Parallel encoder (best for quoting-heavy data)
          #{inspect(__MODULE__)}.dump_to_iodata(rows, strategy: :parallel)

      """
      @impl RustyCSV
      @spec dump_to_iodata(Enumerable.t(), RustyCSV.dump_options()) :: iodata()
      def dump_to_iodata(enumerable, opts \\ []) do
        rows = if is_list(enumerable), do: enumerable, else: Enum.to_list(enumerable)
        strategy = Keyword.get(opts, :strategy)

        result =
          rows
          |> encode_rows_nif(strategy)
          |> retry_with_coerced_fields(rows, strategy)

        if @dump_bom do
          [@bom, result]
        else
          result
        end
      end
    end
  end

  defp quoted_encode_rows_nif_helpers do
    quote do
      defp encode_rows_nif(rows, :parallel) do
        RustyCSV.Native.encode_string_parallel(
          rows,
          @separator_binaries,
          @escape_binary,
          @line_separator,
          @formula_nif_config,
          @encoding,
          @reserved_binaries
        )
      end

      defp encode_rows_nif(rows, _strategy) do
        RustyCSV.Native.encode_string(
          rows,
          @separator_binaries,
          @escape_binary,
          @line_separator,
          @formula_nif_config,
          @encoding,
          @reserved_binaries
        )
      end
    end
  end

  defp quoted_dump_retry_helpers do
    quote do
      defp retry_with_coerced_fields({:error, :non_binary_field}, rows, strategy) do
        rows
        |> coerce_fields_to_binary()
        |> encode_rows_nif(strategy)
      end

      defp retry_with_coerced_fields(result, _rows, _strategy), do: result

      defp coerce_fields_to_binary(rows) do
        Enum.map(rows, &coerce_row_fields_to_binary/1)
      end

      defp coerce_row_fields_to_binary(row) do
        Enum.map(row, fn
          field when is_binary(field) -> field
          field -> to_string(field)
        end)
      end
    end
  end

  defp quoted_dump_to_stream_function do
    quote do
      @doc """
      Lazily converts an enumerable of rows to a stream of iodata.
      """
      @impl RustyCSV
      @spec dump_to_stream(Enumerable.t()) :: Enumerable.t()
      def dump_to_stream(enumerable) do
        Stream.map(enumerable, &encode_single_row_nif/1)
      end

      defp encode_single_row_nif(row) do
        case encode_rows_nif([row], nil) do
          {:error, :non_binary_field} ->
            encode_rows_nif([coerce_row_fields_to_binary(row)], nil)

          result ->
            result
        end
      end
    end
  end
end

# ==========================================================================
# Pre-defined Parsers
# ==========================================================================

RustyCSV.define(RustyCSV.RFC4180,
  separator: ",",
  escape: "\"",
  line_separator: "\r\n",
  newlines: ["\r\n", "\n"],
  strategy: :simd,
  moduledoc: ~S"""
  A CSV parser/dumper following RFC 4180 conventions.

  This module uses comma (`,`) as the field separator and double-quote (`"`)
  as the escape character. It recognizes both CRLF and LF line endings.

  This is a drop-in replacement for `NimbleCSV.RFC4180`.

  ## Quick Start

      alias RustyCSV.RFC4180, as: CSV

      # Parse CSV (skips headers by default)
      CSV.parse_string("name,age\njohn,27\n")
      #=> [["john", "27"]]

      # Include headers
      CSV.parse_string("name,age\njohn,27\n", skip_headers: false)
      #=> [["name", "age"], ["john", "27"]]

      # Use parallel parsing for large files
      CSV.parse_string(large_csv, strategy: :parallel)

      # Stream large files with bounded memory
      "huge.csv"
      |> File.stream!()
      |> CSV.parse_stream()
      |> Enum.each(&process/1)

  ## Dumping

      CSV.dump_to_iodata([["name", "age"], ["john", "27"]])
      |> IO.iodata_to_binary()
      #=> "name,age\njohn,27\n"

  ## Configuration

  This module was defined with:

      RustyCSV.define(RustyCSV.RFC4180,
        separator: ",",
        escape: "\"",
        line_separator: "\n",
        newlines: ["\r\n", "\n"],
        strategy: :simd
      )

  To customize these options, define your own parser with `RustyCSV.define/2`.

  """
)

RustyCSV.define(RustyCSV.Spreadsheet,
  separator: "\t",
  escape: "\"",
  line_separator: "\n",
  newlines: ["\r\n", "\n"],
  encoding: {:utf16, :little},
  trim_bom: true,
  dump_bom: true,
  strategy: :simd,
  moduledoc: ~S"""
  A spreadsheet-compatible parser using UTF-16 Little Endian encoding.

  This module uses tab (`\t`) as the field separator and double-quote (`"`)
  as the escape character. It handles UTF-16 LE encoding with BOM, which is
  the format commonly used by spreadsheet applications like Microsoft Excel.

  This is a drop-in replacement for `NimbleCSV.Spreadsheet`.

  ## Quick Start

      alias RustyCSV.Spreadsheet

      # Parse UTF-16 LE data (with BOM)
      Spreadsheet.parse_string(utf16_data, skip_headers: false)
      #=> [["name", "age"], ["john", "27"]]

      # Dump to UTF-16 LE format (includes BOM)
      Spreadsheet.dump_to_iodata([["name", "age"], ["john", "27"]])
      |> IO.iodata_to_binary()

  ## Configuration

  This module was defined with:

      RustyCSV.define(RustyCSV.Spreadsheet,
        separator: "\t",
        escape: "\"",
        encoding: {:utf16, :little},
        trim_bom: true,
        dump_bom: true
      )

  """
)
