defmodule RustyCSV.ConcurrentAccessTest do
  @moduledoc """
  Tests for concurrent access to streaming parser resources.

  Verifies that:
  - Multiple processes can safely feed/drain the same streaming parser
  - Concurrent batch parsing from many processes doesn't crash
  - No data corruption under contention
  """
  use ExUnit.Case, async: true

  alias RustyCSV.Native
  alias RustyCSV.TestStrategyMatrix

  # ============================================================================
  # Concurrent streaming: many processes, one parser
  # ============================================================================

  describe "concurrent streaming access" do
    test "multiple processes feeding the same parser concurrently" do
      parser = Native.streaming_new()

      # Spawn 20 processes that all feed data concurrently
      tasks =
        for i <- 1..20 do
          Task.async(fn ->
            chunk = "field_#{i}_a,field_#{i}_b\n"
            Native.streaming_feed(parser, chunk)
          end)
        end

      # All feeds should succeed without crash
      results = Task.await_many(tasks, 5_000)
      assert length(results) == 20

      # Drain all rows — should have exactly 20 rows
      rows = Native.streaming_next_rows(parser, 100)
      assert length(rows) == 20
    end

    test "concurrent feed and drain on the same parser" do
      parser = Native.streaming_new()

      # Producers feed data
      producers =
        for i <- 1..10 do
          Task.async(fn ->
            for j <- 1..5 do
              Native.streaming_feed(parser, "p#{i}_r#{j}_a,p#{i}_r#{j}_b\n")
              Process.sleep(1)
            end
          end)
        end

      # Consumer drains rows concurrently with producers
      consumer =
        Task.async(fn ->
          drain_loop(parser, [], 200)
        end)

      # Wait for all producers to finish
      Task.await_many(producers, 10_000)

      # Give consumer a moment to catch up, then finalize
      Process.sleep(50)
      remaining = Native.streaming_next_rows(parser, 1000)

      consumed = Task.await(consumer, 5_000)
      total = length(consumed) + length(remaining)

      # 10 producers × 5 rows each = 50 total
      assert total == 50,
             "expected 50 rows, got #{total} (consumed=#{length(consumed)}, remaining=#{length(remaining)})"
    end

    test "concurrent status checks don't interfere with feed/drain" do
      parser = Native.streaming_new()

      feeder =
        Task.async(fn ->
          for _ <- 1..20 do
            Native.streaming_feed(parser, "a,b,c\n")
            Process.sleep(1)
          end
        end)

      status_checker =
        Task.async(fn ->
          for _ <- 1..50 do
            {avail, buf_size, has_partial} = Native.streaming_status(parser)
            assert is_integer(avail)
            assert is_integer(buf_size)
            assert is_boolean(has_partial)
            Process.sleep(1)
          end
        end)

      drainer =
        Task.async(fn ->
          drain_loop(parser, [], 100)
        end)

      Task.await(feeder, 5_000)
      Task.await(status_checker, 5_000)
      Task.await(drainer, 5_000)

      # No crash = success. The mutex ensures serialized access.
    end

    test "finalize under concurrent access" do
      parser = Native.streaming_new()
      Native.streaming_feed(parser, "a,b\n1,2\n3,4")

      # Drain completed rows first so only the partial row remains
      _completed = Native.streaming_next_rows(parser, 100)

      # Multiple processes try to finalize concurrently
      tasks =
        for _ <- 1..10 do
          Task.async(fn ->
            Native.streaming_finalize(parser)
          end)
        end

      results = Task.await_many(tasks, 5_000)

      # All should return lists (some empty, one with the partial row)
      assert Enum.all?(results, &is_list/1)

      # Exactly one should have gotten the partial row "3,4"
      non_empty = Enum.reject(results, &(&1 == []))
      assert length(non_empty) == 1
      assert hd(non_empty) == [["3", "4"]]
    end
  end

  # ============================================================================
  # Concurrent batch parsing: many processes, independent parses
  # ============================================================================

  describe "concurrent batch parsing" do
    test "many processes parsing different data simultaneously" do
      tasks =
        for i <- 1..50 do
          Task.async(fn ->
            csv = "h1,h2\nval_#{i}_1,val_#{i}_2\n"

            for strategy <- TestStrategyMatrix.batch_strategy_atoms() do
              result = RustyCSV.RFC4180.parse_string(csv, strategy: strategy, skip_headers: false)

              assert result == [["h1", "h2"], ["val_#{i}_1", "val_#{i}_2"]]
              strategy
            end
          end)
        end

      results = Task.await_many(tasks, 15_000)
      assert length(results) == 50

      # Each task exercised all 5 public batch strategy atoms.
      assert Enum.all?(results, &(length(&1) == TestStrategyMatrix.batch_strategy_atom_count()))
    end

    test "many processes parsing the same large CSV concurrently" do
      csv =
        1..100
        |> Enum.map_join("\n", fn i -> "#{i},#{i * 2},#{i * 3}" end)
        |> Kernel.<>("\n")

      tasks =
        for _ <- 1..20 do
          Task.async(fn ->
            RustyCSV.RFC4180.parse_string(csv, skip_headers: false)
          end)
        end

      results = Task.await_many(tasks, 10_000)

      # All 20 should produce identical results
      first = hd(results)
      assert length(first) == 100

      for result <- results do
        assert result == first
      end
    end
  end

  # ============================================================================
  # Concurrent streaming with configured parsers
  # ============================================================================

  describe "concurrent access with configured parsers" do
    test "multiple configured parsers accessed concurrently" do
      # Create parsers with different configs
      parsers =
        for {sep, esc} <- [{44, 34}, {9, 34}, {59, 34}] do
          Native.streaming_new_with_config(sep, esc, :default)
        end

      # Feed data to all parsers concurrently
      tasks =
        for {parser, idx} <- Enum.with_index(parsers) do
          Task.async(fn ->
            sep = Enum.at([",", "\t", ";"], idx)

            for i <- 1..10 do
              Native.streaming_feed(parser, "a#{i}#{sep}b#{i}\n")
            end

            Native.streaming_next_rows(parser, 100)
          end)
        end

      results = Task.await_many(tasks, 5_000)

      for rows <- results do
        assert length(rows) == 10
      end
    end
  end

  # ============================================================================
  # Helpers
  # ============================================================================

  # Repeatedly drain rows from a parser until a timeout of empty drains
  defp drain_loop(parser, acc, remaining_empty) when remaining_empty > 0 do
    rows = Native.streaming_next_rows(parser, 100)

    if rows == [] do
      Process.sleep(5)
      drain_loop(parser, acc, remaining_empty - 1)
    else
      drain_loop(parser, acc ++ rows, remaining_empty)
    end
  end

  defp drain_loop(_parser, acc, _), do: acc
end
