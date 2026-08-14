defmodule RustyCSV.HeadersTest do
  use ExUnit.Case, async: true

  alias RustyCSV.RFC4180, as: CSV
  alias RustyCSV.TestHeaders.FlexSep
  alias RustyCSV.TestHeaders.MultiEsc
  alias RustyCSV.TestHeaders.MultiSep
  alias RustyCSV.TestStrategyMatrix

  @strategies TestStrategyMatrix.batch_strategy_atoms()

  # ============================================================================
  # Cross-strategy consistency — the most important test
  # ============================================================================

  describe "cross-strategy consistency" do
    @inputs [
      {"simple", "name,age\njohn,27\njane,30\n"},
      {"quoted headers", "\"name\",\"age\"\njohn,27\n"},
      {"escaped quotes in header", "\"na\"\"me\",age\njohn,27\n"},
      {"fewer columns", "a,b,c\n1,2\n"},
      {"more columns", "a,b\n1,2,3\n"},
      {"empty header", ",b\n1,2\n"},
      {"single column", "x\n1\n2\n"},
      {"crlf", "a,b\r\njohn,27\r\njane,30\r\n"},
      {"mixed line endings", "a,b\r\n1,2\n3,4\r\n"},
      {"duplicate headers", "a,a\n1,2\n"},
      {"many rows",
       Enum.join(["h1,h2,h3" | Enum.map(1..100, &"a#{&1},b#{&1},c#{&1}")], "\n") <> "\n"}
    ]

    for {label, _input} <- @inputs do
      test "all strategies agree: #{label}" do
        {_label, input} = Enum.find(@inputs, fn {l, _} -> l == unquote(label) end)

        results =
          Enum.map(@strategies, fn strat ->
            {strat, CSV.parse_string(input, headers: true, strategy: strat)}
          end)

        [{base_strat, base_result} | rest] = results

        for {strat, result} <- rest do
          assert result == base_result,
                 "#{strat} disagrees with #{base_strat} for #{unquote(label)}:\n" <>
                   "  #{base_strat}: #{inspect(Enum.take(base_result, 3))}\n" <>
                   "  #{strat}: #{inspect(Enum.take(result, 3))}"
        end
      end
    end
  end

  # ============================================================================
  # parse_string vs parse_stream consistency
  # ============================================================================

  describe "parse_string and parse_stream agree" do
    test "headers: true" do
      input = "name,age\njohn,27\njane,30\n"

      string_result = CSV.parse_string(input, headers: true)

      stream_result =
        [input]
        |> CSV.parse_stream(headers: true)
        |> Enum.to_list()

      assert stream_result == string_result
    end

    test "headers: [atoms]" do
      input = "name,age\njohn,27\njane,30\n"

      string_result = CSV.parse_string(input, headers: [:n, :a])

      stream_result =
        [input]
        |> CSV.parse_stream(headers: [:n, :a])
        |> Enum.to_list()

      assert stream_result == string_result
    end

    test "headers: [strings]" do
      input = "name,age\njohn,27\njane,30\n"

      string_result = CSV.parse_string(input, headers: ["x", "y"])

      stream_result =
        [input]
        |> CSV.parse_stream(headers: ["x", "y"])
        |> Enum.to_list()

      assert stream_result == string_result
    end

    test "headers: [keys] with skip_headers: false" do
      input = "name,age\njohn,27\n"

      string_result = CSV.parse_string(input, headers: [:n, :a], skip_headers: false)

      stream_result =
        [input]
        |> CSV.parse_stream(headers: [:n, :a], skip_headers: false)
        |> Enum.to_list()

      assert stream_result == string_result
    end
  end

  # ============================================================================
  # headers: true — first row as string keys (all strategies)
  # ============================================================================

  describe "headers: true" do
    for strategy <- @strategies do
      @tag strategy: strategy
      test "returns maps with string keys (#{strategy})" do
        result =
          CSV.parse_string("name,age\njohn,27\njane,30\n",
            headers: true,
            strategy: unquote(strategy)
          )

        assert result == [
                 %{"name" => "john", "age" => "27"},
                 %{"name" => "jane", "age" => "30"}
               ]
      end
    end

    test "empty input returns empty list" do
      assert CSV.parse_string("", headers: true) == []
    end

    test "header-only input returns empty list" do
      assert CSV.parse_string("name,age\n", headers: true) == []
    end

    test "single data row" do
      result = CSV.parse_string("a,b\n1,2\n", headers: true)
      assert result == [%{"a" => "1", "b" => "2"}]
    end

    test "single column" do
      result = CSV.parse_string("x\n1\n2\n", headers: true)
      assert result == [%{"x" => "1"}, %{"x" => "2"}]
    end

    test "crlf line endings" do
      result = CSV.parse_string("a,b\r\njohn,27\r\njane,30\r\n", headers: true)

      assert result == [
               %{"a" => "john", "b" => "27"},
               %{"a" => "jane", "b" => "30"}
             ]
    end

    test "skip_headers is ignored when headers: true" do
      # headers: true always consumes first row as keys regardless of skip_headers
      result_default = CSV.parse_string("a,b\n1,2\n", headers: true)
      result_explicit = CSV.parse_string("a,b\n1,2\n", headers: true, skip_headers: false)
      assert result_default == result_explicit
    end
  end

  # ============================================================================
  # headers: [atoms] — explicit atom keys (all strategies)
  # ============================================================================

  describe "headers: [atoms]" do
    for strategy <- @strategies do
      @tag strategy: strategy
      test "returns maps with atom keys (#{strategy})" do
        result =
          CSV.parse_string("name,age\njohn,27\njane,30\n",
            headers: [:name, :age],
            strategy: unquote(strategy)
          )

        assert result == [
                 %{name: "john", age: "27"},
                 %{name: "jane", age: "30"}
               ]
      end
    end

    test "skip_headers: true skips first row (default)" do
      result = CSV.parse_string("name,age\njohn,27\n", headers: [:n, :a])
      assert result == [%{n: "john", a: "27"}]
    end

    test "skip_headers: false includes first row as data" do
      result =
        CSV.parse_string("name,age\njohn,27\n",
          headers: [:n, :a],
          skip_headers: false
        )

      assert result == [
               %{n: "name", a: "age"},
               %{n: "john", a: "27"}
             ]
    end
  end

  # ============================================================================
  # headers: [strings] — explicit string keys (all strategies)
  # ============================================================================

  describe "headers: [strings]" do
    for strategy <- @strategies do
      @tag strategy: strategy
      test "returns maps with custom string keys (#{strategy})" do
        result =
          CSV.parse_string("name,age\njohn,27\n",
            headers: ["n", "a"],
            strategy: unquote(strategy)
          )

        assert result == [%{"n" => "john", "a" => "27"}]
      end
    end
  end

  # ============================================================================
  # Edge cases — tested across all strategies
  # ============================================================================

  describe "edge cases" do
    for strategy <- @strategies do
      @tag strategy: strategy
      test "fewer columns than headers → nil (#{strategy})" do
        result = CSV.parse_string("a,b,c\n1,2\n", headers: true, strategy: unquote(strategy))
        assert result == [%{"a" => "1", "b" => "2", "c" => nil}]
      end

      @tag strategy: strategy
      test "more columns than headers → ignored (#{strategy})" do
        result = CSV.parse_string("a,b\n1,2,3\n", headers: true, strategy: unquote(strategy))
        assert result == [%{"a" => "1", "b" => "2"}]
      end

      @tag strategy: strategy
      test "duplicate headers → last wins (#{strategy})" do
        result = CSV.parse_string("a,a\n1,2\n", headers: true, strategy: unquote(strategy))
        assert result == [%{"a" => "2"}]
      end

      @tag strategy: strategy
      test "empty header field (#{strategy})" do
        result = CSV.parse_string(",b\n1,2\n", headers: true, strategy: unquote(strategy))
        assert result == [%{"" => "1", "b" => "2"}]
      end

      @tag strategy: strategy
      test "quoted headers (#{strategy})" do
        result =
          CSV.parse_string(~s("name","age"\njohn,27\n),
            headers: true,
            strategy: unquote(strategy)
          )

        assert result == [%{"name" => "john", "age" => "27"}]
      end

      @tag strategy: strategy
      test "headers with escaped quotes (#{strategy})" do
        result =
          CSV.parse_string(~s("na""me",age\njohn,27\n),
            headers: true,
            strategy: unquote(strategy)
          )

        assert result == [%{"na\"me" => "john", "age" => "27"}]
      end
    end

    test "fewer columns with explicit keys → nil" do
      result = CSV.parse_string("x,y,z\n1,2\n", headers: [:a, :b, :c])
      assert result == [%{a: "1", b: "2", c: nil}]
    end

    test "more columns with explicit keys → ignored" do
      result = CSV.parse_string("x,y\n1,2,3\n", headers: [:a, :b])
      assert result == [%{a: "1", b: "2"}]
    end

    test "many rows preserves order" do
      rows = Enum.map(1..200, &"#{&1},v#{&1}")
      input = Enum.join(["id,val" | rows], "\n") <> "\n"

      result = CSV.parse_string(input, headers: true)

      assert Enum.count(result) == 200
      assert hd(result) == %{"id" => "1", "val" => "v1"}
      assert List.last(result) == %{"id" => "200", "val" => "v200"}
    end

    test "quoted data values in maps" do
      result = CSV.parse_string(~s(a,b\n"hello, world","line1\nline2"\n), headers: true)
      assert result == [%{"a" => "hello, world", "b" => "line1\nline2"}]
    end

    test "all empty fields" do
      result = CSV.parse_string("a,b\n,\n", headers: true)
      assert result == [%{"a" => "", "b" => ""}]
    end

    test "empty rows" do
      result = CSV.parse_string("a\n\n\n", headers: true)
      assert result == [%{"a" => ""}, %{"a" => ""}]
    end
  end

  # ============================================================================
  # Streaming with headers
  # ============================================================================

  describe "parse_stream with headers: true" do
    test "returns maps from stream" do
      result =
        ["name,age\n", "john,27\n", "jane,30\n"]
        |> CSV.parse_stream(headers: true)
        |> Enum.to_list()

      assert result == [
               %{"name" => "john", "age" => "27"},
               %{"name" => "jane", "age" => "30"}
             ]
    end

    test "streaming with explicit atom keys" do
      result =
        ["name,age\n", "john,27\n"]
        |> CSV.parse_stream(headers: [:n, :a])
        |> Enum.to_list()

      assert result == [%{n: "john", a: "27"}]
    end

    test "streaming with explicit string keys" do
      result =
        ["name,age\n", "john,27\n"]
        |> CSV.parse_stream(headers: ["x", "y"])
        |> Enum.to_list()

      assert result == [%{"x" => "john", "y" => "27"}]
    end

    test "streaming with explicit keys and skip_headers: false" do
      result =
        ["name,age\n", "john,27\n"]
        |> CSV.parse_stream(headers: [:n, :a], skip_headers: false)
        |> Enum.to_list()

      assert result == [%{n: "name", a: "age"}, %{n: "john", a: "27"}]
    end

    test "streaming fewer columns → nil" do
      result =
        ["a,b,c\n", "1,2\n"]
        |> CSV.parse_stream(headers: true)
        |> Enum.to_list()

      assert result == [%{"a" => "1", "b" => "2", "c" => nil}]
    end

    test "streaming more columns → ignored" do
      result =
        ["a,b\n", "1,2,3\n"]
        |> CSV.parse_stream(headers: true)
        |> Enum.to_list()

      assert result == [%{"a" => "1", "b" => "2"}]
    end

    test "header and data split across chunks" do
      # Header row in first chunk, data in subsequent chunks
      result =
        ["na", "me,age\njohn,", "27\njane,30\n"]
        |> CSV.parse_stream(headers: true)
        |> Enum.to_list()

      assert result == [
               %{"name" => "john", "age" => "27"},
               %{"name" => "jane", "age" => "30"}
             ]
    end

    test "many rows streaming" do
      rows = Enum.map(1..200, &"#{&1},v#{&1}\n")
      chunks = ["id,val\n" | rows]

      result =
        chunks
        |> CSV.parse_stream(headers: true)
        |> Enum.to_list()

      assert Enum.count(result) == 200
      assert hd(result) == %{"id" => "1", "val" => "v1"}
      assert List.last(result) == %{"id" => "200", "val" => "v200"}
    end
  end

  # ============================================================================
  # Regression: headers: false unchanged
  # ============================================================================

  describe "headers: false regression" do
    for strategy <- @strategies do
      @tag strategy: strategy
      test "returns lists with headers: false (#{strategy})" do
        result =
          CSV.parse_string("name,age\njohn,27\n",
            headers: false,
            strategy: unquote(strategy)
          )

        assert result == [["john", "27"]]
      end
    end

    test "default behavior unchanged" do
      result = CSV.parse_string("name,age\njohn,27\n")
      assert result == [["john", "27"]]
    end

    test "streaming default unchanged" do
      result =
        ["name,age\n", "john,27\n"]
        |> CSV.parse_stream()
        |> Enum.to_list()

      assert result == [["john", "27"]]
    end
  end

  # ============================================================================
  # Validation
  # ============================================================================

  describe "invalid headers option" do
    test "raises ArgumentError for atom" do
      assert_raise ArgumentError, ~r/invalid :headers option/, fn ->
        CSV.parse_string("a,b\n1,2\n", headers: :invalid)
      end
    end

    test "raises ArgumentError for integer" do
      assert_raise ArgumentError, ~r/invalid :headers option/, fn ->
        CSV.parse_string("a,b\n1,2\n", headers: 42)
      end
    end

    test "raises ArgumentError for stream with invalid value" do
      assert_raise ArgumentError, ~r/invalid :headers option/, fn ->
        ["a,b\n"] |> CSV.parse_stream(headers: :invalid) |> Enum.to_list()
      end
    end
  end

  # ============================================================================
  # Custom parsers (module-level defines for reliable compilation)
  # ============================================================================

  # Multi-byte separator
  RustyCSV.define(RustyCSV.TestHeaders.MultiSep,
    separator: "::",
    escape: "\""
  )

  # Multi-separator
  RustyCSV.define(RustyCSV.TestHeaders.FlexSep,
    separator: [",", ";"],
    escape: "\""
  )

  # Multi-byte escape
  RustyCSV.define(RustyCSV.TestHeaders.MultiEsc,
    separator: ",",
    escape: "$$"
  )

  describe "custom parser with multi-byte separator" do
    test "headers: true with :: separator" do
      result = MultiSep.parse_string("name::age\njohn::27\n", headers: true)

      assert result == [%{"name" => "john", "age" => "27"}]
    end

    test "headers: [atoms] with :: separator" do
      result =
        MultiSep.parse_string("name::age\njohn::27\n",
          headers: [:n, :a]
        )

      assert result == [%{n: "john", a: "27"}]
    end
  end

  describe "custom parser with multi-separator" do
    test "headers: true with multiple separators" do
      result = FlexSep.parse_string("name,age\njohn;27\n", headers: true)

      assert result == [%{"name" => "john", "age" => "27"}]
    end
  end

  describe "custom parser with multi-byte escape" do
    test "headers: true with $$ escape" do
      result = MultiEsc.parse_string("$$name$$,age\njohn,27\n", headers: true)

      assert result == [%{"name" => "john", "age" => "27"}]
    end

    test "escaped $$ in header value" do
      result =
        MultiEsc.parse_string("$$na$$$$me$$,age\njohn,27\n",
          headers: true
        )

      assert result == [%{"na$$me" => "john", "age" => "27"}]
    end
  end
end
