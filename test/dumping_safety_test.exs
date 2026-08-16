defmodule RustyCSV.DumpingSafetyTest do
  use ExUnit.Case, async: true

  alias RustyCSV.RFC4180, as: CSV

  test "dumping raises ArgumentError for malformed rows instead of coercing and retrying" do
    assert_raise ArgumentError, fn ->
      CSV.dump_to_iodata([:not_a_row])
    end

    assert_raise ArgumentError, fn ->
      CSV.dump_to_iodata([:not_a_row], strategy: :parallel)
    end
  end

  test "dumping coerces non-binary fields before default and parallel native encoding" do
    rows = [["name", :age], ["john", 27], [:jane, 30.5]]

    assert CSV.dump_to_iodata(rows) |> IO.iodata_to_binary() ==
             "name,age\r\njohn,27\r\njane,30.5\r\n"

    assert CSV.dump_to_iodata(rows, strategy: :parallel) |> IO.iodata_to_binary() ==
             "name,age\r\njohn,27\r\njane,30.5\r\n"
  end

  test "dumping rejects unknown options" do
    assert_raise ArgumentError, fn -> CSV.dump_to_iodata([], escape_formula: %{"=" => "'"}) end
  end

  test "dumping rejects unsupported strategy values" do
    assert_raise ArgumentError, fn -> CSV.dump_to_iodata([], strategy: :unknown) end
  end
end
