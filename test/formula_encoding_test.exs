defmodule RustyCSV.FormulaEncodingTest do
  @moduledoc """
  Byte-for-byte parity tests between RustyCSV NIF and NimbleCSV for
  formula escaping, non-UTF-8 encoding, and the combination of both.

  Every test that says "matches NimbleCSV" asserts binary equality
  on the flattened iodata output. No pattern matching, no "close enough".
  """
  use ExUnit.Case, async: true

  # ── Module definitions ──────────────────────────────────────────────

  formula_config = %{["=", "+", "-", "@"] => "'"}

  # Plain UTF-8, no formula
  RustyCSV.define(RPlain, line_separator: "\n")
  NimbleCSV.define(NPlain, line_separator: "\n")

  # UTF-8 + formula
  RustyCSV.define(RFormula, line_separator: "\n", escape_formula: formula_config)
  NimbleCSV.define(NFormula, line_separator: "\n", escape_formula: formula_config)

  # UTF-16 LE, no formula
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

  # UTF-16 BE, no formula
  RustyCSV.define(RUTF16BE,
    separator: "\t",
    encoding: {:utf16, :big},
    trim_bom: true,
    dump_bom: true
  )

  NimbleCSV.define(NUTF16BE,
    separator: "\t",
    encoding: {:utf16, :big},
    trim_bom: true,
    dump_bom: true
  )

  # Latin-1, no formula
  RustyCSV.define(RLatin1, encoding: :latin1)
  NimbleCSV.define(NLatin1, encoding: :latin1)

  # UTF-16 LE + formula
  RustyCSV.define(RFormulaUTF16,
    separator: "\t",
    encoding: {:utf16, :little},
    trim_bom: true,
    dump_bom: true,
    escape_formula: formula_config
  )

  NimbleCSV.define(NFormulaUTF16,
    separator: "\t",
    encoding: {:utf16, :little},
    trim_bom: true,
    dump_bom: true,
    escape_formula: formula_config
  )

  # UTF-16 BE + formula
  RustyCSV.define(RFormulaUTF16BE,
    separator: "\t",
    encoding: {:utf16, :big},
    trim_bom: true,
    dump_bom: true,
    escape_formula: formula_config
  )

  NimbleCSV.define(NFormulaUTF16BE,
    separator: "\t",
    encoding: {:utf16, :big},
    trim_bom: true,
    dump_bom: true,
    escape_formula: formula_config
  )

  # Latin-1 + formula
  RustyCSV.define(RFormulaLatin1, encoding: :latin1, escape_formula: formula_config)
  NimbleCSV.define(NFormulaLatin1, encoding: :latin1, escape_formula: formula_config)

  # CRLF line separator (RFC4180)
  RustyCSV.define(RCRLF, line_separator: "\r\n", escape_formula: formula_config)
  NimbleCSV.define(NCRLF, line_separator: "\r\n", escape_formula: formula_config)

  # Helper: flatten + assert binary equality with diff context on failure
  # Options (like strategy: :parallel) are only passed to RustyCSV — NimbleCSV doesn't support them.
  defp assert_identical(rusty_mod, nimble_mod, rows, opts \\ []) do
    rusty =
      case opts do
        [] -> rusty_mod.dump_to_iodata(rows)
        _ -> rusty_mod.dump_to_iodata(rows, opts)
      end
      |> IO.iodata_to_binary()

    nimble = nimble_mod.dump_to_iodata(rows) |> IO.iodata_to_binary()

    if rusty != nimble do
      {byte_idx, rusty_byte, nimble_byte} = first_diff(rusty, nimble)

      flunk("""
      Byte mismatch at index #{byte_idx}:
        rusty  byte: #{rusty_byte} (#{inspect(<<rusty_byte>>)})
        nimble byte: #{nimble_byte} (#{inspect(<<nimble_byte>>)})
        rusty  size: #{byte_size(rusty)}
        nimble size: #{byte_size(nimble)}
        context (rusty):  #{inspect(context(rusty, byte_idx))}
        context (nimble): #{inspect(context(nimble, byte_idx))}
      """)
    end
  end

  defp first_diff(a, b) do
    a_bytes = :binary.bin_to_list(a)
    b_bytes = :binary.bin_to_list(b)

    max_len = max(length(a_bytes), length(b_bytes))
    a_padded = a_bytes ++ List.duplicate(nil, max_len - length(a_bytes))
    b_padded = b_bytes ++ List.duplicate(nil, max_len - length(b_bytes))

    Enum.zip(a_padded, b_padded)
    |> Enum.with_index()
    |> Enum.find(fn {{x, y}, _} -> x != y end)
    |> case do
      {{x, y}, idx} -> {idx, x || :eof, y || :eof}
      nil -> {0, 0, 0}
    end
  end

  defp context(bin, idx) do
    start = max(0, idx - 8)
    len = min(24, byte_size(bin) - start)
    :binary.bin_to_list(binary_part(bin, start, len))
  end

  # ==================================================================
  # 1. Plain UTF-8 — baseline (PostProcess::None)
  # ==================================================================

  describe "PostProcess::None — plain UTF-8" do
    test "clean fields" do
      assert_identical(RPlain, NPlain, [["abc", "123", "xyz"]])
    end

    test "field containing comma" do
      assert_identical(RPlain, NPlain, [["has,comma", "ok"]])
    end

    test "field containing escape char" do
      assert_identical(RPlain, NPlain, [["has\"quote", "ok"]])
    end

    test "field containing newline" do
      assert_identical(RPlain, NPlain, [["has\nnewline", "ok"]])
    end

    test "field containing CRLF" do
      assert_identical(RPlain, NPlain, [["has\r\ncrlf", "ok"]])
    end

    test "field that IS the escape char" do
      assert_identical(RPlain, NPlain, [["\"", "ok"]])
    end

    test "field with consecutive escape chars" do
      assert_identical(RPlain, NPlain, [["\"\"\"\"", "ok"]])
    end

    test "empty field" do
      assert_identical(RPlain, NPlain, [[""]])
    end

    test "all empty fields" do
      assert_identical(RPlain, NPlain, [["", "", "", ""]])
    end

    test "single field row" do
      assert_identical(RPlain, NPlain, [["only"]])
    end

    test "empty row (no fields)" do
      assert_identical(RPlain, NPlain, [[]])
    end

    test "multiple rows, mixed clean and dirty" do
      assert_identical(RPlain, NPlain, [
        ["clean", "also clean"],
        ["has,comma", "has\"quote"],
        ["", "has\nnewline"],
        ["normal", "end"]
      ])
    end

    test "wide row (50 fields)" do
      row = for i <- 1..50, do: "field_#{i}"
      assert_identical(RPlain, NPlain, [row])
    end

    test "parallel matches sequential" do
      rows = for i <- 1..500, do: ["id_#{i}", if(rem(i, 3) == 0, do: "has,comma", else: "clean")]
      assert_identical(RPlain, NPlain, rows, strategy: :parallel)
    end
  end

  # ==================================================================
  # 2. Formula only — UTF-8 (PostProcess::FormulaOnly)
  # ==================================================================

  describe "PostProcess::FormulaOnly — formula UTF-8" do
    test "field is exactly a trigger char: =" do
      assert_identical(RFormula, NFormula, [["="]])
    end

    test "field is exactly a trigger char: +" do
      assert_identical(RFormula, NFormula, [["+"]])
    end

    test "field is exactly a trigger char: -" do
      assert_identical(RFormula, NFormula, [["-"]])
    end

    test "field is exactly a trigger char: @" do
      assert_identical(RFormula, NFormula, [["@"]])
    end

    test "trigger + escape char forces quoting and doubling" do
      # = followed by " — field is dirty (contains escape char) AND formula-triggered
      assert_identical(RFormula, NFormula, [["=\""]])
    end

    test "trigger + separator forces quoting" do
      assert_identical(RFormula, NFormula, [["=,"]])
    end

    test "trigger + newline forces quoting" do
      assert_identical(RFormula, NFormula, [["=\n"]])
    end

    test "trigger + CRLF forces quoting" do
      assert_identical(RFormula, NFormula, [["=\r\n"]])
    end

    test "trigger field with many doubled escapes" do
      # ="x""y""z" — trigger + multiple quotes that need doubling
      assert_identical(RFormula, NFormula, [["=\"x\"\"y\"\"z\""]])
    end

    test "trigger field that is all escape chars" do
      assert_identical(RFormula, NFormula, [["=\"\"\"\""]])
    end

    test "non-trigger field is NOT prefixed" do
      assert_identical(RFormula, NFormula, [["hello", "world"]])
    end

    test "empty field is NOT prefixed" do
      assert_identical(RFormula, NFormula, [[""]])
    end

    test "all four triggers in one row, all clean" do
      assert_identical(RFormula, NFormula, [["=a", "+b", "-c", "@d"]])
    end

    test "all four triggers in one row, all dirty" do
      assert_identical(RFormula, NFormula, [["=a,b", "+c\"d", "-e\nf", "@g,h"]])
    end

    test "mixed trigger/non-trigger, clean/dirty" do
      assert_identical(RFormula, NFormula, [
        ["=clean", "normal", "+dirty,comma"],
        ["safe", "-ok", "@also\"dirty"],
        ["", "=", "no_trigger"]
      ])
    end

    test "field starting with trigger but containing ONLY trigger char repeated" do
      assert_identical(RFormula, NFormula, [["=====", "+++++", "-----", "@@@@@"]])
    end

    test "CRLF line separator with formula" do
      assert_identical(RCRLF, NCRLF, [
        ["=clean", "normal"],
        ["+dirty,comma", "safe"]
      ])
    end

    test "parallel matches NimbleCSV" do
      rows =
        for i <- 1..500 do
          cond do
            rem(i, 7) == 0 -> ["=has,comma_#{i}", "+clean_#{i}"]
            rem(i, 5) == 0 -> ["=clean_#{i}", "-also\"dirty_#{i}"]
            rem(i, 3) == 0 -> ["@mention_#{i}", "safe_#{i}"]
            true -> ["normal_#{i}", "plain_#{i}"]
          end
        end

      assert_identical(RFormula, NFormula, rows, strategy: :parallel)
    end
  end

  # ==================================================================
  # 3. Encoding only — no formula (PostProcess::EncodingOnly)
  # ==================================================================

  describe "PostProcess::EncodingOnly — UTF-16 LE" do
    test "clean ASCII fields" do
      assert_identical(RUTF16, NUTF16, [["abc", "123"]])
    end

    test "empty field" do
      assert_identical(RUTF16, NUTF16, [[""]])
    end

    test "field containing tab (separator) — needs quoting" do
      assert_identical(RUTF16, NUTF16, [["has\ttab", "ok"]])
    end

    test "field containing quote — needs quoting and doubling" do
      assert_identical(RUTF16, NUTF16, [["has\"quote", "ok"]])
    end

    test "field that IS the escape char" do
      assert_identical(RUTF16, NUTF16, [["\"", "ok"]])
    end

    test "field with consecutive escape chars" do
      assert_identical(RUTF16, NUTF16, [["\"\"\"\"", "ok"]])
    end

    test "field containing newline" do
      assert_identical(RUTF16, NUTF16, [["has\nnewline", "ok"]])
    end

    test "unicode: accented chars" do
      assert_identical(RUTF16, NUTF16, [["café", "über"]])
    end

    test "unicode: CJK" do
      assert_identical(RUTF16, NUTF16, [["日本語", "中文"]])
    end

    test "unicode: emoji (surrogate pairs in UTF-16)" do
      assert_identical(RUTF16, NUTF16, [["hello 🎉", "world 🚀"]])
    end

    test "unicode: emoji inside quoted field" do
      assert_identical(RUTF16, NUTF16, [["has\t🎉", "ok"]])
    end

    test "single field row" do
      assert_identical(RUTF16, NUTF16, [["only"]])
    end

    test "empty row" do
      assert_identical(RUTF16, NUTF16, [[]])
    end

    test "all empty fields" do
      assert_identical(RUTF16, NUTF16, [["", "", ""]])
    end

    test "multiple rows mixed clean/dirty" do
      assert_identical(RUTF16, NUTF16, [
        ["clean", "also clean"],
        ["has\ttab", "has\"quote"],
        ["", "has\nnewline"],
        ["日本語", "emoji 🎉"]
      ])
    end
  end

  describe "PostProcess::EncodingOnly — UTF-16 BE" do
    test "clean fields" do
      assert_identical(RUTF16BE, NUTF16BE, [["abc", "123"]])
    end

    test "dirty field with quoting" do
      assert_identical(RUTF16BE, NUTF16BE, [["has\ttab", "has\"quote"]])
    end

    test "unicode: emoji" do
      assert_identical(RUTF16BE, NUTF16BE, [["hello 🎉", "ok"]])
    end
  end

  describe "PostProcess::EncodingOnly — Latin-1" do
    test "ASCII fields" do
      assert_identical(RLatin1, NLatin1, [["hello", "world"]])
    end

    test "accented chars (Latin-1 range)" do
      assert_identical(RLatin1, NLatin1, [["café", "über"], ["naïve", "résumé"]])
    end

    test "dirty field with Latin-1 chars" do
      assert_identical(RLatin1, NLatin1, [["café,latte", "über\"cool"]])
    end

    test "field that is the escape char" do
      assert_identical(RLatin1, NLatin1, [["\"", "ok"]])
    end
  end

  # ==================================================================
  # 4. Formula + Encoding (PostProcess::Full)
  # ==================================================================

  describe "PostProcess::Full — formula + UTF-16 LE" do
    test "clean trigger field" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=test"]])
    end

    test "field is exactly a trigger char" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["="]])
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["+"]])
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["-"]])
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["@"]])
    end

    test "dirty trigger: trigger + separator" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=has\ttab"]])
    end

    test "dirty trigger: trigger + escape" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=\""]])
    end

    test "dirty trigger: trigger + newline" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=\n"]])
    end

    test "dirty trigger: trigger + many doubled escapes" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=\"x\"\"y\"\"z\""]])
    end

    test "non-trigger clean field" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["safe"]])
    end

    test "non-trigger dirty field" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["has\ttab"]])
    end

    test "empty field" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [[""]])
    end

    test "all four triggers clean" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=a", "+b", "-c", "@d"]])
    end

    test "all four triggers dirty" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [
        ["=a\tb", "+c\"d", "-e\nf", "@g\th"]
      ])
    end

    test "mixed triggers/non-triggers, clean/dirty, multiple rows" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [
        ["1", "-$42.50", "Alice", "Regular note", "2024-01-02"],
        ["2", "+15%", "Bob", "=HYPERLINK(\"evil\")", "2024-01-03"],
        ["3", "=SUM(A1:A10)", "Carol", "note 3", "2024-01-04"],
        ["4", "$123.45", "Dave", "note 4", "2024-01-05"],
        ["5", "@admin", "Eve", "note 5", "2024-01-06"]
      ])
    end

    test "unicode: accented chars with trigger" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=café", "über"]])
    end

    test "unicode: emoji with trigger" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=🎉", "+🚀"]])
    end

    test "unicode: CJK with trigger" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=日本語", "中文"]])
    end

    test "row where every field triggers formula" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["=a", "+b", "-c", "@d", "=e"]])
    end

    test "row where no field triggers formula" do
      assert_identical(RFormulaUTF16, NFormulaUTF16, [["safe", "also", "fine"]])
    end

    test "formula prefix byte structure: raw prefix, encoded field" do
      # Structural assertion: the formula prefix ' (0x27) must be a single raw
      # byte, NOT UTF-16 encoded. The field content must be UTF-16 LE encoded.
      rows = [["=test"]]
      rusty = RFormulaUTF16.dump_to_iodata(rows) |> IO.iodata_to_binary()
      nimble = NFormulaUTF16.dump_to_iodata(rows) |> IO.iodata_to_binary()
      assert rusty == nimble

      <<0xFF, 0xFE, rest::binary>> = rusty
      encoded_field = :unicode.characters_to_binary("=test", :utf8, {:utf16, :little})
      encoded_nl = :unicode.characters_to_binary("\n", :utf8, {:utf16, :little})
      # prefix is raw 0x27, not 0x27 0x00
      assert rest == <<0x27>> <> encoded_field <> encoded_nl
    end

    test "dirty formula prefix byte structure: encoded quote, raw prefix, encoded inner" do
      # For dirty fields: [encoded_esc, raw_prefix, encoded_inner, encoded_esc]
      rows = [["=has\ttab"]]
      rusty = RFormulaUTF16.dump_to_iodata(rows) |> IO.iodata_to_binary()
      nimble = NFormulaUTF16.dump_to_iodata(rows) |> IO.iodata_to_binary()
      assert rusty == nimble

      <<0xFF, 0xFE, rest::binary>> = rusty
      encoded_esc = :unicode.characters_to_binary("\"", :utf8, {:utf16, :little})
      encoded_inner = :unicode.characters_to_binary("=has\ttab", :utf8, {:utf16, :little})
      encoded_nl = :unicode.characters_to_binary("\n", :utf8, {:utf16, :little})
      assert rest == encoded_esc <> <<0x27>> <> encoded_inner <> encoded_esc <> encoded_nl
    end

    test "large deterministic dataset" do
      rows =
        for i <- 1..2000 do
          trigger =
            case rem(i, 4) do
              0 -> "="
              1 -> "+"
              2 -> "-"
              3 -> "@"
            end

          dirty? = rem(i, 7) == 0
          has_emoji? = rem(i, 31) == 0

          field1 = Integer.to_string(i)

          field2 =
            cond do
              dirty? -> "#{trigger}has\ttab_#{i}"
              has_emoji? -> "#{trigger}🎉_#{i}"
              true -> "#{trigger}clean_#{i}"
            end

          field3 = if rem(i, 11) == 0, do: "note \"#{i}\"", else: "note_#{i}"
          field4 = if rem(i, 13) == 0, do: "", else: "val_#{i}"

          [field1, field2, field3, field4]
        end

      assert_identical(RFormulaUTF16, NFormulaUTF16, rows)
    end
  end

  describe "PostProcess::Full — formula + UTF-16 BE" do
    test "clean and dirty triggers" do
      assert_identical(RFormulaUTF16BE, NFormulaUTF16BE, [
        ["=clean", "+dirty\tfield"],
        ["-ok", "@also\"dirty"],
        ["safe", "plain"]
      ])
    end

    test "emoji with trigger" do
      assert_identical(RFormulaUTF16BE, NFormulaUTF16BE, [["=🎉", "ok"]])
    end
  end

  describe "PostProcess::Full — formula + Latin-1" do
    test "trigger with accented chars" do
      assert_identical(RFormulaLatin1, NFormulaLatin1, [["=résumé", "café"]])
    end

    test "dirty trigger with accented chars" do
      assert_identical(RFormulaLatin1, NFormulaLatin1, [["=café,latte", "ok"]])
    end

    test "trigger that is just the char" do
      assert_identical(RFormulaLatin1, NFormulaLatin1, [["="], ["+"]])
    end

    test "non-trigger with accented chars" do
      assert_identical(RFormulaLatin1, NFormulaLatin1, [["naïve", "über"]])
    end
  end

  # ==================================================================
  # 5. Parallel encoding — all PostProcess variants
  # ==================================================================

  describe "parallel encoding" do
    test "FormulaOnly: parallel matches NimbleCSV" do
      rows =
        for i <- 1..500 do
          cond do
            rem(i, 7) == 0 -> ["=has,comma_#{i}", "+clean_#{i}"]
            rem(i, 5) == 0 -> ["=clean_#{i}", "-also\"dirty_#{i}"]
            rem(i, 3) == 0 -> ["@mention_#{i}", "safe_#{i}"]
            true -> ["normal_#{i}", "plain_#{i}"]
          end
        end

      assert_identical(RFormula, NFormula, rows, strategy: :parallel)
    end

    test "EncodingOnly: parallel matches NimbleCSV" do
      rows =
        for i <- 1..500 do
          if rem(i, 5) == 0 do
            ["has\ttab_#{i}", "has\"q_#{i}"]
          else
            ["clean_#{i}", "val_#{i}"]
          end
        end

      # Tab separator and quote escape are single-byte, so this exercises the
      # real parallel encoder path with non-UTF-8 output.
      assert_identical(RUTF16, NUTF16, rows, strategy: :parallel)
    end

    test "Full: parallel matches NimbleCSV (formula + UTF-16)" do
      rows =
        for i <- 1..500 do
          cond do
            rem(i, 7) == 0 -> ["=has\ttab_#{i}", "ok_#{i}"]
            rem(i, 5) == 0 -> ["=clean_#{i}", "+also_#{i}"]
            rem(i, 3) == 0 -> ["@mention_#{i}", "safe_#{i}"]
            true -> ["normal_#{i}", "plain_#{i}"]
          end
        end

      # Tab separator and quote escape are single-byte, so this exercises the
      # real parallel encoder path with formula escaping and UTF-16 output.
      assert_identical(RFormulaUTF16, NFormulaUTF16, rows, strategy: :parallel)
    end

    test "None: parallel matches NimbleCSV" do
      rows =
        for i <- 1..500 do
          if rem(i, 4) == 0 do
            ["has,comma_#{i}", "has\"q_#{i}"]
          else
            ["clean_#{i}", "val_#{i}"]
          end
        end

      assert_identical(RPlain, NPlain, rows, strategy: :parallel)
    end
  end

  # ==================================================================
  # 6. dump_to_stream parity
  # ==================================================================

  describe "dump_to_stream" do
    test "formula: stream output equals batch output" do
      rows = [["=foo", "bar"], ["safe", "+baz,quux"], ["", "-ok"]]
      batch = RFormula.dump_to_iodata(rows) |> IO.iodata_to_binary()
      stream = rows |> RFormula.dump_to_stream() |> Enum.to_list() |> IO.iodata_to_binary()
      assert stream == batch
    end

    test "formula + UTF-16: stream output equals batch output" do
      rows = [["=test", "ok"], ["+dirty\ttab", "safe"]]
      batch = RFormulaUTF16.dump_to_iodata(rows) |> IO.iodata_to_binary()
      stream = rows |> RFormulaUTF16.dump_to_stream() |> Enum.to_list() |> IO.iodata_to_binary()
      assert stream == batch
    end

    test "encoding only: stream output equals batch output" do
      rows = [["hello", "world"], ["has\ttab", "café"]]
      batch = RUTF16.dump_to_iodata(rows) |> IO.iodata_to_binary()
      stream = rows |> RUTF16.dump_to_stream() |> Enum.to_list() |> IO.iodata_to_binary()
      assert stream == batch
    end
  end

  # ==================================================================
  # 7. Structural edge cases
  # ==================================================================

  describe "structural edge cases" do
    test "single row, single field, formula trigger" do
      assert_identical(RFormula, NFormula, [["="]])
    end

    test "single row, single empty field" do
      assert_identical(RFormula, NFormula, [[""]])
    end

    test "single row, single field, non-trigger" do
      assert_identical(RFormula, NFormula, [["safe"]])
    end

    test "many empty rows" do
      assert_identical(RFormula, NFormula, [[], [], []])
    end

    test "rows with all empty fields" do
      assert_identical(RFormula, NFormula, [["", ""], ["", ""], ["", ""]])
    end

    test "wide row (50 fields) with scattered triggers" do
      row =
        for i <- 1..50 do
          case rem(i, 10) do
            0 -> "=trigger_#{i}"
            3 -> "+trigger_#{i}"
            7 -> "has,comma_#{i}"
            _ -> "field_#{i}"
          end
        end

      assert_identical(RFormula, NFormula, [row])
    end

    test "wide row (50 fields) formula + UTF-16" do
      row =
        for i <- 1..50 do
          case rem(i, 10) do
            0 -> "=trigger_#{i}"
            3 -> "+has\ttab_#{i}"
            7 -> "@mention_#{i}"
            _ -> "field_#{i}"
          end
        end

      assert_identical(RFormulaUTF16, NFormulaUTF16, [row])
    end

    test "alternating trigger/non-trigger across many rows" do
      rows =
        for i <- 1..100 do
          if rem(i, 2) == 0, do: ["=trigger_#{i}"], else: ["safe_#{i}"]
        end

      assert_identical(RFormula, NFormula, rows)
    end

    test "field with only whitespace is not trigger-prefixed" do
      assert_identical(RFormula, NFormula, [[" ", "  ", "\t"]])
    end

    test "field starting with trigger in multi-byte UTF-8 context" do
      # = followed by multi-byte UTF-8 char
      assert_identical(RFormula, NFormula, [["=café"]])
      assert_identical(RFormula, NFormula, [["=日本語"]])
      assert_identical(RFormula, NFormula, [["=🎉"]])
    end
  end
end
