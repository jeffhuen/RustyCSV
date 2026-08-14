defmodule RustyCSV.EncodingTest do
  use ExUnit.Case, async: true

  # Define test parsers for different encodings
  RustyCSV.define(TestUTF16LE,
    separator: ",",
    escape: "\"",
    encoding: {:utf16, :little},
    trim_bom: true,
    dump_bom: true
  )

  RustyCSV.define(TestUTF16BE,
    separator: ",",
    escape: "\"",
    encoding: {:utf16, :big},
    trim_bom: true,
    dump_bom: true
  )

  RustyCSV.define(TestUTF32LE,
    separator: ",",
    escape: "\"",
    encoding: {:utf32, :little},
    trim_bom: true,
    dump_bom: true
  )

  RustyCSV.define(TestUTF32BE,
    separator: ",",
    escape: "\"",
    encoding: {:utf32, :big},
    trim_bom: true,
    dump_bom: true
  )

  RustyCSV.define(TestLatin1,
    separator: ",",
    escape: "\"",
    encoding: :latin1,
    trim_bom: false,
    dump_bom: false
  )

  RustyCSV.define(TestUTF8WithBOM,
    separator: ",",
    escape: "\"",
    encoding: :utf8,
    trim_bom: true,
    dump_bom: true
  )

  describe "UTF-16 Little Endian" do
    test "parses UTF-16 LE with BOM" do
      # UTF-16 LE BOM + "a,b\n1,2\n"
      utf16_data =
        <<0xFF, 0xFE>> <> :unicode.characters_to_binary("a,b\n1,2\n", :utf8, {:utf16, :little})

      result = TestUTF16LE.parse_string(utf16_data, skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "parses UTF-16 LE without BOM" do
      utf16_data = :unicode.characters_to_binary("a,b\n1,2\n", :utf8, {:utf16, :little})

      result = TestUTF16LE.parse_string(utf16_data, skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "dumps to UTF-16 LE with BOM" do
      rows = [["a", "b"], ["1", "2"]]
      result = TestUTF16LE.dump_to_iodata(rows) |> IO.iodata_to_binary()

      # Should start with UTF-16 LE BOM
      assert <<0xFF, 0xFE, rest::binary>> = result

      # Convert back and verify
      {:ok, utf8} = :unicode.characters_to_binary(rest, {:utf16, :little}, :utf8) |> wrap_ok()
      assert utf8 == "a,b\n1,2\n"
    end

    test "round-trip UTF-16 LE" do
      original = [["hello", "world"], ["foo", "bar"]]
      encoded = TestUTF16LE.dump_to_iodata(original) |> IO.iodata_to_binary()
      decoded = TestUTF16LE.parse_string(encoded, skip_headers: false)
      assert decoded == original
    end

    test "handles unicode characters in UTF-16 LE" do
      # Japanese characters
      utf16_data =
        <<0xFF, 0xFE>> <>
          :unicode.characters_to_binary(
            "name,value\n\u3053\u3093\u306B\u3061\u306F,hello\n",
            :utf8,
            {:utf16, :little}
          )

      result = TestUTF16LE.parse_string(utf16_data, skip_headers: false)
      assert result == [["name", "value"], ["\u3053\u3093\u306B\u3061\u306F", "hello"]]
    end
  end

  describe "UTF-16 Big Endian" do
    test "parses UTF-16 BE with BOM" do
      # UTF-16 BE BOM + "a,b\n1,2\n"
      utf16_data =
        <<0xFE, 0xFF>> <> :unicode.characters_to_binary("a,b\n1,2\n", :utf8, {:utf16, :big})

      result = TestUTF16BE.parse_string(utf16_data, skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "round-trip UTF-16 BE" do
      original = [["a", "b"], ["1", "2"]]
      encoded = TestUTF16BE.dump_to_iodata(original) |> IO.iodata_to_binary()
      decoded = TestUTF16BE.parse_string(encoded, skip_headers: false)
      assert decoded == original
    end
  end

  describe "UTF-32 Little Endian" do
    test "parses UTF-32 LE with BOM" do
      utf32_data =
        <<0xFF, 0xFE, 0x00, 0x00>> <>
          :unicode.characters_to_binary("a,b\n1,2\n", :utf8, {:utf32, :little})

      result = TestUTF32LE.parse_string(utf32_data, skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "round-trip UTF-32 LE" do
      original = [["hello", "world"], ["emoji 🎉", "café"]]
      encoded = TestUTF32LE.dump_to_iodata(original) |> IO.iodata_to_binary()
      decoded = TestUTF32LE.parse_string(encoded, skip_headers: false)
      assert decoded == original
    end
  end

  describe "UTF-32 Big Endian" do
    test "parses UTF-32 BE with BOM" do
      utf32_data =
        <<0x00, 0x00, 0xFE, 0xFF>> <>
          :unicode.characters_to_binary("a,b\n1,2\n", :utf8, {:utf32, :big})

      result = TestUTF32BE.parse_string(utf32_data, skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "round-trip UTF-32 BE" do
      original = [["left", "right"], ["こんにちは", "world"]]
      encoded = TestUTF32BE.dump_to_iodata(original) |> IO.iodata_to_binary()
      decoded = TestUTF32BE.parse_string(encoded, skip_headers: false)
      assert decoded == original
    end
  end

  describe "Latin-1" do
    test "parses Latin-1 encoded data" do
      # Latin-1 characters: "caf\xe9" (café)
      latin1_data = "name,drink\njohn,caf\xe9\n"

      result = TestLatin1.parse_string(latin1_data, skip_headers: false)
      assert result == [["name", "drink"], ["john", "café"]]
    end

    test "dumps to Latin-1" do
      rows = [["name", "drink"], ["john", "café"]]
      result = TestLatin1.dump_to_iodata(rows) |> IO.iodata_to_binary()

      # Should be Latin-1 encoded
      assert result == "name,drink\njohn,caf\xe9\n"
    end

    test "round-trip Latin-1" do
      original = [["hello", "café"], ["über", "test"]]
      encoded = TestLatin1.dump_to_iodata(original) |> IO.iodata_to_binary()
      decoded = TestLatin1.parse_string(encoded, skip_headers: false)
      assert decoded == original
    end
  end

  describe "UTF-8 with BOM" do
    test "parses UTF-8 with BOM" do
      # UTF-8 BOM + data
      utf8_bom = <<0xEF, 0xBB, 0xBF>>
      data = utf8_bom <> "a,b\n1,2\n"

      result = TestUTF8WithBOM.parse_string(data, skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "dumps UTF-8 with BOM" do
      rows = [["a", "b"], ["1", "2"]]
      result = TestUTF8WithBOM.dump_to_iodata(rows) |> IO.iodata_to_binary()

      # Should start with UTF-8 BOM
      assert <<0xEF, 0xBB, 0xBF, rest::binary>> = result
      assert rest == "a,b\n1,2\n"
    end
  end

  describe "RustyCSV.Spreadsheet" do
    test "parses tab-separated UTF-16 LE" do
      # UTF-16 LE BOM + tab-separated data
      utf16_data =
        <<0xFF, 0xFE>> <> :unicode.characters_to_binary("a\tb\n1\t2\n", :utf8, {:utf16, :little})

      result = RustyCSV.Spreadsheet.parse_string(utf16_data, skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "round-trip spreadsheet format" do
      original = [["Column A", "Column B"], ["Value 1", "Value 2"]]
      encoded = RustyCSV.Spreadsheet.dump_to_iodata(original) |> IO.iodata_to_binary()
      decoded = RustyCSV.Spreadsheet.parse_string(encoded, skip_headers: false)
      assert decoded == original
    end
  end

  describe "streaming with encoding" do
    test "streams UTF-16 LE data" do
      # Create UTF-16 LE encoded chunks
      chunk1 = <<0xFF, 0xFE>> <> :unicode.characters_to_binary("a,b\n", :utf8, {:utf16, :little})
      chunk2 = :unicode.characters_to_binary("1,2\n", :utf8, {:utf16, :little})
      chunk3 = :unicode.characters_to_binary("3,4\n", :utf8, {:utf16, :little})

      result =
        [chunk1, chunk2, chunk3]
        |> TestUTF16LE.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["a", "b"], ["1", "2"], ["3", "4"]]
    end

    test "streams Latin-1 data" do
      chunks = ["name,drink\n", "john,caf\xe9\n", "jane,th\xe9\n"]

      result =
        chunks
        |> TestLatin1.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["name", "drink"], ["john", "café"], ["jane", "thé"]]
    end

    test "streams UTF-16 LE data with headers" do
      chunk1 =
        <<0xFF, 0xFE>> <>
          :unicode.characters_to_binary("name,drink\n", :utf8, {:utf16, :little})

      chunk2 = :unicode.characters_to_binary("john,café\n", :utf8, {:utf16, :little})
      chunk3 = :unicode.characters_to_binary("jane,thé\n", :utf8, {:utf16, :little})

      result =
        [chunk1, chunk2, chunk3]
        |> TestUTF16LE.parse_stream(headers: true)
        |> Enum.to_list()

      assert result == [
               %{"name" => "john", "drink" => "café"},
               %{"name" => "jane", "drink" => "thé"}
             ]
    end

    test "streams UTF-16 BE data with headers" do
      chunk1 =
        <<0xFE, 0xFF>> <>
          :unicode.characters_to_binary("name,drink\n", :utf8, {:utf16, :big})

      chunk2 = :unicode.characters_to_binary("john,café\n", :utf8, {:utf16, :big})
      chunk3 = :unicode.characters_to_binary("jane,thé\n", :utf8, {:utf16, :big})

      result =
        [chunk1, chunk2, chunk3]
        |> TestUTF16BE.parse_stream(headers: true)
        |> Enum.to_list()

      assert result == [
               %{"name" => "john", "drink" => "café"},
               %{"name" => "jane", "drink" => "thé"}
             ]
    end

    test "streams Latin-1 data with headers" do
      chunks = ["name,drink\n", "john,caf\xe9\n", "jane,th\xe9\n"]

      result =
        chunks
        |> TestLatin1.parse_stream(headers: true)
        |> Enum.to_list()

      assert result == [
               %{"name" => "john", "drink" => "café"},
               %{"name" => "jane", "drink" => "thé"}
             ]
    end

    test "handles multi-byte boundary in UTF-16 streaming" do
      # Split a UTF-16 character across chunk boundary
      full_data = :unicode.characters_to_binary("a,b\n1,2\n", :utf8, {:utf16, :little})

      # Split in the middle of a character (odd byte boundary)
      chunk1 = <<0xFF, 0xFE>> <> binary_part(full_data, 0, 5)
      chunk2 = binary_part(full_data, 5, byte_size(full_data) - 5)

      result =
        [chunk1, chunk2]
        |> TestUTF16LE.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["a", "b"], ["1", "2"]]
    end

    test "raises on truncated UTF-16 sequence at end of stream" do
      valid_prefix =
        <<0xFF, 0xFE>> <>
          :unicode.characters_to_binary("a,b\n1,", :utf8, {:utf16, :little})

      truncated_digit = <<?2>>

      assert_raise RustyCSV.ParseError, ~r/truncated.*utf16/i, fn ->
        [valid_prefix, truncated_digit]
        |> TestUTF16LE.parse_stream(skip_headers: false)
        |> Enum.to_list()
      end
    end
  end

  describe "error handling" do
    test "raises on invalid encoding option" do
      assert_raise ArgumentError, ~r/Invalid encoding/, fn ->
        RustyCSV.define(InvalidEncoder, encoding: :invalid)
      end
    end

    test "raises on invalid UTF-16 sequence" do
      # Invalid UTF-16 sequence (incomplete surrogate pair)
      invalid_data = <<0xFF, 0xFE, 0x00, 0xD8>>

      assert_raise RustyCSV.ParseError, ~r/Incomplete|Invalid/, fn ->
        TestUTF16LE.parse_string(invalid_data)
      end
    end
  end

  describe "encoding options" do
    test "reports encoding in options/0" do
      assert Keyword.get(TestUTF16LE.options(), :encoding) == {:utf16, :little}
      assert Keyword.get(TestUTF32LE.options(), :encoding) == {:utf32, :little}
      assert Keyword.get(TestLatin1.options(), :encoding) == :latin1
      assert Keyword.get(RustyCSV.RFC4180.options(), :encoding) == nil
    end
  end

  # Helper to wrap a successful result
  defp wrap_ok(binary) when is_binary(binary), do: {:ok, binary}
  defp wrap_ok(other), do: {:error, other}
end
