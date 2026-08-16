defmodule RustyCSV.ParityRegressionTest do
  use ExUnit.Case, async: true

  RustyCSV.define(Plain, line_separator: "\n")
  NimbleCSV.define(NimblePlain, line_separator: "\n")

  RustyCSV.define(MultiSeparator, separator: [",", ";"], line_separator: "\n")
  NimbleCSV.define(NimbleMultiSeparator, separator: [",", ";"], line_separator: "\n")

  RustyCSV.define(MixedSeparator, separator: [",", "::"], line_separator: "\n")
  NimbleCSV.define(NimbleMixedSeparator, separator: [",", "::"], line_separator: "\n")

  RustyCSV.define(ParityPipeNewline, newlines: ["|"], line_separator: "|")
  NimbleCSV.define(NimbleParityPipeNewline, newlines: ["|"], line_separator: "|")

  RustyCSV.define(MultiByteFormula, escape_formula: %{["–"] => "'"})

  RustyCSV.define(Utf16, encoding: {:utf16, :little}, line_separator: "\n")
  RustyCSV.define(Latin1, encoding: :latin1)

  test "strict streaming accepts a final doubled quote without a newline" do
    input = ~s("a""a")

    assert Plain.parse_stream([input], skip_headers: false) |> Enum.to_list() ==
             NimblePlain.parse_stream([input], skip_headers: false) |> Enum.to_list()

    assert ParityPipeNewline.parse_stream([input], skip_headers: false) |> Enum.to_list() ==
             NimbleParityPipeNewline.parse_stream([input], skip_headers: false) |> Enum.to_list()

    assert MixedSeparator.parse_stream([input], skip_headers: false) |> Enum.to_list() ==
             NimbleMixedSeparator.parse_stream([input], skip_headers: false) |> Enum.to_list()
  end

  test "parallel and streaming preserve blank rows" do
    input = "a\n\nb\n"
    expected = NimblePlain.parse_string(input, skip_headers: false)

    assert Plain.parse_string(input, skip_headers: false, strategy: :parallel) == expected
    assert Plain.parse_stream([input], skip_headers: false) |> Enum.to_list() == expected
  end

  test "default reserved patterns preserve custom newlines and every separator" do
    rows = [["\"", "|", "ab\""]]
    assert_dump_matches(ParityPipeNewline, NimbleParityPipeNewline, rows)

    rows = [["a;b", "c,d"]]
    assert_dump_matches(MultiSeparator, NimbleMultiSeparator, rows)
    assert_dump_matches(MultiSeparator, NimbleMultiSeparator, rows, strategy: :parallel)

    assert_dump_matches(MixedSeparator, NimbleMixedSeparator, [["a::b", "x"]])
  end

  test "formula escaping matches the entire configured trigger" do
    rows = [["–formula", "—not-a-trigger", "…not-a-trigger"]]
    expected = [["'–formula", "—not-a-trigger", "…not-a-trigger"]]

    for opts <- [[], [strategy: :parallel]] do
      output = MultiByteFormula.dump_to_iodata(rows, opts) |> IO.iodata_to_binary()
      assert MultiByteFormula.parse_string(output, skip_headers: false) == expected
    end
  end

  test "non-UTF-8 encoders reject invalid or unmappable input" do
    assert_raise RuntimeError, fn -> Utf16.dump_to_iodata([[<<0xFF>>]]) end
    assert_raise RuntimeError, fn -> Utf16.dump_to_iodata([[<<0xFF>>]], strategy: :parallel) end
    assert_raise RuntimeError, fn -> Utf16.dump_to_stream([[<<0xFF>>]]) |> Enum.to_list() end
    assert_raise RuntimeError, fn -> Latin1.dump_to_iodata([["日本"]]) end
  end

  test "to_line_stream recognizes newlines in the configured encoding" do
    input = :unicode.characters_to_binary("a,b\nc,d\nlast", :utf8, {:utf16, :little})
    <<first::binary-size(7), second::binary-size(4), rest::binary>> = input

    expected = [
      :unicode.characters_to_binary("a,b\n", :utf8, {:utf16, :little}),
      :unicode.characters_to_binary("c,d\n", :utf8, {:utf16, :little}),
      :unicode.characters_to_binary("last", :utf8, {:utf16, :little})
    ]

    assert Utf16.to_line_stream([first, second, rest]) |> Enum.to_list() == expected
  end

  test "stream_device continues after a chunk ending mid-row" do
    {:ok, device} = StringIO.open("a,b\nc,d\n")

    assert RustyCSV.Streaming.stream_device(device, chunk_size: 2) |> Enum.to_list() == [
             ["a", "b"],
             ["c", "d"]
           ]
  end

  test "stream_device applies configured input encoding" do
    input = :unicode.characters_to_binary("a,b\nc,d\n", :utf8, {:utf16, :little})
    {:ok, device} = StringIO.open(input)

    assert RustyCSV.Streaming.stream_device(device,
             chunk_size: 3,
             encoding: {:utf16, :little}
           )
           |> Enum.to_list() == [["a", "b"], ["c", "d"]]
  end

  @tag :tmp_dir
  test "stream_file reads fixed-size binary chunks", %{tmp_dir: tmp_dir} do
    path = Path.join(tmp_dir, "rows.csv")
    File.write!(path, "a,b\nc,d\n")

    assert RustyCSV.Streaming.stream_file(path, chunk_size: 2) |> Enum.to_list() == [
             ["a", "b"],
             ["c", "d"]
           ]
  end

  test "a finalized native parser does not retain an old strict violation" do
    parser = RustyCSV.Native.streaming_new_with_config(",", "\"", :default, true)

    RustyCSV.Native.streaming_feed(parser, ~s("unterminated))

    assert {:error, {:malformed_csv, :unterminated_quote, _position}} =
             RustyCSV.Native.streaming_finalize(parser)

    RustyCSV.Native.streaming_feed(parser, "a,b\n")
    assert RustyCSV.Native.streaming_next_rows(parser, 1) == [["a", "b"]]
    assert RustyCSV.Native.streaming_finalize(parser) == []
  end

  defp assert_dump_matches(rusty, nimble, rows, opts \\ []) do
    rusty_output = rusty.dump_to_iodata(rows, opts) |> IO.iodata_to_binary()
    nimble_output = nimble.dump_to_iodata(rows) |> IO.iodata_to_binary()
    assert rusty_output == nimble_output
  end
end
