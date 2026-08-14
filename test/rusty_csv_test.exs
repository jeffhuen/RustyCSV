# Define test parsers before the test module
RustyCSV.define(TestTSV,
  separator: "\t",
  escape: "\"",
  line_separator: "\n"
)

RustyCSV.define(TestPSV,
  separator: "|",
  escape: "\"",
  line_separator: "\n"
)

RustyCSV.define(TestSSV,
  separator: ";",
  escape: "\"",
  line_separator: "\n"
)

defmodule RustyCSVTest do
  use ExUnit.Case

  alias RustyCSV.RFC4180, as: CSV

  describe "basic parsing" do
    test "parses simple CSV (skips headers by default)" do
      assert CSV.parse_string("a,b,c\n1,2,3\n") == [["1", "2", "3"]]
    end

    test "parses simple CSV with skip_headers: false" do
      assert CSV.parse_string("a,b,c\n1,2,3\n", skip_headers: false) ==
               [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "parses with explicit skip_headers: true" do
      assert CSV.parse_string("a,b,c\n1,2,3\n", skip_headers: true) == [["1", "2", "3"]]
    end
  end

  describe "strategies" do
    test "all strategies produce identical output" do
      csv = "a,b,c\n1,2,3\n"
      expected = [["a", "b", "c"], ["1", "2", "3"]]

      assert CSV.parse_string(csv, strategy: :basic, skip_headers: false) == expected
      assert CSV.parse_string(csv, strategy: :simd, skip_headers: false) == expected
      assert CSV.parse_string(csv, strategy: :indexed, skip_headers: false) == expected
      assert CSV.parse_string(csv, strategy: :parallel, skip_headers: false) == expected
    end
  end

  describe "options/0" do
    test "returns module configuration" do
      opts = CSV.options()
      assert opts[:separator] == ","
      assert opts[:escape] == "\""
      assert opts[:strategy] == :simd
    end
  end

  describe "configurable separator" do
    test "TSV parsing with tab separator" do
      tsv = "name\tage\njohn\t27\njane\t30\n"
      result = TestTSV.parse_string(tsv, skip_headers: false)
      assert result == [["name", "age"], ["john", "27"], ["jane", "30"]]
    end

    test "TSV with all strategies" do
      tsv = "a\tb\nc\td\n"
      expected = [["a", "b"], ["c", "d"]]

      assert TestTSV.parse_string(tsv, strategy: :basic, skip_headers: false) == expected
      assert TestTSV.parse_string(tsv, strategy: :simd, skip_headers: false) == expected
      assert TestTSV.parse_string(tsv, strategy: :indexed, skip_headers: false) == expected
      assert TestTSV.parse_string(tsv, strategy: :parallel, skip_headers: false) == expected
    end

    test "TSV with quoted fields containing tabs" do
      tsv = "a\t\"b\tc\"\td\n"
      result = TestTSV.parse_string(tsv, skip_headers: false)
      assert result == [["a", "b\tc", "d"]]
    end

    test "pipe-separated values" do
      psv = "a|b|c\n1|2|3\n"
      result = TestPSV.parse_string(psv, skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end

    test "semicolon-separated values" do
      ssv = "a;b;c\n1;2;3\n"
      result = TestSSV.parse_string(ssv, skip_headers: false)
      assert result == [["a", "b", "c"], ["1", "2", "3"]]
    end
  end
end
