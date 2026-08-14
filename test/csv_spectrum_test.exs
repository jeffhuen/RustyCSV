defmodule CsvSpectrumTest do
  @moduledoc """
  Tests RustyCSV against the csv-spectrum acid test suite.

  csv-spectrum is an industry-standard test suite for CSV parsers.
  See: https://github.com/max-mapper/csv-spectrum

  These tests verify RFC 4180 compliance and edge case handling.
  """
  use ExUnit.Case

  alias RustyCSV.RFC4180, as: CSV
  alias RustyCSV.TestStrategyMatrix

  @fixtures_path "test/fixtures/csv-spectrum"

  # Helper to load CSV and expected JSON
  defp load_test(name) do
    csv_path = Path.join(@fixtures_path, "#{name}.csv")
    json_path = Path.join(@fixtures_path, "#{name}.json")

    csv = File.read!(csv_path)
    expected = Jason.decode!(File.read!(json_path))

    {csv, expected}
  end

  # Convert our parsed output to the JSON format (list of maps with headers as keys)
  defp to_json_format([headers | rows]) do
    Enum.map(rows, fn row ->
      Enum.zip(headers, row) |> Map.new()
    end)
  end

  defp to_json_format([]), do: []

  describe "csv-spectrum acid test suite" do
    test "simple.csv - basic CSV parsing" do
      {csv, expected} = load_test("simple")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "simple_crlf.csv - CRLF line endings" do
      {csv, expected} = load_test("simple_crlf")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "comma_in_quotes.csv - commas inside quoted fields" do
      {csv, expected} = load_test("comma_in_quotes")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "escaped_quotes.csv - doubled quotes inside fields" do
      {csv, expected} = load_test("escaped_quotes")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "newlines.csv - LF inside quoted fields" do
      {csv, expected} = load_test("newlines")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "newlines_crlf.csv - CRLF inside quoted fields" do
      {csv, expected} = load_test("newlines_crlf")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "quotes_and_newlines.csv - combined edge cases" do
      {csv, expected} = load_test("quotes_and_newlines")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "empty.csv - empty file with headers only (LF)" do
      {csv, expected} = load_test("empty")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "empty_crlf.csv - empty file with headers only (CRLF)" do
      {csv, expected} = load_test("empty_crlf")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "utf8.csv - Unicode content" do
      {csv, expected} = load_test("utf8")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "json.csv - JSON-like content in fields" do
      {csv, expected} = load_test("json")
      result = CSV.parse_string(csv, skip_headers: false) |> to_json_format()
      assert result == expected
    end

    test "location_coordinates.csv - numeric/coordinate data" do
      {csv, expected} = load_test("location_coordinates")

      # The fixture contains unescaped quote characters in coordinate values.
      # Keep it as a compatibility check for the explicit lenient mode.
      result = CSV.parse_string(csv, skip_headers: false, strict: false) |> to_json_format()
      assert result == expected
    end
  end

  describe "csv-spectrum with all strategies" do
    @strategies TestStrategyMatrix.batch_strategy_atoms()

    for strategy <- @strategies do
      test "all tests pass with #{strategy} strategy" do
        strategy = unquote(strategy)

        for name <- ~w(simple comma_in_quotes escaped_quotes newlines quotes_and_newlines utf8) do
          {csv, expected} = load_test(name)

          result =
            CSV.parse_string(csv, skip_headers: false, strategy: strategy)
            |> to_json_format()

          assert result == expected,
                 "#{name}.csv failed with strategy #{strategy}"
        end
      end
    end
  end
end
