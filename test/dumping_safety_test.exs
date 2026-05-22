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
end
