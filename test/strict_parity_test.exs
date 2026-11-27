defmodule RustyCSV.StrictParityTest do
  use ExUnit.Case

  alias RustyCSV.ParseError
  alias RustyCSV.RFC4180, as: CSV
  alias RustyCSV.TestStrategyMatrix

  RustyCSV.define(MultiByteSeparator,
    separator: "::",
    escape: "\"",
    line_separator: "\n"
  )

  RustyCSV.define(MultiByteEscape,
    separator: ",",
    escape: "$$",
    line_separator: "\n"
  )

  RustyCSV.define(CustomNewline,
    separator: ",",
    escape: "\"",
    newlines: ["|"]
  )

  @malformed [
    {"x\"y,2\n", "unexpected escape character"},
    {"\"x\"junk,2\n", "unexpected data after a closing escape"},
    {"\"x,2\n", "expected a closing escape before end of input"}
  ]

  test "all batch strategies reject the same malformed quoting categories" do
    for strategy <- TestStrategyMatrix.batch_strategy_atoms(), {input, category} <- @malformed do
      error =
        assert_raise ParseError, fn ->
          CSV.parse_string(input, skip_headers: false, strategy: strategy)
        end

      assert error.message =~ category
    end
  end

  test "strict false preserves lenient batch parsing" do
    for strategy <- TestStrategyMatrix.batch_strategy_atoms(), {input, _category} <- @malformed do
      assert is_list(
               CSV.parse_string(input,
                 skip_headers: false,
                 strategy: strategy,
                 strict: false
               )
             )
    end
  end

  test "map parsing is strict for serial and parallel strategies" do
    for strategy <- [:simd, :parallel] do
      assert_raise ParseError, fn ->
        CSV.parse_string("name,value\nx\"y,2\n", headers: true, strategy: strategy)
      end
    end
  end

  test "errors report line, column, byte offset, and reason without exposing CSV contents" do
    error =
      assert_raise ParseError, fn ->
        CSV.parse_string("private-value\"TOP_SECRET,2\n", skip_headers: false)
      end

    assert error.line == 1
    assert error.column == 14
    assert error.byte_offset == 13
    assert error.reason == :unexpected_quote
    assert error.message =~ "line 1, column 14"
    assert error.message =~ "byte 13"
    refute error.message =~ "private-value"
    refute error.message =~ "TOP_SECRET"
  end

  test "all batch strategies and chunked streaming agree on line, column, byte offset, and reason" do
    cases = [
      {"a,b\nx\"y,2\n", 2, 2, 5, :unexpected_quote},
      {"a,b\r\n\"x\"junk,2\r\n", 2, 4, 8, :trailing_garbage},
      {"a,b\n\"line1\nline2\"\n名前,x\"y\n", 4, 5, 26, :unexpected_quote},
      {"a,b\n\"x,2", 2, 5, 8, :unterminated_quote},
      {"a,b\n\"x,2\n", 3, 1, 9, :unterminated_quote}
    ]

    for {input, line, col, offset, reason} <- cases do
      for strategy <- TestStrategyMatrix.batch_strategy_atoms() do
        error =
          assert_raise ParseError, fn ->
            CSV.parse_string(input, skip_headers: false, strategy: strategy)
          end

        assert {error.line, error.column, error.byte_offset, error.reason} ==
                 {line, col, offset, reason},
               "strategy #{strategy} mismatch for #{inspect(input)}: #{inspect(error)}"
      end

      # Single-byte chunks exercise compaction and cross-chunk state tracking.
      chunks = for <<byte <- input>>, do: <<byte>>

      stream_error =
        assert_raise ParseError, fn ->
          chunks |> CSV.parse_stream(skip_headers: false, chunk_size: 4) |> Enum.to_list()
        end

      assert {stream_error.line, stream_error.column, stream_error.byte_offset,
              stream_error.reason} == {line, col, offset, reason},
             "streaming mismatch for #{inspect(input)}: #{inspect(stream_error)}"
    end
  end

  test "multi-byte separators, escapes, and custom newlines report matching line and column" do
    for strategy <- [:simd, :parallel] do
      sep_err =
        assert_raise ParseError, fn ->
          MultiByteSeparator.parse_string("a::b\nx\"y::2\n",
            skip_headers: false,
            strategy: strategy
          )
        end

      assert {sep_err.line, sep_err.column, sep_err.byte_offset, sep_err.reason} ==
               {2, 2, 6, :unexpected_quote}

      esc_err =
        assert_raise ParseError, fn ->
          MultiByteEscape.parse_string("a,b\nx$$y,2\n",
            skip_headers: false,
            strategy: strategy
          )
        end

      assert {esc_err.line, esc_err.column, esc_err.byte_offset, esc_err.reason} ==
               {2, 2, 5, :unexpected_quote}

      nl_err =
        assert_raise ParseError, fn ->
          CustomNewline.parse_string("a,b|\"x\"junk,2|",
            skip_headers: false,
            strategy: strategy
          )
        end

      assert {nl_err.line, nl_err.column, nl_err.byte_offset, nl_err.reason} ==
               {2, 4, 7, :trailing_garbage}
    end

    nl_stream_err =
      assert_raise ParseError, fn ->
        ["a,b|", "\"x\"", "junk,2|"]
        |> CustomNewline.parse_stream(skip_headers: false, chunk_size: 2)
        |> Enum.to_list()
      end

    assert {nl_stream_err.line, nl_stream_err.column, nl_stream_err.byte_offset,
            nl_stream_err.reason} == {2, 4, 7, :trailing_garbage}
  end

  test "streaming detects violations across chunks and at finalize" do
    assert_raise ParseError, fn ->
      RustyCSV.Streaming.parse_chunks(["x", "\"y,2\n"])
    end

    assert_raise ParseError, fn ->
      RustyCSV.Streaming.parse_chunks(["\"x\"", "junk,2\n"])
    end

    assert_raise ParseError, fn ->
      RustyCSV.Streaming.parse_chunks(["\"x", ",2"])
    end

    assert_raise ParseError, fn ->
      RustyCSV.Streaming.parse_chunks(["x\"", "y::2\n"], separator: "::")
    end

    assert_raise ParseError, fn ->
      RustyCSV.Streaming.parse_chunks(["x$", "$y,2\n"], escape: "$$")
    end

    assert_raise ParseError, fn ->
      RustyCSV.Streaming.parse_chunks(["\"x\"", "junk,2|"], newlines: ["|"])
    end
  end

  test "strict false preserves lenient streaming" do
    assert is_list(RustyCSV.Streaming.parse_chunks(["x", "\"y,2\n"], strict: false))

    assert is_list(
             RustyCSV.Streaming.parse_chunks(["x$", "$y,2\n"],
               escape: "$$",
               strict: false
             )
           )
  end
end
