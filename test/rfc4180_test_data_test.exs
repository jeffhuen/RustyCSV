defmodule RFC4180TestDataTest do
  @moduledoc """
  Tests RustyCSV against the csv-test-data RFC 4180 test suite.

  csv-test-data provides comprehensive RFC 4180 compliance tests including
  both valid and invalid CSV cases.
  See: https://github.com/sineemore/csv-test-data

  These tests verify strict RFC 4180 compliance.
  """
  use ExUnit.Case

  alias RustyCSV.RFC4180, as: CSV
  alias RustyCSV.TestStrategyMatrix

  @fixtures_path "test/fixtures/csv-test-data"

  # Helper to load CSV and expected JSON
  defp load_test(name) do
    csv_path = Path.join(@fixtures_path, "#{name}.csv")
    json_path = Path.join(@fixtures_path, "#{name}.json")

    csv = File.read!(csv_path)

    expected =
      if File.exists?(json_path) do
        :json.decode(File.read!(json_path))
      else
        nil
      end

    {csv, expected}
  end

  describe "RFC 4180 valid cases - no headers" do
    test "simple-lf.csv - basic with LF endings" do
      {csv, expected} = load_test("simple-lf")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "simple-crlf.csv - basic with CRLF endings" do
      {csv, expected} = load_test("simple-crlf")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "quotes-with-comma.csv - commas in quoted fields" do
      {csv, expected} = load_test("quotes-with-comma")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "quotes-with-escaped-quote.csv - escaped quotes" do
      {csv, expected} = load_test("quotes-with-escaped-quote")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "quotes-with-newline.csv - newlines in quoted fields" do
      {csv, expected} = load_test("quotes-with-newline")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "quotes-with-space.csv - spaces in quoted fields" do
      {csv, expected} = load_test("quotes-with-space")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "quotes-empty.csv - empty quoted fields" do
      {csv, expected} = load_test("quotes-empty")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "empty-field.csv - empty unquoted fields" do
      {csv, expected} = load_test("empty-field")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "one-column.csv - single column" do
      {csv, expected} = load_test("one-column")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "empty-one-column.csv - single empty column" do
      {csv, expected} = load_test("empty-one-column")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "leading-space.csv - leading spaces preserved" do
      {csv, expected} = load_test("leading-space")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "trailing-space.csv - trailing spaces preserved" do
      {csv, expected} = load_test("trailing-space")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "trailing-newline.csv - file ends with newline" do
      {csv, expected} = load_test("trailing-newline")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "trailing-newline-one-field.csv - single field with trailing newline" do
      {csv, expected} = load_test("trailing-newline-one-field")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "utf8.csv - UTF-8 encoded content" do
      {csv, expected} = load_test("utf8")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end

    test "all-empty.csv - all empty fields" do
      {csv, expected} = load_test("all-empty")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == expected
    end
  end

  describe "RFC 4180 valid cases - with headers" do
    test "header-simple.csv - basic with header row" do
      {csv, _expected} = load_test("header-simple")
      # Parse with headers
      [headers | rows] = CSV.parse_string(csv, skip_headers: false)
      assert headers == ["foo", "bar", "baz"]
      assert [_row] = rows
    end

    test "header-no-rows.csv - headers only, no data" do
      {csv, _expected} = load_test("header-no-rows")
      result = CSV.parse_string(csv, skip_headers: false)
      assert result == [["foo", "bar", "baz"]]

      # With skip_headers, should return empty
      result_skip = CSV.parse_string(csv, skip_headers: true)
      assert result_skip == []
    end
  end

  describe "all strategies produce consistent results" do
    @strategies TestStrategyMatrix.batch_strategy_atoms()
    @test_files ~w(simple-lf quotes-with-comma quotes-with-escaped-quote quotes-with-newline utf8)

    for strategy <- @strategies do
      test "#{strategy} strategy matches expected output" do
        strategy = unquote(strategy)

        for name <- @test_files do
          {csv, expected} = load_test(name)

          result = CSV.parse_string(csv, skip_headers: false, strategy: strategy)

          assert result == expected,
                 "#{name}.csv failed with strategy #{strategy}\n" <>
                   "Expected: #{inspect(expected)}\nGot: #{inspect(result)}"
        end
      end
    end
  end
end
