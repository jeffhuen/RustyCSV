# CSV Decoding Benchmark: RustyCSV Strategies vs NimbleCSV
#
# Usage: mix run bench/decode_bench.exs
#
# For memory tracking (requires feature flag):
#   1. Edit native/rustycsv/Cargo.toml: default = ["memory_tracking"]
#   2. FORCE_RUSTYCSV_BUILD=true mix compile --force
#   3. mix run bench/decode_bench.exs
#
# Strategies benchmarked:
#   :basic     - Byte-by-byte parsing (reference implementation)
#   :simd      - SIMD-accelerated via memchr (default)
#   :indexed   - Two-phase index-then-extract
#   :parallel  - Multi-threaded via rayon (dirty CPU scheduler)
#   :zero_copy - Sub-binary references (NimbleCSV-like memory model)
#   :streaming - Bounded-memory streaming (via parse_stream)

alias RustyCSV.RFC4180, as: CSV
alias NimbleCSV.RFC4180, as: NimbleCSV

defmodule DecodeBench do
  @strategies [:basic, :simd, :indexed, :parallel, :zero_copy]
  @output_dir "bench/results"

  def run do
    File.mkdir_p!(@output_dir)
    timestamp = DateTime.utc_now() |> DateTime.to_iso8601(:basic) |> String.slice(0..14)

    IO.puts("\n" <> String.duplicate("=", 70))
    IO.puts("CSV DECODING BENCHMARK")
    IO.puts("Timestamp: #{timestamp}")
    IO.puts("Strategies: #{inspect(@strategies)}")
    IO.puts(String.duplicate("=", 70))

    # System info
    print_system_info()

    # Check memory tracking
    mem_enabled = check_memory_tracking()

    # Generate test files
    test_files = generate_test_files()

    # Run benchmarks
    results = []

    # 1. Simple CSV (no quotes, no special chars)
    results = results ++ run_benchmark("Simple CSV", test_files.simple, mem_enabled)

    # 2. Quoted CSV (fields with quotes, commas, newlines)
    results = results ++ run_benchmark("Quoted CSV", test_files.quoted, mem_enabled)

    # 3. Mixed CSV (realistic - some quoted, some not)
    results = results ++ run_benchmark("Mixed CSV (Realistic)", test_files.mixed, mem_enabled)

    # 4. Large file benchmark (~7MB)
    results = results ++ run_benchmark("Large File (7MB)", test_files.large, mem_enabled)

    # 5. Very large file benchmark (~100MB) - demonstrates :parallel crossover
    results = results ++ run_parallel_crossover_benchmark("Very Large File (100MB)", test_files.very_large, mem_enabled)

    # 6. Streaming benchmark (fair comparison)
    run_streaming_benchmark(test_files.large_path)

    # 7. Memory comparison (with honest metrics)
    run_memory_comparison(test_files.mixed)

    # 8. Correctness verification
    verify_all_strategies(test_files.mixed)

    # Save results
    save_results(timestamp, results, mem_enabled)

    IO.puts("\n" <> String.duplicate("=", 70))
    IO.puts("BENCHMARK COMPLETE")
    IO.puts("Results saved to: #{@output_dir}/")
    IO.puts(String.duplicate("=", 70))
  end

  defp print_system_info do
    IO.puts("\n--- System Information ---")
    IO.puts("Elixir: #{System.version()}")
    IO.puts("OTP: #{System.otp_release()}")
    IO.puts("OS: #{:os.type() |> inspect()}")
    IO.puts("Schedulers: #{System.schedulers_online()}")
    IO.puts("RustyCSV: #{Application.spec(:rusty_csv, :vsn)}")
    IO.puts("NimbleCSV: #{Application.spec(:nimble_csv, :vsn)}")
  end

  defp check_memory_tracking do
    IO.puts("\n--- Memory Tracking ---")
    RustyCSV.Native.reset_rust_memory_stats()
    # Do a small allocation
    _ = CSV.parse_string("a,b\n1,2\n", skip_headers: false)
    peak = RustyCSV.Native.get_rust_memory_peak()
    mem_enabled = peak > 0

    if mem_enabled do
      IO.puts("Status: ENABLED (memory_tracking feature active)")
    else
      IO.puts("Status: DISABLED (returns 0 - enable memory_tracking feature for detailed stats)")
    end

    mem_enabled
  end

  defp generate_test_files do
    IO.puts("\n--- Generating Test Files ---")

    # Simple CSV (10K rows, no quotes)
    simple = generate_simple_csv(10_000)
    IO.puts("Simple CSV: #{format_size(byte_size(simple))} (10K rows)")

    # Quoted CSV (10K rows, all fields quoted, some with special chars)
    quoted = generate_quoted_csv(10_000)
    IO.puts("Quoted CSV: #{format_size(byte_size(quoted))} (10K rows)")

    # Mixed/realistic CSV (10K rows)
    mixed = generate_mixed_csv(10_000)
    IO.puts("Mixed CSV: #{format_size(byte_size(mixed))} (10K rows)")

    # Large file (100K rows, ~7MB)
    large_path = "bench/data/large_bench.csv"
    large = ensure_large_file(large_path, 100_000)
    IO.puts("Large CSV: #{format_size(byte_size(large))} (100K rows)")

    # Very large file (1.5M rows, ~100MB) - for parallel strategy crossover
    very_large_path = "bench/data/very_large_bench.csv"
    very_large = ensure_large_file(very_large_path, 1_500_000)
    IO.puts("Very Large CSV: #{format_size(byte_size(very_large))} (1.5M rows)")

    %{
      simple: simple,
      quoted: quoted,
      mixed: mixed,
      large: large,
      large_path: large_path,
      very_large: very_large,
      very_large_path: very_large_path
    }
  end

  defp run_benchmark(name, csv, mem_enabled) do
    IO.puts("\n" <> String.duplicate("-", 50))
    IO.puts("Benchmark: #{name}")
    IO.puts("Size: #{format_size(byte_size(csv))}")
    IO.puts(String.duplicate("-", 50))

    # Measure NIF peak per strategy
    nif_peaks =
      if mem_enabled do
        for strategy <- @strategies, into: %{} do
          RustyCSV.Native.reset_rust_memory_stats()
          _ = CSV.parse_string(csv, strategy: strategy)
          {strategy, RustyCSV.Native.get_rust_memory_peak()}
        end
      else
        %{}
      end

    # Build benchmark map
    benchmarks =
      @strategies
      |> Enum.map(fn strategy ->
        {"RustyCSV (#{strategy})", fn -> CSV.parse_string(csv, strategy: strategy) end}
      end)
      |> Map.new()
      |> Map.put("NimbleCSV", fn -> NimbleCSV.parse_string(csv) end)

    suite =
      Benchee.run(
        benchmarks,
        warmup: 1,
        time: 3,
        memory_time: 1,
        print: [configuration: false],
        formatters: [
          Benchee.Formatters.Console
        ]
      )

    extract_results(name, byte_size(csv), suite, @strategies, nif_peaks)
  end

  # Benchmark specifically for :parallel strategy on very large files
  defp run_parallel_crossover_benchmark(name, csv, mem_enabled) do
    IO.puts("\n" <> String.duplicate("-", 50))
    IO.puts("Benchmark: #{name}")
    IO.puts("Size: #{format_size(byte_size(csv))}")
    IO.puts("Purpose: Demonstrate :parallel strategy crossover point")
    IO.puts(String.duplicate("-", 50))

    # Only compare strategies relevant to large files
    strategies_to_test = [:simd, :zero_copy, :parallel]

    # Measure NIF peak per strategy
    nif_peaks =
      if mem_enabled do
        for strategy <- strategies_to_test, into: %{} do
          RustyCSV.Native.reset_rust_memory_stats()
          _ = CSV.parse_string(csv, strategy: strategy)
          {strategy, RustyCSV.Native.get_rust_memory_peak()}
        end
      else
        %{}
      end

    # Build benchmark map
    benchmarks =
      strategies_to_test
      |> Enum.map(fn strategy ->
        {"RustyCSV (#{strategy})", fn -> CSV.parse_string(csv, strategy: strategy) end}
      end)
      |> Map.new()
      |> Map.put("NimbleCSV", fn -> NimbleCSV.parse_string(csv) end)

    suite =
      Benchee.run(
        benchmarks,
        warmup: 1,
        time: 5,  # Longer time for more stable results on large files
        memory_time: 1,
        print: [configuration: false],
        formatters: [
          Benchee.Formatters.Console
        ]
      )

    extract_results(name, byte_size(csv), suite, strategies_to_test, nif_peaks)
  end

  defp run_streaming_benchmark(path) do
    IO.puts("\n" <> String.duplicate("-", 50))
    IO.puts("Benchmark: Streaming (Bounded Memory)")
    IO.puts("File: #{path}")
    IO.puts(String.duplicate("-", 50))

    # Get expected row count
    expected_rows = path |> File.read!() |> String.split("\n", trim: true) |> length() |> Kernel.-(1)
    IO.puts("Expected rows (excluding header): #{format_number(expected_rows)}")

    # RustyCSV streaming with binary chunks (unique capability)
    IO.puts("\n1. RustyCSV streaming (64KB binary chunks):")
    IO.puts("   Note: RustyCSV can handle arbitrary binary chunks")
    {rusty_chunk_time, rusty_chunk_count} = :timer.tc(fn ->
      path
      |> File.stream!([], 64 * 1024)
      |> CSV.parse_stream()
      |> Enum.count()
    end)
    IO.puts("   Rows: #{format_number(rusty_chunk_count)}")
    IO.puts("   Time: #{format_time(rusty_chunk_time)}")
    IO.puts("   Correct: #{rusty_chunk_count == expected_rows}")

    # RustyCSV streaming with line-based input (for fair comparison)
    IO.puts("\n2. RustyCSV streaming (line-based):")
    {rusty_line_time, rusty_line_count} = :timer.tc(fn ->
      path
      |> File.stream!()  # Line-based (default)
      |> CSV.parse_stream()
      |> Enum.count()
    end)
    IO.puts("   Rows: #{format_number(rusty_line_count)}")
    IO.puts("   Time: #{format_time(rusty_line_time)}")
    IO.puts("   Correct: #{rusty_line_count == expected_rows}")

    # NimbleCSV streaming (MUST use line-based input)
    IO.puts("\n3. NimbleCSV streaming (line-based - required):")
    IO.puts("   Note: NimbleCSV requires line-based streams")
    {nimble_time, nimble_count} = :timer.tc(fn ->
      path
      |> File.stream!()  # Line-based (required for NimbleCSV)
      |> NimbleCSV.parse_stream()
      |> Enum.count()
    end)
    IO.puts("   Rows: #{format_number(nimble_count)}")
    IO.puts("   Time: #{format_time(nimble_time)}")
    IO.puts("   Correct: #{nimble_count == expected_rows}")

    # Fair comparison (both line-based)
    IO.puts("\n--- Fair Comparison (both line-based) ---")
    speedup = nimble_time / rusty_line_time
    IO.puts("RustyCSV vs NimbleCSV: #{Float.round(speedup, 2)}x faster")

    # Highlight RustyCSV's unique capability
    IO.puts("\n--- RustyCSV Unique Capability ---")
    IO.puts("RustyCSV can process binary chunks (useful for network streams, etc.)")
    IO.puts("Binary chunk throughput: #{format_size(trunc(byte_size(File.read!(path)) / (rusty_chunk_time / 1_000_000)))}/sec")
  end

  defp run_memory_comparison(csv) do
    IO.puts("\n" <> String.duplicate("-", 50))
    IO.puts("Memory Comparison (HONEST METRICS)")
    IO.puts("CSV Size: #{format_size(byte_size(csv))}")
    IO.puts(String.duplicate("-", 50))

    IO.puts("\n=== IMPORTANT: Memory Measurement Methodology ===")
    IO.puts("- 'Process Heap': Memory delta in the calling process (excludes refc binaries)")
    IO.puts("- 'Total Retained': Actual RAM used by the parsed result (heap + binary refs)")
    IO.puts("- 'Rust NIF': Peak allocation on the Rust/NIF side during parsing")
    IO.puts("- NimbleCSV allocates entirely on BEAM; RustyCSV allocates on both sides")

    # Process heap memory (what Benchee measures - can be misleading!)
    IO.puts("\n1. Process Heap Memory (Benchee-style, excludes binaries):")
    IO.puts("   WARNING: This metric is misleading for sub-binary strategies!")
    for strategy <- @strategies do
      mem = measure_process_heap(fn -> CSV.parse_string(csv, strategy: strategy) end)
      IO.puts("   RustyCSV (#{strategy}): #{format_size(mem)}")
    end
    nimble_heap = measure_process_heap(fn -> NimbleCSV.parse_string(csv) end)
    IO.puts("   NimbleCSV: #{format_size(nimble_heap)}")

    # Total retained memory (honest measurement)
    IO.puts("\n2. Total Retained Memory (heap + binary refs - HONEST):")
    for strategy <- @strategies do
      mem = measure_total_retained(fn -> CSV.parse_string(csv, strategy: strategy) end)
      IO.puts("   RustyCSV (#{strategy}): #{format_size(mem)}")
    end
    nimble_total = measure_total_retained(fn -> NimbleCSV.parse_string(csv) end)
    IO.puts("   NimbleCSV: #{format_size(nimble_total)}")

    # Rust memory (if tracking enabled)
    peak = RustyCSV.Native.get_rust_memory_peak()
    if peak > 0 do
      IO.puts("\n3. Rust NIF Memory (peak allocation during parsing):")
      rust_mems = for strategy <- @strategies do
        RustyCSV.Native.reset_rust_memory_stats()
        _ = CSV.parse_string(csv, strategy: strategy)
        rust_peak = RustyCSV.Native.get_rust_memory_peak()
        IO.puts("   RustyCSV (#{strategy}): #{format_size(rust_peak)}")
        {strategy, rust_peak}
      end

      # Calculate true total (BEAM retained + Rust peak)
      IO.puts("\n4. True Total Memory (BEAM retained + Rust NIF):")
      for {strategy, rust_mem} <- rust_mems do
        beam_mem = measure_total_retained(fn -> CSV.parse_string(csv, strategy: strategy) end)
        total = beam_mem + rust_mem
        IO.puts("   RustyCSV (#{strategy}): #{format_size(total)} (#{format_size(beam_mem)} BEAM + #{format_size(rust_mem)} Rust)")
      end
      IO.puts("   NimbleCSV: #{format_size(nimble_total)} (all BEAM)")
    else
      IO.puts("\n3-4. Rust NIF Memory: SKIPPED (memory_tracking disabled)")
      IO.puts("   To measure Rust-side allocation for apples-to-apples comparison:")
      IO.puts("   1. Edit native/rustycsv/Cargo.toml:")
      IO.puts("      default = [\"memory_tracking\"]")
      IO.puts("   2. Rebuild: FORCE_RUSTYCSV_BUILD=true mix compile --force")
      IO.puts("   3. Re-run this benchmark")
    end

    # BEAM reductions
    IO.puts("\n5. BEAM Reductions (scheduler work):")
    IO.puts("   Note: Low reductions = less scheduler overhead, but NIFs can't be preempted")
    for strategy <- @strategies do
      reds = measure_reductions(fn -> CSV.parse_string(csv, strategy: strategy) end)
      IO.puts("   RustyCSV (#{strategy}): #{format_number(reds)}")
    end
    nimble_reds = measure_reductions(fn -> NimbleCSV.parse_string(csv) end)
    IO.puts("   NimbleCSV: #{format_number(nimble_reds)}")
  end

  defp verify_all_strategies(csv) do
    IO.puts("\n" <> String.duplicate("-", 50))
    IO.puts("Correctness Verification")
    IO.puts(String.duplicate("-", 50))

    results = for strategy <- @strategies, into: %{} do
      {strategy, CSV.parse_string(csv, strategy: strategy)}
    end
    nimble_result = NimbleCSV.parse_string(csv)

    # Check all RustyCSV strategies match each other
    reference = results[:simd]
    all_match = Enum.all?(@strategies, fn s -> results[s] == reference end)

    # Check RustyCSV matches NimbleCSV
    matches_nimble = reference == nimble_result

    IO.puts("All RustyCSV strategies identical: #{all_match}")
    IO.puts("RustyCSV matches NimbleCSV: #{matches_nimble}")
    IO.puts("Row count: #{length(reference)}")

    unless all_match do
      IO.puts("\nWARNING: Strategy mismatch detected!")
      for strategy <- @strategies do
        IO.puts("  #{strategy}: #{length(results[strategy])} rows")
      end
    end

    unless matches_nimble do
      IO.puts("\nWARNING: RustyCSV differs from NimbleCSV!")
      IO.puts("  RustyCSV: #{length(reference)} rows")
      IO.puts("  NimbleCSV: #{length(nimble_result)} rows")
    end
  end

  # Extract structured results from a Benchee suite for the summary file
  defp extract_results(name, csv_bytes, suite, strategies, nif_peaks) do
    nimble_stats = Enum.find(suite.scenarios, &(&1.name == "NimbleCSV"))

    for strategy <- strategies do
      scenario = Enum.find(suite.scenarios, &(&1.name == "RustyCSV (#{strategy})"))

      %{
        name: name,
        strategy: strategy,
        csv_bytes: csv_bytes,
        nif_peak_bytes: Map.get(nif_peaks, strategy),
        rusty_ips: scenario.run_time_data.statistics.ips,
        rusty_avg_us: scenario.run_time_data.statistics.average,
        rusty_mem: scenario.memory_usage_data.statistics.average,
        nimble_ips: nimble_stats.run_time_data.statistics.ips,
        nimble_avg_us: nimble_stats.run_time_data.statistics.average,
        nimble_mem: nimble_stats.memory_usage_data.statistics.average,
        speedup: nimble_stats.run_time_data.statistics.average / scenario.run_time_data.statistics.average
      }
    end
  end

  defp save_results(timestamp, results, mem_enabled) do
    path = "#{@output_dir}/#{timestamp}_decode_summary.md"

    lines = [
      "# Decoding Benchmark Results - #{timestamp}",
      "",
      "## System",
      "- Elixir: #{System.version()}",
      "- OTP: #{System.otp_release()}",
      "- Schedulers: #{System.schedulers_online()}",
      "- NIF memory tracking: #{if mem_enabled, do: "ENABLED", else: "DISABLED"}",
      "",
      "## Results",
      "",
      build_results_table(results),
      "",
      "## Memory Details",
      "",
      build_memory_table(results, mem_enabled),
      ""
    ]

    content = Enum.join(lines, "\n")
    File.write!(path, content)
    IO.puts("Results saved to #{path}")
  end

  defp build_results_table(results) do
    header = "| Scenario | Strategy | CSV Size | RustyCSV ips | NimbleCSV ips | Speedup |"
    sep = "|----------|----------|----------|-------------|---------------|---------|"

    rows =
      Enum.map(results, fn r ->
        "| #{r.name} | #{r.strategy} | #{format_size(r.csv_bytes)} | #{Float.round(r.rusty_ips, 1)} | #{Float.round(r.nimble_ips, 1)} | **#{Float.round(r.speedup, 2)}x** |"
      end)

    Enum.join([header, sep | rows], "\n")
  end

  defp build_memory_table(results, mem_enabled) do
    if mem_enabled do
      header = "| Scenario | Strategy | NIF Peak | BEAM (RustyCSV) | **Total (RustyCSV)** | BEAM (NimbleCSV) | Ratio |"
      sep = "|----------|----------|----------|-----------------|----------------------|------------------|-------|"

      rows =
        Enum.map(results, fn r ->
          rusty_beam = if r.rusty_mem, do: round(r.rusty_mem), else: 0
          nimble_beam = if r.nimble_mem, do: round(r.nimble_mem), else: 0
          nif_peak = r.nif_peak_bytes || 0
          rusty_total = nif_peak + rusty_beam

          ratio =
            if nimble_beam > 0 do
              "#{Float.round(rusty_total / nimble_beam, 2)}x"
            else
              "N/A"
            end

          "| #{r.name} | #{r.strategy} | #{format_size(nif_peak)} | #{format_size(rusty_beam)} | **#{format_size(rusty_total)}** | #{format_size(nimble_beam)} | #{ratio} |"
        end)

      Enum.join([header, sep | rows], "\n")
    else
      header = "| Scenario | Strategy | BEAM (RustyCSV) | BEAM (NimbleCSV) |"
      sep = "|----------|----------|-----------------|------------------|"

      rows =
        Enum.map(results, fn r ->
          rusty_mem = if r.rusty_mem, do: format_size(round(r.rusty_mem)), else: "N/A"
          nimble_mem = if r.nimble_mem, do: format_size(round(r.nimble_mem)), else: "N/A"

          "| #{r.name} | #{r.strategy} | #{rusty_mem} | #{nimble_mem} |"
        end)

      Enum.join([header, sep | rows], "\n")
    end
  end

  # --- Test Data Generation ---

  defp generate_simple_csv(rows) do
    header = "id,name,value,category,timestamp\n"
    data =
      1..rows
      |> Enum.map(fn i ->
        "#{i},user#{i},#{:rand.uniform(1000)},cat#{rem(i, 5)},2024-01-#{rem(i, 28) + 1}"
      end)
      |> Enum.join("\n")
    header <> data <> "\n"
  end

  defp generate_quoted_csv(rows) do
    header = ~s("id","name","description","amount","notes"\n)
    data =
      1..rows
      |> Enum.map(fn i ->
        # RFC 4180: escape quotes by doubling them
        desc = ~s(Description with ""quotes"" and, commas for row #{i})
        notes = if rem(i, 10) == 0, do: "Line 1\nLine 2", else: "Normal notes"
        ~s("#{i}","User #{i}","#{desc}","#{:rand.uniform(1000)}","#{notes}")
      end)
      |> Enum.join("\n")
    header <> data <> "\n"
  end

  defp generate_mixed_csv(rows) do
    header = "id,name,email,amount,description,status\n"
    data =
      1..rows
      |> Enum.map(fn i ->
        # Mix of quoted and unquoted fields (RFC 4180 compliant)
        name = if rem(i, 3) == 0, do: ~s("User, Jr. #{i}"), else: "User#{i}"
        # RFC 4180: escape quotes by doubling them
        desc = if rem(i, 5) == 0, do: ~s("Has ""quotes"" inside"), else: "Simple desc"
        amount = :rand.uniform() * 1000 |> Float.round(2)
        status = Enum.random(["active", "pending", "done"])
        "#{i},#{name},user#{i}@example.com,#{amount},#{desc},#{status}"
      end)
      |> Enum.join("\n")
    header <> data <> "\n"
  end

  defp ensure_large_file(path, rows) do
    if File.exists?(path) do
      File.read!(path)
    else
      File.mkdir_p!(Path.dirname(path))
      csv = generate_mixed_csv(rows)
      File.write!(path, csv)
      csv
    end
  end

  # --- Measurement Helpers ---

  # Process heap only (what Benchee measures - misleading for sub-binaries!)
  defp measure_process_heap(fun) do
    :erlang.garbage_collect()
    {_, mem_before} = :erlang.process_info(self(), :memory)
    result = fun.()
    {_, mem_after} = :erlang.process_info(self(), :memory)
    # Keep result alive to prevent GC
    _ = result
    max(0, mem_after - mem_before)
  end

  # Total retained memory including binary references (HONEST measurement)
  defp measure_total_retained(fun) do
    :erlang.garbage_collect()

    # Get baseline
    {:memory, heap_before} = :erlang.process_info(self(), :memory)
    {:binary, bins_before} = :erlang.process_info(self(), :binary)
    bin_size_before = bins_before |> Enum.map(&elem(&1, 1)) |> Enum.sum()

    result = fun.()

    # Force GC to clean up temporaries, but keep result
    :erlang.garbage_collect()

    {:memory, heap_after} = :erlang.process_info(self(), :memory)
    {:binary, bins_after} = :erlang.process_info(self(), :binary)
    bin_size_after = bins_after |> Enum.map(&elem(&1, 1)) |> Enum.sum()

    # Keep result alive
    _ = result

    heap_delta = max(0, heap_after - heap_before)
    bin_delta = max(0, bin_size_after - bin_size_before)

    heap_delta + bin_delta
  end

  defp measure_reductions(fun) do
    {:reductions, before} = Process.info(self(), :reductions)
    _ = fun.()
    {:reductions, after_reds} = Process.info(self(), :reductions)
    after_reds - before
  end

  # --- Formatting Helpers ---

  defp format_size(bytes) when bytes >= 1_000_000, do: "#{Float.round(bytes / 1_000_000, 2)} MB"
  defp format_size(bytes) when bytes >= 1_000, do: "#{Float.round(bytes / 1_000, 1)} KB"
  defp format_size(bytes), do: "#{bytes} B"

  defp format_number(n) when n >= 1_000_000, do: "#{Float.round(n / 1_000_000, 2)}M"
  defp format_number(n) when n >= 1_000, do: "#{Float.round(n / 1_000, 1)}K"
  defp format_number(n), do: "#{n}"

  defp format_time(microseconds) when microseconds >= 1_000_000 do
    "#{Float.round(microseconds / 1_000_000, 2)}s"
  end
  defp format_time(microseconds) when microseconds >= 1_000 do
    "#{Float.round(microseconds / 1_000, 2)}ms"
  end
  defp format_time(microseconds), do: "#{microseconds}µs"
end

DecodeBench.run()
