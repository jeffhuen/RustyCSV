defmodule NativeSeparatorTest do
  @moduledoc """
  Tests that all 6 low-level parser entrypoints accept integer, binary, and
  list-of-binaries separators, and both integer and binary escape values.
  """
  use ExUnit.Case

  @csv "a,b\n1,2\n"
  @tsv "a\tb\n1\t2\n"
  @expected [["a", "b"], ["1", "2"]]

  @multi_csv "a,b;c\n1;2,3\n"
  @multi_expected [["a", "b", "c"], ["1", "2", "3"]]

  # ==========================================================================
  # Integer separator (u8)
  # ==========================================================================

  describe "integer separator" do
    test "parse_string_with_config accepts integer separator" do
      assert RustyCSV.Native.parse_string_with_config(@csv, 44, 34, :default) == @expected
    end

    test "parse_string_fast_with_config accepts integer separator" do
      assert RustyCSV.Native.parse_string_fast_with_config(@csv, 44, 34, :default) == @expected
    end

    test "parse_string_indexed_with_config accepts integer separator" do
      assert RustyCSV.Native.parse_string_indexed_with_config(@csv, 44, 34, :default) == @expected
    end

    test "parse_string_parallel_with_config accepts integer separator" do
      assert RustyCSV.Native.parse_string_parallel_with_config(@csv, 44, 34, :default) ==
               @expected
    end

    test "parse_string_zero_copy_with_config accepts integer separator" do
      assert RustyCSV.Native.parse_string_zero_copy_with_config(@csv, 44, 34, :default) ==
               @expected
    end

    test "streaming_new_with_config accepts integer separator" do
      parser = RustyCSV.Native.streaming_new_with_config(44, 34, :default)
      RustyCSV.Native.streaming_feed(parser, @csv)
      rows = RustyCSV.Native.streaming_next_rows(parser, 100)
      assert rows == @expected
    end
  end

  # ==========================================================================
  # Integer separator with tab
  # ==========================================================================

  describe "integer tab separator" do
    test "parse_string_with_config accepts tab as integer 9" do
      assert RustyCSV.Native.parse_string_with_config(@tsv, 9, 34, :default) == @expected
    end

    test "parse_string_fast_with_config accepts tab as integer 9" do
      assert RustyCSV.Native.parse_string_fast_with_config(@tsv, 9, 34, :default) == @expected
    end

    test "parse_string_indexed_with_config accepts tab as integer 9" do
      assert RustyCSV.Native.parse_string_indexed_with_config(@tsv, 9, 34, :default) == @expected
    end

    test "parse_string_parallel_with_config accepts tab as integer 9" do
      assert RustyCSV.Native.parse_string_parallel_with_config(@tsv, 9, 34, :default) == @expected
    end

    test "parse_string_zero_copy_with_config accepts tab as integer 9" do
      assert RustyCSV.Native.parse_string_zero_copy_with_config(@tsv, 9, 34, :default) ==
               @expected
    end

    test "streaming_new_with_config accepts tab as integer 9" do
      parser = RustyCSV.Native.streaming_new_with_config(9, 34, :default)
      RustyCSV.Native.streaming_feed(parser, @tsv)
      rows = RustyCSV.Native.streaming_next_rows(parser, 100)
      assert rows == @expected
    end
  end

  # ==========================================================================
  # Binary separator
  # ==========================================================================

  describe "binary separator" do
    test "parse_string_with_config accepts binary separator" do
      assert RustyCSV.Native.parse_string_with_config(@csv, <<44>>, 34, :default) == @expected
    end

    test "parse_string_fast_with_config accepts binary separator" do
      assert RustyCSV.Native.parse_string_fast_with_config(@csv, <<44>>, 34, :default) ==
               @expected
    end

    test "parse_string_indexed_with_config accepts binary separator" do
      assert RustyCSV.Native.parse_string_indexed_with_config(@csv, <<44>>, 34, :default) ==
               @expected
    end

    test "parse_string_parallel_with_config accepts binary separator" do
      assert RustyCSV.Native.parse_string_parallel_with_config(@csv, <<44>>, 34, :default) ==
               @expected
    end

    test "parse_string_zero_copy_with_config accepts binary separator" do
      assert RustyCSV.Native.parse_string_zero_copy_with_config(@csv, <<44>>, 34, :default) ==
               @expected
    end

    test "streaming_new_with_config accepts binary separator" do
      parser = RustyCSV.Native.streaming_new_with_config(<<44>>, 34, :default)
      RustyCSV.Native.streaming_feed(parser, @csv)
      rows = RustyCSV.Native.streaming_next_rows(parser, 100)
      assert rows == @expected
    end
  end

  # ==========================================================================
  # List-of-binaries separator (multiple single-byte separators)
  # ==========================================================================

  describe "list-of-binaries separator" do
    test "parse_string_with_config accepts list of binaries" do
      assert RustyCSV.Native.parse_string_with_config(@multi_csv, [<<44>>, <<59>>], 34, :default) ==
               @multi_expected
    end

    test "parse_string_fast_with_config accepts list of binaries" do
      assert RustyCSV.Native.parse_string_fast_with_config(
               @multi_csv,
               [<<44>>, <<59>>],
               34,
               :default
             ) ==
               @multi_expected
    end

    test "parse_string_indexed_with_config accepts list of binaries" do
      assert RustyCSV.Native.parse_string_indexed_with_config(
               @multi_csv,
               [<<44>>, <<59>>],
               34,
               :default
             ) ==
               @multi_expected
    end

    test "parse_string_parallel_with_config accepts list of binaries" do
      assert RustyCSV.Native.parse_string_parallel_with_config(
               @multi_csv,
               [<<44>>, <<59>>],
               34,
               :default
             ) ==
               @multi_expected
    end

    test "parse_string_zero_copy_with_config accepts list of binaries" do
      assert RustyCSV.Native.parse_string_zero_copy_with_config(
               @multi_csv,
               [<<44>>, <<59>>],
               34,
               :default
             ) ==
               @multi_expected
    end

    test "streaming_new_with_config accepts list of binaries" do
      parser = RustyCSV.Native.streaming_new_with_config([<<44>>, <<59>>], 34, :default)
      RustyCSV.Native.streaming_feed(parser, @multi_csv)
      rows = RustyCSV.Native.streaming_next_rows(parser, 100)
      assert rows == @multi_expected
    end
  end

  # ==========================================================================
  # Multi-byte separator via Native API
  # ==========================================================================

  describe "multi-byte separator" do
    @double_colon_csv "a::b::c\n1::2::3\n"
    @double_colon_expected [["a", "b", "c"], ["1", "2", "3"]]

    test "parse_string_with_config accepts multi-byte binary separator" do
      assert RustyCSV.Native.parse_string_with_config(@double_colon_csv, "::", 34, :default) ==
               @double_colon_expected
    end

    test "parse_string_fast_with_config accepts multi-byte binary separator" do
      assert RustyCSV.Native.parse_string_fast_with_config(@double_colon_csv, "::", 34, :default) ==
               @double_colon_expected
    end

    test "parse_string_indexed_with_config accepts multi-byte binary separator" do
      assert RustyCSV.Native.parse_string_indexed_with_config(
               @double_colon_csv,
               "::",
               34,
               :default
             ) ==
               @double_colon_expected
    end

    test "parse_string_parallel_with_config accepts multi-byte binary separator" do
      assert RustyCSV.Native.parse_string_parallel_with_config(
               @double_colon_csv,
               "::",
               34,
               :default
             ) ==
               @double_colon_expected
    end

    test "parse_string_zero_copy_with_config accepts multi-byte binary separator" do
      assert RustyCSV.Native.parse_string_zero_copy_with_config(
               @double_colon_csv,
               "::",
               34,
               :default
             ) ==
               @double_colon_expected
    end

    test "streaming_new_with_config accepts multi-byte binary separator" do
      parser = RustyCSV.Native.streaming_new_with_config("::", 34, :default)
      RustyCSV.Native.streaming_feed(parser, @double_colon_csv)
      rows = RustyCSV.Native.streaming_next_rows(parser, 100)
      assert rows == @double_colon_expected
    end
  end

  # ==========================================================================
  # Binary escape via Native API
  # ==========================================================================

  describe "binary escape" do
    test "parse_string_with_config accepts binary escape" do
      assert RustyCSV.Native.parse_string_with_config(@csv, 44, <<34>>, :default) == @expected
    end

    test "parse_string_with_config accepts binary escape for quoting" do
      csv = "\"hello\",world\n"

      assert RustyCSV.Native.parse_string_with_config(csv, 44, <<34>>, :default) == [
               ["hello", "world"]
             ]
    end

    test "streaming_new_with_config accepts binary escape" do
      parser = RustyCSV.Native.streaming_new_with_config(44, <<34>>, :default)
      RustyCSV.Native.streaming_feed(parser, @csv)
      rows = RustyCSV.Native.streaming_next_rows(parser, 100)
      assert rows == @expected
    end
  end

  # ==========================================================================
  # Multi-byte escape via Native API
  # ==========================================================================

  describe "multi-byte escape" do
    test "parse_string_with_config handles multi-byte escape" do
      csv = "$$hello$$,world\n"

      assert RustyCSV.Native.parse_string_with_config(csv, 44, "$$", :default) == [
               ["hello", "world"]
             ]
    end

    test "parse_string_with_config handles doubled multi-byte escape" do
      csv = "$$val$$$$ue$$,other\n"

      assert RustyCSV.Native.parse_string_with_config(csv, 44, "$$", :default) == [
               ["val$$ue", "other"]
             ]
    end
  end

  # ==========================================================================
  # Streaming end-to-end with integer separator
  # ==========================================================================

  describe "streaming with integer separator" do
    test "parse_chunks works with integer separator" do
      result = RustyCSV.Streaming.parse_chunks(["a\tb\n1\t", "2\n3\t4\n"], separator: 9)
      assert result == [["a", "b"], ["1", "2"], ["3", "4"]]
    end

    test "stream_enumerable works with integer separator" do
      result =
        ["a\tb\n", "1\t2\n"]
        |> RustyCSV.Streaming.stream_enumerable(separator: 9)
        |> Enum.to_list()

      assert result == [["a", "b"], ["1", "2"]]
    end
  end

  # ==========================================================================
  # Error cases
  # ==========================================================================

  describe "invalid separator" do
    test "empty binary separator raises ArgumentError" do
      assert_raise ArgumentError, fn ->
        RustyCSV.Native.parse_string_with_config(@csv, <<>>, 34, :default)
      end
    end

    test "empty list separator raises ArgumentError" do
      assert_raise ArgumentError, fn ->
        RustyCSV.Native.parse_string_with_config(@csv, [], 34, :default)
      end
    end

    test "empty escape raises ArgumentError" do
      assert_raise ArgumentError, fn ->
        RustyCSV.Native.parse_string_with_config(@csv, 44, <<>>, :default)
      end
    end
  end
end
