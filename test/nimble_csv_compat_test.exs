defmodule NimbleCSVCompatTest do
  @moduledoc """
  Tests adapted from NimbleCSV to verify RustyCSV is a drop-in replacement.

  Sources:
  - https://github.com/dashbitco/nimble_csv/blob/v1.3.0/test/nimble_csv_test.exs
  - https://github.com/dashbitco/nimble_csv/blob/master/test/nimble_csv_test.exs

  RustyCSV adds strategy selection and redacts raw CSV content from parse errors.
  """
  use ExUnit.Case

  alias RustyCSV.RFC4180, as: CSV

  describe "parse_string/2" do
    test "with headers (skip_headers: true to match NimbleCSV default)" do
      assert CSV.parse_string(
               """
               name,last,year
               john,doe,1986
               """,
               skip_headers: true
             ) == [~w(john doe 1986)]
    end

    test "without skipping headers" do
      assert CSV.parse_string(
               """
               name,last,year
               john,doe,1986
               """,
               skip_headers: false
             ) == [~w(name last year), ~w(john doe 1986)]

      assert CSV.parse_string(
               """
               name,last,year
               john,doe,1986
               mary,jane,1985
               """,
               skip_headers: false
             ) == [~w(name last year), ~w(john doe 1986), ~w(mary jane 1985)]
    end

    test "without trailing new line" do
      assert CSV.parse_string(
               String.trim("""
               name,last,year
               john,doe,1986
               mary,jane,1985
               """),
               skip_headers: true
             ) == [~w(john doe 1986), ~w(mary jane 1985)]
    end

    test "with CRLF terminations" do
      assert CSV.parse_string("name,last,year\r\njohn,doe,1986\r\n", skip_headers: true) ==
               [~w(john doe 1986)]
    end

    test "with empty string" do
      assert CSV.parse_string("", skip_headers: false) == []

      assert CSV.parse_string(
               """
               name

               john

               """,
               skip_headers: false
             ) == [["name"], [""], ["john"], [""]]
    end

    test "with whitespace" do
      assert CSV.parse_string(
               """
               name,last,year
               \sjohn , doe , 1986\s
               """,
               skip_headers: true
             ) == [[" john ", " doe ", " 1986 "]]
    end

    test "with escape characters" do
      assert CSV.parse_string(
               """
               name,last,year
               john,"doe",1986
               """,
               skip_headers: true
             ) == [~w(john doe 1986)]

      assert CSV.parse_string(
               """
               name,last,year
               "john",doe,"1986"
               """,
               skip_headers: true
             ) == [~w(john doe 1986)]

      assert CSV.parse_string(
               """
               name,last,year
               "john","doe","1986"
               mary,"jane",1985
               """,
               skip_headers: true
             ) == [~w(john doe 1986), ~w(mary jane 1985)]

      assert CSV.parse_string(
               """
               name,year
               "doe, john",1986
               "jane, mary",1985
               """,
               skip_headers: true
             ) == [["doe, john", "1986"], ["jane, mary", "1985"]]
    end

    test "with escape characters spanning multiple lines" do
      assert CSV.parse_string(
               """
               name,last,comments
               john,"doe","this is a
               really long comment
               with multiple lines"
               mary,jane,short comment
               """,
               skip_headers: true
             ) == [
               ["john", "doe", "this is a\nreally long comment\nwith multiple lines"],
               ["mary", "jane", "short comment"]
             ]
    end

    test "with escaped escape characters (double quotes)" do
      assert CSV.parse_string(
               """
               name,last,comments
               john,"doe","with ""double-quotes"" inside"
               mary,jane,"with , inside"
               """,
               skip_headers: true
             ) == [
               ["john", "doe", "with \"double-quotes\" inside"],
               ["mary", "jane", "with , inside"]
             ]
    end
  end

  describe "parse_enumerable/2" do
    test "basic parsing" do
      assert CSV.parse_enumerable(
               [
                 "name,last,year\n",
                 "john,doe,1986\n"
               ],
               skip_headers: true
             ) == [~w(john doe 1986)]

      assert CSV.parse_enumerable(
               [
                 "name,last,year\n",
                 "john,doe,1986\n"
               ],
               skip_headers: false
             ) == [~w(name last year), ~w(john doe 1986)]
    end
  end

  describe "parse_stream/2" do
    test "basic streaming" do
      stream =
        [
          "name,last,year\n",
          "john,doe,1986\n"
        ]
        |> Stream.map(&String.upcase/1)

      assert CSV.parse_stream(stream, skip_headers: true) |> Enum.to_list() == [~w(JOHN DOE 1986)]

      stream =
        [
          "name,last,year\n",
          "john,doe,1986\n"
        ]
        |> Stream.map(&String.upcase/1)

      assert CSV.parse_stream(stream, skip_headers: false) |> Enum.to_list() ==
               [~w(NAME LAST YEAR), ~w(JOHN DOE 1986)]
    end
  end

  describe "dump_to_iodata/1" do
    test "empty dumping returns an empty list" do
      assert CSV.dump_to_iodata([]) == []
    end

    test "basic dumping" do
      # RFC 4180 specifies CRLF line endings
      result = CSV.dump_to_iodata([["name", "age"], ["john", 27]])

      assert [[first_row], [second_row]] = result
      assert is_binary(first_row)
      assert is_binary(second_row)

      assert IO.iodata_to_binary(result) ==
               "name,age\r\njohn,27\r\n"

      parallel =
        CSV.dump_to_iodata([["name", "age"], ["john", 27]], strategy: :parallel)

      assert [[first_parallel_row], [second_parallel_row]] = parallel
      assert is_binary(first_parallel_row)
      assert is_binary(second_parallel_row)
      assert IO.iodata_to_binary(parallel) == IO.iodata_to_binary(result)
    end

    test "dumping with newlines in fields" do
      assert IO.iodata_to_binary(CSV.dump_to_iodata([["name", "age"], ["john\ndoe", 27]])) ==
               "name,age\r\n\"john\ndoe\",27\r\n"
    end

    test "dumping with quotes in fields" do
      assert IO.iodata_to_binary(CSV.dump_to_iodata([["name", "age"], ["john \"nick\" doe", 27]])) ==
               "name,age\r\n\"john \"\"nick\"\" doe\",27\r\n"
    end

    test "dumping with commas in fields" do
      assert IO.iodata_to_binary(CSV.dump_to_iodata([["name", "age"], ["doe, john", 27]])) ==
               "name,age\r\n\"doe, john\",27\r\n"
    end
  end

  describe "dump_to_stream/1" do
    test "basic streaming dump" do
      result = Enum.to_list(CSV.dump_to_stream([["name", "age"], ["john", 27]]))

      assert Enum.all?(result, &is_list/1)

      assert IO.iodata_to_binary(result) ==
               "name,age\r\njohn,27\r\n"
    end

    test "streaming dump with special characters" do
      assert IO.iodata_to_binary(
               Enum.to_list(CSV.dump_to_stream([["name", "age"], ["john \"nick\" doe", 27]]))
             ) == "name,age\r\n\"john \"\"nick\"\" doe\",27\r\n"
    end
  end

  describe "to_line_stream/1" do
    test "converts arbitrary chunks to lines" do
      stream = [
        "name,last,year\n",
        "john,doe,1986\n",
        "jane,",
        "doe,1987\n",
        "james,doe,1992\nryan,doe",
        ",1893"
      ]

      assert [
               "name,last,year\n",
               "john,doe,1986\n",
               "jane,doe,1987\n",
               "james,doe,1992\n",
               "ryan,doe,1893"
             ] = CSV.to_line_stream(stream) |> Enum.into([])
    end
  end

  describe "round-trip" do
    test "parse then dump produces equivalent output" do
      original = [["name", "age"], ["john", "27"], ["mary", "32"]]
      dumped = CSV.dump_to_iodata(original)
      parsed = CSV.parse_string(IO.iodata_to_binary(dumped), skip_headers: false)
      assert parsed == original
    end

    test "round-trip with special characters" do
      original = [
        ["name", "comment"],
        ["john", "hello, world"],
        ["mary", "say \"hi\""],
        ["jane", "line1\nline2"]
      ]

      dumped = CSV.dump_to_iodata(original)
      parsed = CSV.parse_string(IO.iodata_to_binary(dumped), skip_headers: false)
      assert parsed == original
    end
  end

  describe "strategy selection" do
    test "all strategies produce identical output" do
      csv = """
      name,age,comment
      john,27,"hello, world"
      mary,32,"say ""hi\"""
      jane,28,"multi
      line"
      """

      basic = CSV.parse_string(csv, strategy: :basic, skip_headers: false)
      simd = CSV.parse_string(csv, strategy: :simd, skip_headers: false)
      indexed = CSV.parse_string(csv, strategy: :indexed, skip_headers: false)
      parallel = CSV.parse_string(csv, strategy: :parallel, skip_headers: false)

      assert basic == simd
      assert simd == indexed
      assert indexed == parallel
    end
  end

  describe "edge cases" do
    test "single field" do
      assert CSV.parse_string("a\n", skip_headers: false) == [["a"]]
    end

    test "empty fields" do
      assert CSV.parse_string("a,,b\n", skip_headers: false) == [["a", "", "b"]]
      assert CSV.parse_string(",a,\n", skip_headers: false) == [["", "a", ""]]
    end

    test "quoted empty field" do
      assert CSV.parse_string("\"\",a\n", skip_headers: false) == [["", "a"]]
    end

    test "only whitespace in quoted field" do
      assert CSV.parse_string("\" \",a\n", skip_headers: false) == [[" ", "a"]]
    end

    test "mixed line endings" do
      assert CSV.parse_string("a,b\nc,d\r\ne,f\n", skip_headers: false) ==
               [["a", "b"], ["c", "d"], ["e", "f"]]
    end
  end
end
