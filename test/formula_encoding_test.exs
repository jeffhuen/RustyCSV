defmodule RustyCSV.FormulaEncodingTest do
  @moduledoc """
  Public encoding-contract tests for formula neutralization and target encodings.

  Formula-neutralized output intentionally differs from NimbleCSV: RustyCSV
  quotes the complete prefixed field and converts it to the target encoding in
  one pass. Formula-disabled output remains byte-identical to NimbleCSV.
  """
  use ExUnit.Case, async: true

  formula_config = %{["=", "+", "-", "@"] => "'"}

  RustyCSV.define(RPlain, line_separator: "\n")
  NimbleCSV.define(NPlain, line_separator: "\n")

  RustyCSV.define(RFormula, line_separator: "\n", escape_formula: formula_config)

  RustyCSV.define(RUTF16,
    separator: "\t",
    encoding: {:utf16, :little},
    trim_bom: true,
    dump_bom: true
  )

  NimbleCSV.define(NUTF16,
    separator: "\t",
    encoding: {:utf16, :little},
    trim_bom: true,
    dump_bom: true
  )

  RustyCSV.define(RFormulaUTF16,
    separator: "\t",
    encoding: {:utf16, :little},
    trim_bom: true,
    dump_bom: true,
    escape_formula: %{"=" => "é"}
  )

  RustyCSV.define(RFormulaUTF32,
    separator: "\t",
    encoding: {:utf32, :little},
    trim_bom: true,
    dump_bom: true,
    escape_formula: %{"=" => "é"}
  )

  RustyCSV.define(RLatin1, encoding: :latin1)
  NimbleCSV.define(NLatin1, encoding: :latin1)

  RustyCSV.define(RFormulaLatin1,
    encoding: :latin1,
    escape_formula: %{"=" => "é"}
  )

  RustyCSV.define(RUnmappableFormulaLatin1,
    encoding: :latin1,
    escape_formula: %{"=" => "€"}
  )

  RustyCSV.define(RQuotedReplacement,
    line_separator: "\n",
    escape_formula: %{"=" => "\""}
  )

  RustyCSV.define(RSeparatorReplacement,
    line_separator: "\n",
    escape_formula: %{"=" => ","}
  )

  RustyCSV.define(RFormulaMultiByte,
    separator: "::",
    escape: "$=",
    line_separator: "\n",
    escape_formula: %{"=" => "$=prefix"}
  )

  test "formula-neutralized fields are prefixed and always quoted" do
    output = dump(RFormula, [["=1", "+2", "-3", "@4", "safe"]])

    assert output == ~s("'=1","'+2","'-3","'@4",safe\n)
  end

  test "formula-neutralized output parses to the exact prefixed values" do
    rows = [["=clean", "+has,comma", "-has\nnewline", "@has\"quote", "safe"]]
    output = dump(RFormula, rows)

    assert RFormula.parse_string(output, skip_headers: false) == [
             ["'=clean", "'+has,comma", "'-has\nnewline", "'@has\"quote", "safe"]
           ]
  end

  test "replacement and original quotes are escaped as one logical field" do
    output = dump(RQuotedReplacement, [["=\"value\""]])

    assert RQuotedReplacement.parse_string(output, skip_headers: false) == [
             ["\"=\"value\""]
           ]
  end

  test "a separator in the replacement remains inside one field" do
    output = dump(RSeparatorReplacement, [["=value"]])

    assert RSeparatorReplacement.parse_string(output, skip_headers: false) == [[",=value"]]
  end

  test "multi-byte separators and escapes preserve the prefixed logical field" do
    output = dump(RFormulaMultiByte, [["=value$=", "plain::field"]])

    assert RFormulaMultiByte.parse_string(output, skip_headers: false) == [
             ["$=prefix=value$=", "plain::field"]
           ]
  end

  test "parallel formula output is byte-identical to serial output" do
    rows =
      for i <- 1..200 do
        if rem(i, 3) == 0,
          do: ["=formula_#{i}", "+has,comma_#{i}"],
          else: ["safe_#{i}", "@value_#{i}"]
      end

    assert dump(RFormula, rows, strategy: :parallel) == dump(RFormula, rows)
  end

  test "parallel formula output remains identical after UTF-16 conversion" do
    rows = for i <- 1..200, do: ["=café_#{i}", "safe_#{i}"]

    assert dump(RFormulaUTF16, rows, strategy: :parallel) == dump(RFormulaUTF16, rows)
  end

  test "streaming formula output is byte-identical to batch output" do
    rows = [["=café", "safe"], ["=has\ttab", "other"]]
    streamed = rows |> RFormulaUTF16.dump_to_stream() |> Enum.to_list() |> IO.iodata_to_binary()

    assert streamed == dump(RFormulaUTF16, rows)
  end

  test "a non-ASCII formula replacement round-trips through Latin-1" do
    output = dump(RFormulaLatin1, [["=café"]])

    assert RFormulaLatin1.parse_string(output, skip_headers: false) == [["é=café"]]
  end

  test "a non-ASCII formula replacement round-trips through UTF-16" do
    output = dump(RFormulaUTF16, [["=café"]])

    assert RFormulaUTF16.parse_string(output, skip_headers: false) == [["é=café"]]
  end

  test "a non-ASCII formula replacement round-trips through UTF-32" do
    output = dump(RFormulaUTF32, [["=café"]])

    assert RFormulaUTF32.parse_string(output, skip_headers: false) == [["é=café"]]
  end

  test "an unmappable Latin-1 formula replacement raises" do
    assert_raise RuntimeError, fn -> RUnmappableFormulaLatin1.dump_to_iodata([["=value"]]) end
  end

  test "an empty formula replacement is rejected when defining a module" do
    assert_raise ArgumentError, fn ->
      RustyCSV.define(RustyCSV.FormulaEncodingTest.EmptyReplacement,
        escape_formula: %{"=" => ""}
      )
    end
  end

  test "the native boundary rejects an empty formula replacement" do
    assert_raise ArgumentError, fn ->
      RustyCSV.Native.encode_string(
        [["=value"]],
        [","],
        "\"",
        "\n",
        [{"=", ""}],
        :utf8,
        [",", "\"", "\n"]
      )
    end
  end

  test "formula-disabled UTF-8 output remains byte-identical to NimbleCSV" do
    rows = [["clean", "has,comma", "has\"quote", "has\nnewline"]]

    assert dump(RPlain, rows) == dump(NPlain, rows)
  end

  test "formula-disabled UTF-16 output remains byte-identical to NimbleCSV" do
    rows = [["café", "has\ttab", "has\"quote"]]

    assert dump(RUTF16, rows) == dump(NUTF16, rows)
  end

  test "formula-disabled Latin-1 output remains byte-identical to NimbleCSV" do
    rows = [["café", "has,comma", "über"]]

    assert dump(RLatin1, rows) == dump(NLatin1, rows)
  end

  defp dump(module, rows, opts \\ []) do
    output =
      case opts do
        [] -> module.dump_to_iodata(rows)
        _ -> module.dump_to_iodata(rows, opts)
      end

    IO.iodata_to_binary(output)
  end
end
