defmodule MultiByteSeparatorTest do
  @moduledoc """
  Tests for multi-byte separator support.
  """
  use ExUnit.Case

  # Define parsers with multi-byte separators
  RustyCSV.define(TestDoubleColon,
    separator: "::",
    escape: "\"",
    line_separator: "\n"
  )

  RustyCSV.define(TestDoublePipe,
    separator: "||",
    escape: "\"",
    line_separator: "\n"
  )

  RustyCSV.define(TestMixedMultiByte,
    separator: [",", "::"],
    escape: "\"",
    line_separator: "\n"
  )

  describe "double-colon separator" do
    test "parses basic data" do
      result = TestDoubleColon.parse_string("a::b::c\n1::2::3\n", skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "handles quoted fields containing separator" do
      result = TestDoubleColon.parse_string("\"a::b\"::c\n", skip_headers: false)
      assert result == [["a::b", "c"]]
    end

    test "handles empty fields" do
      result = TestDoubleColon.parse_string("a::::b\n", skip_headers: false)
      assert result == [["a", "", "b"]]
    end

    test "handles single colon in data (not a separator)" do
      result = TestDoubleColon.parse_string("a:b::c:d\n", skip_headers: false)
      assert result == [["a:b", "c:d"]]
    end
  end

  describe "double-pipe separator" do
    test "parses basic data" do
      result = TestDoublePipe.parse_string("a||b||c\n1||2||3\n", skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "handles single pipe in data (not a separator)" do
      result = TestDoublePipe.parse_string("a|b||c|d\n", skip_headers: false)
      assert result == [["a|b", "c|d"]]
    end
  end

  describe "mixed single and multi-byte separators" do
    test "parses with both comma and double-colon" do
      result = TestMixedMultiByte.parse_string("a,b::c\n1::2,3\n", skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end
  end

  describe "strategy compatibility with multi-byte separator" do
    test "all strategies produce identical output" do
      csv = "a::b::c\n1::2::3\n\"x::y\"::z\n"

      basic = TestDoubleColon.parse_string(csv, strategy: :basic, skip_headers: false)
      simd = TestDoubleColon.parse_string(csv, strategy: :simd, skip_headers: false)
      indexed = TestDoubleColon.parse_string(csv, strategy: :indexed, skip_headers: false)
      parallel = TestDoubleColon.parse_string(csv, strategy: :parallel, skip_headers: false)
      zero_copy = TestDoubleColon.parse_string(csv, strategy: :zero_copy, skip_headers: false)

      expected = [
        ["a", "b", "c"],
        ["1", "2", "3"],
        ["x::y", "z"]
      ]

      assert basic == expected
      assert simd == expected
      assert indexed == expected
      assert parallel == expected
      assert zero_copy == expected
    end
  end

  describe "streaming with multi-byte separator" do
    test "streams correctly" do
      chunks = ["a::b::c\n", "1::2::3\n"]

      result =
        chunks
        |> TestDoubleColon.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "streams with chunks splitting separator" do
      # Chunk boundary falls in the middle of "::"
      chunks = ["a:", ":b::c\n"]

      result =
        chunks
        |> TestDoubleColon.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["a", "b", "c"]]
    end
  end

  describe "round-trip with multi-byte separator" do
    test "parse then dump produces consistent output" do
      parsed = TestDoubleColon.parse_string("a::b::c\n1::2::3\n", skip_headers: false)
      dumped = TestDoubleColon.dump_to_iodata(parsed) |> IO.iodata_to_binary()
      # Dump uses the separator for output
      assert dumped == "a::b::c\n1::2::3\n"

      reparsed = TestDoubleColon.parse_string(dumped, skip_headers: false)
      assert reparsed == parsed
    end
  end

  describe "options/0" do
    test "returns multi-byte separator" do
      opts = TestDoubleColon.options()
      assert opts[:separator] == "::"
    end

    test "returns mixed separator list" do
      opts = TestMixedMultiByte.options()
      assert opts[:separator] == [",", "::"]
    end
  end
end
