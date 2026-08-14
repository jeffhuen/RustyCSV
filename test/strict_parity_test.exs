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

  test "errors report a byte position without exposing CSV contents" do
    error =
      assert_raise ParseError, fn ->
        CSV.parse_string("private-value\"TOP_SECRET,2\n", skip_headers: false)
      end

    assert error.message =~ "byte 13"
    refute error.message =~ "private-value"
    refute error.message =~ "TOP_SECRET"
  end

  test "multi-byte separators, escapes, and custom newlines are strict" do
    assert_raise ParseError, fn ->
      MultiByteSeparator.parse_string("x\"y::2\n", skip_headers: false)
    end

    assert_raise ParseError, fn ->
      MultiByteEscape.parse_string("x$$y,2\n", skip_headers: false)
    end

    assert_raise ParseError, fn ->
      CustomNewline.parse_string("\"x\"junk,2|", skip_headers: false)
    end
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
