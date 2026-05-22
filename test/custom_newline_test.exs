defmodule CustomNewlineTest do
  @moduledoc """
  Tests for custom newline support via the `newlines` option in `RustyCSV.define/2`.
  """
  use ExUnit.Case

  alias RustyCSV.TestStrategyMatrix

  # Define parsers with custom newlines
  RustyCSV.define(PipeNewline,
    separator: ",",
    escape: "\"",
    newlines: ["|"]
  )

  RustyCSV.define(BrNewline,
    separator: ",",
    escape: "\"",
    newlines: ["<br>"]
  )

  RustyCSV.define(MultiNewline,
    separator: ",",
    escape: "\"",
    newlines: ["<br>", "|"]
  )

  # ============================================================
  # Single-byte custom newline
  # ============================================================

  describe "pipe newline" do
    test "parse_string basic" do
      assert PipeNewline.parse_string("a,b|1,2|", skip_headers: false) ==
               [["a", "b"], ["1", "2"]]
    end

    test "parse_string with skip_headers" do
      assert PipeNewline.parse_string("a,b|1,2|") == [["1", "2"]]
    end

    test "parse_string no trailing newline" do
      assert PipeNewline.parse_string("a,b|1,2", skip_headers: false) ==
               [["a", "b"], ["1", "2"]]
    end

    test "parse_string with quoted field containing pipe" do
      assert PipeNewline.parse_string("\"a|b\",c|1,2|", skip_headers: false) ==
               [["a|b", "c"], ["1", "2"]]
    end

    test "parse_string with all strategies" do
      csv = "a,b|1,2|"
      expected = [["a", "b"], ["1", "2"]]

      for strategy <- TestStrategyMatrix.batch_strategy_atoms() do
        result = PipeNewline.parse_string(csv, skip_headers: false, strategy: strategy)
        assert result == expected, "Failed for strategy: #{strategy}"
      end
    end

    test "streaming" do
      result =
        ["a,b|1,", "2|3,4|"]
        |> PipeNewline.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["a", "b"], ["1", "2"], ["3", "4"]]
    end

    test "headers: true" do
      result = PipeNewline.parse_string("name,age|john,27|jane,30|", headers: true)
      assert result == [%{"name" => "john", "age" => "27"}, %{"name" => "jane", "age" => "30"}]
    end

    test "headers: list" do
      result =
        PipeNewline.parse_string("a,b|1,2|",
          headers: [:x, :y],
          skip_headers: false
        )

      assert result == [%{x: "a", y: "b"}, %{x: "1", y: "2"}]
    end
  end

  # ============================================================
  # Multi-byte custom newline
  # ============================================================

  describe "multi-byte newline (<br>)" do
    test "parse_string basic" do
      assert BrNewline.parse_string("a,b<br>1,2<br>", skip_headers: false) ==
               [["a", "b"], ["1", "2"]]
    end

    test "parse_string no trailing newline" do
      assert BrNewline.parse_string("a,b<br>1,2", skip_headers: false) ==
               [["a", "b"], ["1", "2"]]
    end

    test "parse_string with all strategies" do
      csv = "a,b<br>1,2<br>"
      expected = [["a", "b"], ["1", "2"]]

      for strategy <- TestStrategyMatrix.batch_strategy_atoms() do
        result = BrNewline.parse_string(csv, skip_headers: false, strategy: strategy)
        assert result == expected, "Failed for strategy: #{strategy}"
      end
    end

    test "streaming" do
      result =
        ["a,b<br>1,", "2<br>3,4<br>"]
        |> BrNewline.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert result == [["a", "b"], ["1", "2"], ["3", "4"]]
    end
  end

  # ============================================================
  # Multiple custom newlines
  # ============================================================

  describe "multiple custom newlines (<br> and |)" do
    test "parse_string with both newline types" do
      assert MultiNewline.parse_string("a,b<br>1,2|3,4<br>", skip_headers: false) ==
               [["a", "b"], ["1", "2"], ["3", "4"]]
    end
  end

  # ============================================================
  # Default newlines still work
  # ============================================================

  describe "default newlines unchanged" do
    test "RFC4180 with \\r\\n" do
      result = RustyCSV.RFC4180.parse_string("a,b\r\n1,2\n", skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end

    test "RFC4180 with \\n" do
      result = RustyCSV.RFC4180.parse_string("a,b\n1,2\n", skip_headers: false)
      assert result == [["a", "b"], ["1", "2"]]
    end
  end

  # ============================================================
  # Dumping still works with custom newlines
  # ============================================================

  describe "dumping" do
    test "dump uses line_separator, not newlines" do
      # Custom newlines only affect parsing; dumping uses @line_separator
      result = PipeNewline.dump_to_iodata([["a", "b"], ["1", "2"]]) |> IO.iodata_to_binary()
      assert result == "a,b\n1,2\n"
    end
  end

  # ============================================================
  # Options/0 reflects newlines
  # ============================================================

  describe "options" do
    test "pipe newline options include custom newlines" do
      opts = PipeNewline.options()
      assert opts[:newlines] == ["|"]
    end

    test "br newline options include custom newlines" do
      opts = BrNewline.options()
      assert opts[:newlines] == ["<br>"]
    end
  end
end
