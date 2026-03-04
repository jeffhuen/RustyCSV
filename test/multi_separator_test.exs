defmodule MultiSeparatorTest do
  @moduledoc """
  Tests for multi-separator support (NimbleCSV compatibility).

  NimbleCSV allows specifying multiple separator characters:
  - `separator: [",", ";"]` - any of these characters acts as a field delimiter
  - When dumping, only the first separator is used for output
  """
  use ExUnit.Case

  # Define a multi-separator parser (comma or semicolon)
  RustyCSV.define(TestMultiSep,
    separator: [",", ";"],
    escape: "\"",
    line_separator: "\n"
  )

  # Define a parser with three separators (comma, semicolon, tab)
  RustyCSV.define(TestTripleSep,
    separator: [",", ";", "\t"],
    escape: "\"",
    line_separator: "\n"
  )

  # Define edge case parsers
  RustyCSV.define(TestSingleSep, separator: ",", escape: "\"")
  RustyCSV.define(TestSingleList, separator: [","], escape: "\"")
  RustyCSV.define(TestMultiByteSep, separator: [",", "::"], escape: "\"")
  RustyCSV.define(TestIntSep, separator: 44, escape: "\"")

  describe "multi-separator parsing" do
    test "parses comma-separated values" do
      result = TestMultiSep.parse_string("a,b,c\n1,2,3\n", skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "parses semicolon-separated values" do
      result = TestMultiSep.parse_string("a;b;c\n1;2;3\n", skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "parses mixed separators in same file" do
      result = TestMultiSep.parse_string("a,b;c\n1;2,3\n", skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "parses mixed separators in same row" do
      result = TestMultiSep.parse_string("a,b;c,d;e\n", skip_headers: false)
      assert result == [["a", "b", "c", "d", "e"]]
    end

    test "handles quoted fields containing separators" do
      # Comma inside quotes should not split
      result = TestMultiSep.parse_string("a,\"b,c\";d\n", skip_headers: false)
      assert result == [["a", "b,c", "d"]]

      # Semicolon inside quotes should not split
      result = TestMultiSep.parse_string("a;\"b;c\",d\n", skip_headers: false)
      assert result == [["a", "b;c", "d"]]
    end

    test "handles empty fields with multi-separator" do
      result = TestMultiSep.parse_string("a,,b;;c\n", skip_headers: false)
      assert result == [["a", "", "b", "", "c"]]
    end

    test "works with three separators" do
      result = TestTripleSep.parse_string("a,b;c\td\n", skip_headers: false)
      assert result == [["a", "b", "c", "d"]]
    end

    test "handles tab separator in triple-sep parser" do
      result = TestTripleSep.parse_string("a\tb\tc\n", skip_headers: false)
      assert result == [["a", "b", "c"]]
    end
  end

  describe "multi-separator dumping" do
    test "uses only the first separator for output" do
      # When dumping, only the first separator (comma) should be used
      result =
        TestMultiSep.dump_to_iodata([["a", "b", "c"], ["1", "2", "3"]])
        |> IO.iodata_to_binary()

      assert result == "a,b,c\n1,2,3\n"
    end

    test "escapes all separators in fields when dumping" do
      # Both comma and semicolon should be escaped since they're both separators
      result =
        TestMultiSep.dump_to_iodata([["a;b", "c,d"]])
        |> IO.iodata_to_binary()

      assert result == "\"a;b\",\"c,d\"\n"
    end
  end

  describe "round-trip with multi-separator" do
    test "parsing then dumping produces consistent output" do
      # Parse input with mixed separators
      parsed = TestMultiSep.parse_string("a,b;c\n1;2,3\n", skip_headers: false)

      # Dump should use only comma
      dumped = TestMultiSep.dump_to_iodata(parsed) |> IO.iodata_to_binary()
      assert dumped == "a,b,c\n1,2,3\n"

      # Re-parsing dumped output should give same result
      reparsed = TestMultiSep.parse_string(dumped, skip_headers: false)
      assert reparsed == parsed
    end
  end

  describe "strategy compatibility with multi-separator" do
    test "all strategies produce identical output" do
      csv = "a,b;c\n1;2,3\n\"x;y\",\"z,w\";q\n"

      basic = TestMultiSep.parse_string(csv, strategy: :basic, skip_headers: false)
      simd = TestMultiSep.parse_string(csv, strategy: :simd, skip_headers: false)
      indexed = TestMultiSep.parse_string(csv, strategy: :indexed, skip_headers: false)
      parallel = TestMultiSep.parse_string(csv, strategy: :parallel, skip_headers: false)
      zero_copy = TestMultiSep.parse_string(csv, strategy: :zero_copy, skip_headers: false)

      expected = [
        ["a", "b", "c"],
        ["1", "2", "3"],
        ["x;y", "z,w", "q"]
      ]

      assert basic == expected
      assert simd == expected
      assert indexed == expected
      assert parallel == expected
      assert zero_copy == expected
    end
  end

  describe "streaming with multi-separator" do
    test "streams with multi-separator correctly" do
      chunks = ["a,b;c\n", "1;2,3\n"]

      result =
        chunks
        |> TestMultiSep.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end
  end

  describe "options/0 returns original separator format" do
    test "returns list for multi-separator" do
      opts = TestMultiSep.options()
      assert opts[:separator] == [",", ";"]
    end
  end

  describe "edge cases" do
    test "single separator string still works" do
      # This should behave exactly like a regular single-separator parser
      result = TestSingleSep.parse_string("a,b,c\n", skip_headers: false)
      assert result == [["a", "b", "c"]]
    end

    test "separator list with single element works" do
      result = TestSingleList.parse_string("a,b,c\n", skip_headers: false)
      assert result == [["a", "b", "c"]]
    end

    test "raises on empty separator list" do
      assert_raise ArgumentError, ~r/cannot be empty/, fn ->
        RustyCSV.define(TestEmpty, separator: [], escape: "\"")
      end
    end

    test "multi-byte separator in list is now allowed" do
      result = TestMultiByteSep.parse_string("a,b::c\n", skip_headers: false)
      assert result == [["a", "b", "c"]]
    end

    test "accepts integer codepoint as separator" do
      result = TestIntSep.parse_string("a,b\n", skip_headers: false)
      assert result == [["a", "b"]]
    end
  end
end
