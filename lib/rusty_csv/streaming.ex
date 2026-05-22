defmodule RustyCSV.Streaming do
  @moduledoc """
  Streaming CSV parser for processing large files with bounded memory.

  This module provides a streaming interface to the Rust-based streaming
  parser (Strategy D). It reads data in chunks and yields complete rows
  as they become available.

  ## Memory Behavior

  The streaming parser maintains a small buffer for partial rows. Memory
  usage is bounded by:

    * `chunk_size` - bytes per IO read operation
    * `batch_size` - rows held before yielding
    * Maximum single row size in your data

  ## Usage

  For most use cases, use the high-level `parse_stream/2` function from
  your CSV module:

      alias RustyCSV.RFC4180, as: CSV

      "data.csv"
      |> File.stream!()
      |> CSV.parse_stream()
      |> Enum.each(&process_row/1)

  ## Direct Usage

  For more control, you can use this module directly:

      # Stream a file row by row
      RustyCSV.Streaming.stream_file("data.csv")
      |> Enum.each(&process_row/1)

      # Stream with custom chunk size
      RustyCSV.Streaming.stream_file("data.csv", chunk_size: 1024 * 1024)
      |> Enum.to_list()

      # Stream from an already-open device
      File.open!("data.csv", [:read, :binary], fn device ->
        RustyCSV.Streaming.stream_device(device)
        |> Enum.each(&IO.inspect/1)
      end)

  ## Encoding Support

  The streaming functions support character encoding conversion via the
  `:encoding` option. When a non-UTF8 encoding is specified, the stream
  is automatically converted to UTF-8 before parsing, with proper handling
  of multi-byte character boundaries across chunks.

  ## Buffer Limit

  The streaming parser enforces a maximum buffer size (default 256 MB) to
  prevent unbounded memory growth when parsing data without newlines or with
  very long rows. If a `streaming_feed/2` call would push the buffer past
  this limit, it raises a `:buffer_overflow` exception.

  To adjust the limit, pass the `:max_buffer_size` option (in bytes) to any
  streaming function:

      RustyCSV.Streaming.stream_file("huge.csv", max_buffer_size: 512 * 1024 * 1024)

  Or from a defined parser:

      CSV.parse_stream(stream, max_buffer_size: 512 * 1024 * 1024)

  ## Concurrency

  Streaming parser references are safe to share across BEAM processes — the
  underlying Rust state is protected by a mutex. Concurrent access is serialized,
  so for maximum throughput use one parser per process.

  If a NIF panics while holding the mutex, the lock is poisoned and subsequent
  calls raise `:mutex_poisoned`. The parser should be discarded in this case.

  ## Scheduling

  `streaming_feed/2`, `streaming_next_rows/2`, and `streaming_finalize/1` run
  on dirty CPU schedulers to avoid blocking normal BEAM schedulers.

  ## Implementation Notes

  The streaming parser:

    * Handles quoted fields that span multiple chunks correctly
    * Preserves quote state across chunk boundaries
    * Handles multi-byte character boundaries for non-UTF8 encodings
    * Compacts internal buffer to prevent unbounded growth
    * Enforces a configurable maximum buffer size (default 256 MB)
    * Returns owned data (copies bytes) since input chunks are temporary

  """

  # ==========================================================================
  # Types
  # ==========================================================================

  @typedoc "A parsed row (list of field binaries)"
  @type row :: [binary()]

  @typedoc """
  Options for streaming functions.

  The `:separator` option accepts a binary (e.g., `<<?,>>` or `","`) or an
  integer byte (e.g., `?,` or `9`). When called from a module defined via
  `RustyCSV.define/2`, the separator is already normalized to a binary.

  The `:max_buffer_size` option sets the maximum internal buffer size in bytes.
  Defaults to `268_435_456` (256 MB). If a `streaming_feed/2` call would push
  the buffer past this limit, a `:buffer_overflow` exception is raised. Increase
  this if your data contains rows longer than 256 MB, or decrease it to fail
  faster on malformed input.
  """
  @type stream_options :: [
          chunk_size: pos_integer(),
          batch_size: pos_integer(),
          separator: binary() | non_neg_integer() | [binary()],
          escape: binary() | non_neg_integer(),
          newlines: :default | [binary()],
          encoding: RustyCSV.encoding(),
          bom: binary(),
          trim_bom: boolean(),
          max_buffer_size: pos_integer()
        ]

  # ==========================================================================
  # Constants
  # ==========================================================================

  @default_chunk_size 64 * 1024
  @default_batch_size 1000
  @min_buffer_size 64 * 1024

  # ==========================================================================
  # Public API
  # ==========================================================================

  @doc """
  Create a stream that reads a CSV file in chunks.

  Opens the file, creates a streaming parser, and returns a `Stream` that
  yields rows as they are parsed. The file is automatically closed when
  the stream is consumed or halted.

  ## Options

    * `:chunk_size` - Bytes to read per IO operation. Defaults to `65536` (64 KB).
      Larger chunks mean fewer IO operations but more memory per read.

    * `:batch_size` - Maximum rows to yield per stream iteration. Defaults to `1000`.
      Larger batches are more efficient but delay processing of early rows.

    * `:max_buffer_size` - Maximum internal buffer in bytes. Defaults to
      `268_435_456` (256 MB). Raises `:buffer_overflow` if exceeded.

  ## Returns

  A `Stream` that yields rows. Each row is a list of field binaries.

  ## Examples

      # Process a file row by row
      RustyCSV.Streaming.stream_file("data.csv")
      |> Enum.each(fn row ->
        IO.inspect(row)
      end)

      # Take first 5 rows
      RustyCSV.Streaming.stream_file("data.csv")
      |> Enum.take(5)

      # With custom options
      RustyCSV.Streaming.stream_file("huge.csv",
        chunk_size: 1024 * 1024,  # 1 MB chunks
        batch_size: 5000
      )
      |> Stream.map(&process_row/1)
      |> Stream.run()

  """
  @spec stream_file(Path.t(), stream_options()) :: Enumerable.t()
  def stream_file(path, opts \\ []) do
    chunk_size = Keyword.get(opts, :chunk_size, @default_chunk_size)
    batch_size = Keyword.get(opts, :batch_size, @default_batch_size)
    separator = Keyword.get(opts, :separator, <<?,>>)
    escape = Keyword.get(opts, :escape, ?")
    newlines = Keyword.get(opts, :newlines, :default)

    Stream.resource(
      fn -> init_file_stream(path, chunk_size, batch_size, separator, escape, newlines, opts) end,
      &next_rows_file/1,
      &cleanup_file_stream/1
    )
  end

  @doc """
  Create a stream from an enumerable (like `File.stream!/1`).

  This is used internally by `parse_stream/2` to handle line-oriented or
  chunk-oriented input from any enumerable source.

  ## Options

    * `:chunk_size` - Not used for enumerables (chunks come from source).

    * `:batch_size` - Maximum rows to yield per iteration. Defaults to `1000`.

    * `:encoding` - Character encoding of input. Defaults to `:utf8`.

    * `:bom` - BOM to strip if `:trim_bom` is true. Defaults to `""`.

    * `:trim_bom` - Whether to strip BOM from start. Defaults to `false`.

    * `:max_buffer_size` - Maximum internal buffer in bytes. Defaults to
      `268_435_456` (256 MB). Raises `:buffer_overflow` if exceeded.

  ## Examples

      # Parse from a list of chunks
      ["name,age\\n", "john,27\\n", "jane,30\\n"]
      |> RustyCSV.Streaming.stream_enumerable()
      |> Enum.to_list()
      #=> [["name", "age"], ["john", "27"], ["jane", "30"]]

      # Parse from File.stream!
      File.stream!("data.csv")
      |> RustyCSV.Streaming.stream_enumerable()
      |> Enum.each(&process/1)

  """
  @spec stream_enumerable(Enumerable.t(), stream_options()) :: Enumerable.t()
  def stream_enumerable(enumerable, opts \\ []) do
    batch_size = Keyword.get(opts, :batch_size, @default_batch_size)
    separator = Keyword.get(opts, :separator, <<?,>>)
    escape = Keyword.get(opts, :escape, ?")
    newlines = Keyword.get(opts, :newlines, :default)
    encoding = Keyword.get(opts, :encoding, :utf8)
    bom = Keyword.get(opts, :bom, "")
    trim_bom = Keyword.get(opts, :trim_bom, false)

    # Optimize File.Stream: switch line-mode to binary chunk mode to avoid
    # iterating 100K+ lines through Stream.transform (each line = 1 closure call).
    # Binary chunks (~64KB each) reduce iterations from ~100K to ~100.
    enumerable = optimize_file_stream(enumerable)

    # If encoding is not UTF-8, convert stream to UTF-8 first
    converted_enumerable =
      if encoding == :utf8 do
        if trim_bom and bom != "" do
          strip_bom_stream(enumerable, bom)
        else
          enumerable
        end
      else
        enumerable
        |> maybe_strip_bom_stream(trim_bom, bom)
        |> convert_stream_to_utf8(encoding)
      end

    parser = new_parser(separator, escape, newlines, opts)

    Stream.transform(
      converted_enumerable,
      fn -> {[], 0} end,
      fn chunk, {buf_chunks, buf_size} ->
        chunk_binary = if is_binary(chunk), do: chunk, else: to_string(chunk)
        new_buf_chunks = [chunk_binary | buf_chunks]
        new_buf_size = buf_size + byte_size(chunk_binary)

        if new_buf_size >= @min_buffer_size do
          combined = new_buf_chunks |> Enum.reverse() |> IO.iodata_to_binary()
          RustyCSV.Native.streaming_feed(parser, combined)
          rows = RustyCSV.Native.streaming_next_rows(parser, batch_size)
          {rows, {[], 0}}
        else
          {[], {new_buf_chunks, new_buf_size}}
        end
      end,
      fn {buf_chunks, _buf_size} ->
        unless buf_chunks == [] do
          combined = buf_chunks |> Enum.reverse() |> IO.iodata_to_binary()
          RustyCSV.Native.streaming_feed(parser, combined)
        end

        rows_from_buffer = RustyCSV.Native.streaming_next_rows(parser, 100_000)
        final_rows = RustyCSV.Native.streaming_finalize(parser)
        {rows_from_buffer ++ final_rows, {[], 0}}
      end,
      fn _acc -> :ok end
    )
  end

  # Strip BOM from first chunk of stream if present
  defp strip_bom_stream(enumerable, bom) do
    bom_size = byte_size(bom)

    Stream.transform(enumerable, true, fn
      chunk, true ->
        # First chunk - check and strip BOM
        if binary_part(chunk, 0, min(byte_size(chunk), bom_size)) == bom do
          {[binary_part(chunk, bom_size, byte_size(chunk) - bom_size)], false}
        else
          {[chunk], false}
        end

      chunk, false ->
        {[chunk], false}
    end)
  end

  defp maybe_strip_bom_stream(enumerable, true, bom) when bom != "",
    do: strip_bom_stream(enumerable, bom)

  defp maybe_strip_bom_stream(enumerable, _, _), do: enumerable

  # Convert stream from source encoding to UTF-8, handling multi-byte boundaries
  defp convert_stream_to_utf8(stream, encoding) do
    Stream.transform(
      stream,
      fn -> <<>> end,
      fn chunk, acc ->
        input = acc <> chunk

        case :unicode.characters_to_binary(input, encoding, :utf8) do
          binary when is_binary(binary) ->
            # Full conversion succeeded
            {[binary], <<>>}

          {:incomplete, converted, rest} ->
            # Partial conversion - rest contains incomplete multi-byte sequence
            {[converted], rest}

          {:error, converted, rest} ->
            raise RustyCSV.ParseError,
              message:
                "Invalid #{inspect(encoding)} sequence at byte #{byte_size(converted)}: " <>
                  "#{inspect(binary_part(rest, 0, min(byte_size(rest), 10)))}"
        end
      end,
      fn
        <<>> ->
          {[], <<>>}

        rest ->
          raise RustyCSV.ParseError,
            message:
              "Truncated #{inspect(encoding)} sequence at end of input: " <>
                "#{inspect(binary_part(rest, 0, min(byte_size(rest), 10)))}"
      end,
      fn _acc -> :ok end
    )
  end

  @doc """
  Stream from an already-open IO device.

  Useful when you want more control over file opening/closing, or when
  reading from a socket or other IO device.

  Note: This function does NOT close the device when done. The caller
  is responsible for closing it.

  ## Options

    * `:chunk_size` - Bytes to read per IO operation. Defaults to `65536`.

    * `:batch_size` - Maximum rows to yield per iteration. Defaults to `1000`.

    * `:max_buffer_size` - Maximum internal buffer in bytes. Defaults to
      `268_435_456` (256 MB). Raises `:buffer_overflow` if exceeded.

  ## Examples

      File.open!("data.csv", [:read, :binary], fn device ->
        RustyCSV.Streaming.stream_device(device)
        |> Enum.each(&IO.inspect/1)
      end)

  """
  @spec stream_device(IO.device(), stream_options()) :: Enumerable.t()
  def stream_device(device, opts \\ []) do
    chunk_size = Keyword.get(opts, :chunk_size, @default_chunk_size)
    batch_size = Keyword.get(opts, :batch_size, @default_batch_size)
    separator = Keyword.get(opts, :separator, <<?,>>)
    escape = Keyword.get(opts, :escape, ?")
    newlines = Keyword.get(opts, :newlines, :default)

    Stream.resource(
      fn ->
        init_device_stream(device, chunk_size, batch_size, separator, escape, newlines, opts)
      end,
      &next_rows_device/1,
      fn _state -> :ok end
    )
  end

  @doc """
  Parse binary chunks and return all rows.

  This is mainly useful for testing the streaming parser with in-memory data.
  For actual streaming use cases, use `stream_file/2` or `stream_enumerable/2`.

  ## Options

    * `:separator` - Field separator. Accepts an integer byte (e.g., `9` for tab),
      a binary (e.g., `"\\t"`, `"::"`), or a list of binaries (e.g., `[",", ";"]`).
      Defaults to `","`.
    * `:escape` - Escape/quote sequence. Accepts an integer byte (e.g., `34`) or
      a binary (e.g., `"\""`, `"$$"`). Defaults to `"` (34).
    * `:max_buffer_size` - Maximum internal buffer in bytes. Defaults to
      `268_435_456` (256 MB). Raises `:buffer_overflow` if exceeded.

  ## Examples

      RustyCSV.Streaming.parse_chunks(["a,b\\n1,", "2\\n3,4\\n"])
      #=> [["a", "b"], ["1", "2"], ["3", "4"]]

      # TSV parsing (integer or binary separator)
      RustyCSV.Streaming.parse_chunks(["a\\tb\\n1\\t2\\n"], separator: 9)
      RustyCSV.Streaming.parse_chunks(["a\\tb\\n1\\t2\\n"], separator: "\\t")
      #=> [["a", "b"], ["1", "2"]]

  """
  @spec parse_chunks([binary()], keyword()) :: [row()]
  def parse_chunks(chunks, opts \\ []) when is_list(chunks) do
    separator = Keyword.get(opts, :separator, <<?,>>)
    escape = Keyword.get(opts, :escape, ?")
    newlines = Keyword.get(opts, :newlines, :default)
    parser = new_parser(separator, escape, newlines, opts)

    # Feed all chunks
    Enum.each(chunks, fn chunk ->
      RustyCSV.Native.streaming_feed(parser, chunk)
    end)

    # Take all available rows
    {available, _buffer_size, _has_partial} = RustyCSV.Native.streaming_status(parser)
    rows = RustyCSV.Native.streaming_next_rows(parser, available + 1)

    # Finalize to get any remaining partial row
    final_rows = RustyCSV.Native.streaming_finalize(parser)

    rows ++ final_rows
  end

  # ==========================================================================
  # Parser Creation (Private)
  # ==========================================================================

  defp new_parser(separator, escape, newlines, opts) do
    parser = RustyCSV.Native.streaming_new_with_config(separator, escape, newlines)

    if max = Keyword.get(opts, :max_buffer_size) do
      RustyCSV.Native.streaming_set_max_buffer(parser, max)
    end

    parser
  end

  # ==========================================================================
  # File Streaming (Private)
  # ==========================================================================

  defp init_file_stream(path, chunk_size, batch_size, separator, escape, newlines, opts) do
    device = File.open!(path, [:read, :binary, :raw])
    parser = new_parser(separator, escape, newlines, opts)
    {:file, device, parser, chunk_size, batch_size}
  end

  defp next_rows_file({:file, device, parser, chunk_size, batch_size} = state) do
    {available, _buffer_size, _has_partial} = RustyCSV.Native.streaming_status(parser)

    if available > 0 do
      rows = RustyCSV.Native.streaming_next_rows(parser, batch_size)
      emit_rows(rows, state)
    else
      read_and_process_file(device, parser, chunk_size, batch_size, state)
    end
  end

  defp next_rows_file({:done, _device, _parser, _chunk_size, _batch_size} = state) do
    {:halt, state}
  end

  defp read_and_process_file(device, parser, chunk_size, batch_size, state) do
    case IO.binread(device, chunk_size) do
      :eof ->
        finalize_file_stream(parser, device, chunk_size, batch_size, state)

      {:error, reason} ->
        raise "Error reading CSV file: #{inspect(reason)}"

      chunk when is_binary(chunk) ->
        {_available, _buffer_size} = RustyCSV.Native.streaming_feed(parser, chunk)
        rows = RustyCSV.Native.streaming_next_rows(parser, batch_size)
        emit_rows(rows, state)
    end
  end

  defp finalize_file_stream(parser, device, chunk_size, batch_size, state) do
    case RustyCSV.Native.streaming_finalize(parser) do
      [] -> {:halt, state}
      rows -> {rows, {:done, device, parser, chunk_size, batch_size}}
    end
  end

  defp cleanup_file_stream({_, device, _parser, _chunk_size, _batch_size}) do
    File.close(device)
  end

  # ==========================================================================
  # Device Streaming (Private)
  # ==========================================================================

  defp init_device_stream(device, chunk_size, batch_size, separator, escape, newlines, opts) do
    parser = new_parser(separator, escape, newlines, opts)
    {:device, device, parser, chunk_size, batch_size}
  end

  defp next_rows_device({:device, device, parser, chunk_size, batch_size} = state) do
    {available, _buffer_size, _has_partial} = RustyCSV.Native.streaming_status(parser)

    if available > 0 do
      rows = RustyCSV.Native.streaming_next_rows(parser, batch_size)
      emit_rows(rows, state)
    else
      read_and_process_device(device, parser, chunk_size, batch_size, state)
    end
  end

  defp next_rows_device({:device_done, _device, _parser, _chunk_size, _batch_size} = state) do
    {:halt, state}
  end

  defp read_and_process_device(device, parser, chunk_size, batch_size, state) do
    case IO.binread(device, chunk_size) do
      :eof ->
        finalize_device_stream(parser, device, chunk_size, batch_size, state)

      {:error, reason} ->
        raise "Error reading from device: #{inspect(reason)}"

      chunk when is_binary(chunk) ->
        {_available, _buffer_size} = RustyCSV.Native.streaming_feed(parser, chunk)
        rows = RustyCSV.Native.streaming_next_rows(parser, batch_size)
        emit_rows(rows, state)
    end
  end

  defp finalize_device_stream(parser, device, chunk_size, batch_size, state) do
    case RustyCSV.Native.streaming_finalize(parser) do
      [] -> {:halt, state}
      rows -> {rows, {:device_done, device, parser, chunk_size, batch_size}}
    end
  end

  # ==========================================================================
  # Helpers (Private)
  # ==========================================================================

  defp emit_rows([], state), do: next_rows_file(state)
  defp emit_rows(rows, state), do: {rows, state}

  # Switch File.Stream from line mode to binary chunk mode.
  # Line mode emits ~100K elements for a typical CSV; binary chunk mode emits ~100.
  # This eliminates the dominant overhead: 100K Stream.transform closure calls.
  defp optimize_file_stream(%File.Stream{line_or_bytes: :line} = stream) do
    %{stream | line_or_bytes: @min_buffer_size}
  end

  defp optimize_file_stream(enumerable), do: enumerable
end
