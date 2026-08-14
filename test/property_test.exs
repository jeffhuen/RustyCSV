defmodule RustyCSV.PropertyTest do
  @moduledoc """
  Property-based tests using StreamData.

  Verifies invariants that must hold for all inputs:
  - All strategies produce identical output
  - parse → dump → parse round-trips
  - Streaming produces the same result as batch parsing
  - No panics on adversarial input
  """
  use ExUnit.Case, async: true
  use ExUnitProperties

  alias RustyCSV.RFC4180, as: CSV
  alias RustyCSV.TestStrategyMatrix

  # ============================================================================
  # Generators
  # ============================================================================

  # A CSV field: printable string that avoids the separator, escape, and newlines
  # so we get "clean" fields (no quoting needed). This lets us test structural
  # consistency across strategies without quoting ambiguity.
  # min_length: 1 ensures no empty-only rows that produce bare "\n" lines.
  defp clean_field do
    gen all(
          str <-
            StreamData.string(Enum.concat([?a..?z, ?A..?Z, ?0..?9]),
              min_length: 1,
              max_length: 20
            )
        ) do
      str
    end
  end

  # A CSV document: 1–20 rows, all with the same number of fields
  defp clean_csv do
    gen all(
          num_fields <- StreamData.integer(1..8),
          rows <-
            StreamData.list_of(
              StreamData.list_of(clean_field(), length: num_fields),
              min_length: 1,
              max_length: 20
            )
        ) do
      rows
    end
  end

  # Arbitrary binary — may contain any bytes including quotes, commas, newlines
  defp adversarial_binary do
    StreamData.binary(min_length: 0, max_length: 500)
  end

  # Build a CSV string from rows
  defp rows_to_csv(rows) do
    rows
    |> Enum.map_join("\n", fn row -> Enum.join(row, ",") end)
    |> Kernel.<>("\n")
  end

  # ============================================================================
  # Strategy consistency
  # ============================================================================

  property "all batch strategies produce identical output for clean CSV" do
    check all(rows <- clean_csv(), max_runs: 200) do
      csv = rows_to_csv(rows)
      opts = [skip_headers: false]

      basic = CSV.parse_string(csv, [{:strategy, :basic} | opts])
      simd = CSV.parse_string(csv, [{:strategy, :simd} | opts])
      indexed = CSV.parse_string(csv, [{:strategy, :indexed} | opts])
      parallel = CSV.parse_string(csv, [{:strategy, :parallel} | opts])
      zero_copy = CSV.parse_string(csv, [{:strategy, :zero_copy} | opts])

      assert basic == simd,
             "basic vs simd mismatch on: #{inspect(csv)}"

      assert simd == indexed,
             "simd vs indexed mismatch on: #{inspect(csv)}"

      assert indexed == parallel,
             "indexed vs parallel mismatch on: #{inspect(csv)}"

      assert parallel == zero_copy,
             "parallel vs zero_copy mismatch on: #{inspect(csv)}"
    end
  end

  # ============================================================================
  # Round-trip: parse → dump → parse
  # ============================================================================

  property "parse then dump then parse round-trips for clean CSV" do
    check all(rows <- clean_csv(), max_runs: 200) do
      csv = rows_to_csv(rows)

      parsed = CSV.parse_string(csv, skip_headers: false)
      dumped = parsed |> CSV.dump_to_iodata() |> IO.iodata_to_binary()
      reparsed = CSV.parse_string(dumped, skip_headers: false)

      assert parsed == reparsed,
             "round-trip failed:\n  original: #{inspect(csv)}\n  dumped: #{inspect(dumped)}"
    end
  end

  # ============================================================================
  # Streaming consistency
  # ============================================================================

  property "streaming produces same result as batch parsing for clean CSV" do
    check all(
            rows <- clean_csv(),
            chunk_sizes <-
              StreamData.list_of(StreamData.integer(1..50), min_length: 1, max_length: 50),
            max_runs: 100
          ) do
      csv = rows_to_csv(rows)
      batch = CSV.parse_string(csv, skip_headers: false)

      # Chunking is generator-driven so failures are reproducible under the test seed.
      chunks = chunk_binary(csv, chunk_sizes)

      streamed =
        chunks
        |> Stream.map(& &1)
        |> CSV.parse_stream(skip_headers: false)
        |> Enum.to_list()

      assert batch == streamed,
             "batch vs streaming mismatch on: #{inspect(csv)}"
    end
  end

  # ============================================================================
  # Adversarial input — no panics
  # ============================================================================

  property "no strategy panics on arbitrary binary input" do
    check all(input <- adversarial_binary(), max_runs: 300) do
      for strategy <- TestStrategyMatrix.batch_strategy_atoms() do
        # Should either return a result or raise a clean Elixir error, never panic
        try do
          CSV.parse_string(input, strategy: strategy, skip_headers: false)
        rescue
          # Any Elixir-level exception is acceptable (bad encoding, etc.)
          _ -> :ok
        end
      end
    end
  end

  property "streaming does not panic on arbitrary binary chunks" do
    check all(
            chunks <- StreamData.list_of(adversarial_binary(), min_length: 1, max_length: 10),
            max_runs: 200
          ) do
      try do
        chunks
        |> Stream.map(& &1)
        |> CSV.parse_stream(skip_headers: false)
        |> Enum.to_list()
      rescue
        _ -> :ok
      end
    end
  end

  # ============================================================================
  # Helpers
  # ============================================================================

  # Split a binary into chunks using the provided sequence of chunk sizes.
  # Sizes are cycled so the property stays deterministic for a given generator seed.
  defp chunk_binary(bin, chunk_sizes) do
    chunk_binary(bin, chunk_sizes, chunk_sizes, [])
  end

  defp chunk_binary(<<>>, _chunk_sizes, _original_chunk_sizes, acc), do: Enum.reverse(acc)

  defp chunk_binary(bin, [], original_chunk_sizes, acc) do
    chunk_binary(bin, original_chunk_sizes, original_chunk_sizes, acc)
  end

  defp chunk_binary(bin, [size | remaining_sizes], original_chunk_sizes, acc) do
    size = min(size, byte_size(bin))
    <<chunk::binary-size(^size), remaining_bin::binary>> = bin
    chunk_binary(remaining_bin, remaining_sizes, original_chunk_sizes, [chunk | acc])
  end
end
