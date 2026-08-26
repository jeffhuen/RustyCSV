# CSV Encoding Benchmark: RustyCSV NIF vs NimbleCSV
#
# Usage: mix run bench/encode_bench.exs
#
# Measures real end-to-end dump_to_iodata performance through the actual
# APIs users would call. NimbleCSV is the library RustyCSV replaces, so
# it's the baseline that matters.
#
# Covers all four PostProcess code paths:
#   1. Plain UTF-8              (no formula, no encoding conversion)
#   2. Formula escaping         (escape_formula, UTF-8)
#   3. Non-UTF-8 encoding       (UTF-16 LE, no formula)
#   4. Formula + non-UTF-8      (escape_formula + UTF-16 LE)
#
# Memory tracking (optional):
#   When the `memory_tracking` Cargo feature is enabled, the benchmark
#   prints per-scenario peak NIF heap usage alongside correctness checks.
#   Enable it via native/rustycsv/Cargo.toml:
#     default = ["memory_tracking"]
#   then: FORCE_RUSTYCSV_BUILD=1 mix compile --force

# ── Module definitions ──────────────────────────────────────────────────

# 1. Plain UTF-8 (uses \n so output is byte-identical to NimbleCSV)
RustyCSV.define(RPlain, line_separator: "\n")
NimbleCSV.define(NPlain, line_separator: "\n")

# 2. Formula escaping (UTF-8)
formula_config = %{["=", "+", "-", "@"] => "'"}

RustyCSV.define(RFormula, line_separator: "\n", escape_formula: formula_config)
NimbleCSV.define(NFormula, line_separator: "\n", escape_formula: formula_config)

# 3. Non-UTF-8: UTF-16 LE tab-separated (spreadsheet format)
RustyCSV.define(RSpreadsheet,
  separator: "\t",
  encoding: {:utf16, :little},
  trim_bom: true,
  dump_bom: true
)

NimbleCSV.define(NSpreadsheet,
  separator: "\t",
  encoding: {:utf16, :little},
  trim_bom: true,
  dump_bom: true
)

# 4. Formula + UTF-16 LE
RustyCSV.define(RBoth,
  separator: "\t",
  encoding: {:utf16, :little},
  trim_bom: true,
  dump_bom: true,
  escape_formula: formula_config
)

NimbleCSV.define(NBoth,
  separator: "\t",
  encoding: {:utf16, :little},
  trim_bom: true,
  dump_bom: true,
  escape_formula: formula_config
)

defmodule EncodeBench do
  def run do
    IO.puts("\n=== CSV Encoding Benchmark: RustyCSV NIF vs NimbleCSV ===")
    IO.puts("Erlang/OTP #{System.otp_release()}, Elixir #{System.version()}")

    # Probe memory tracking: reset, do a tiny encode, check if peak > 0
    RustyCSV.Native.reset_rust_memory_stats()
    RPlain.dump_to_iodata([["a", "b"]])
    mem_enabled = RustyCSV.Native.get_rust_memory_peak() > 0

    if mem_enabled do
      IO.puts("NIF memory tracking: ENABLED")
    else
      IO.puts("NIF memory tracking: DISABLED")
      IO.puts("  To measure Rust-side allocation for apples-to-apples comparison:")
      IO.puts("  1. Edit native/rustycsv/Cargo.toml: default = [\"memory_tracking\"]")
      IO.puts("  2. Rebuild: FORCE_RUSTYCSV_BUILD=true mix compile --force")
      IO.puts("  3. Re-run this benchmark")
    end

    IO.puts("")

    # ── Datasets ──────────────────────────────────────────────────────
    db_10k = generate_db_export(10_000)
    db_100k = generate_db_export(100_000)
    ugc_10k = generate_user_content(10_000)
    wide_10k = generate_wide_table(10_000, 50)
    formula_10k = generate_formula_data(10_000)

    # Collect all results for the summary file
    results = []

    # ── 1. Plain UTF-8 ───────────────────────────────────────────────
    IO.puts("=== 1. Plain UTF-8 (no formula, no encoding) ===\n")

    results =
      results ++
        bench_section(
          "Plain UTF-8",
          [
            {"DB export (10K rows x 8 cols)", db_10k},
            {"DB export (100K rows x 8 cols)", db_100k},
            {"User content (10K rows, heavy quoting)", ugc_10k},
            {"Wide table (10K rows x 50 cols)", wide_10k}
          ],
          RPlain,
          NPlain,
          mem_enabled
        )

    # ── 2. Formula escaping (UTF-8) ──────────────────────────────────
    IO.puts("=== 2. Formula Escaping (UTF-8 + escape_formula) ===\n")

    results =
      results ++
        bench_section(
          "Formula UTF-8",
          [
            {"DB export (10K rows)", db_10k},
            {"Formula-heavy (10K rows, ~40% trigger)", formula_10k}
          ],
          RFormula,
          NFormula,
          mem_enabled
        )

    # ── 3. Non-UTF-8 encoding (UTF-16 LE) ────────────────────────────
    IO.puts("=== 3. Non-UTF-8 Encoding (UTF-16 LE, tab-separated) ===\n")

    results =
      results ++
        bench_section(
          "UTF-16 LE",
          [
            {"DB export (10K rows)", db_10k}
          ],
          RSpreadsheet,
          NSpreadsheet,
          mem_enabled
        )

    # ── 4. Formula + UTF-16 LE ───────────────────────────────────────
    IO.puts("=== 4. Formula + Non-UTF-8 (UTF-16 LE + escape_formula) ===\n")

    results =
      results ++
        bench_section(
          "Formula + UTF-16 LE",
          [
            {"Formula-heavy (10K rows)", formula_10k}
          ],
          RBoth,
          NBoth,
          mem_enabled
        )

    # ── Save results ─────────────────────────────────────────────────
    save_results(results, mem_enabled)
  end

  # ── Bench helper ─────────────────────────────────────────────────────

  defp bench_section(section, datasets, rusty_mod, nimble_mod, mem_enabled) do
    for {name, rows} <- datasets do
      # Reset tracking, encode once, read peak
      if mem_enabled, do: RustyCSV.Native.reset_rust_memory_stats()
      rusty = rusty_mod.dump_to_iodata(rows) |> IO.iodata_to_binary()
      nif_peak = if mem_enabled, do: RustyCSV.Native.get_rust_memory_peak(), else: nil

      nimble = nimble_mod.dump_to_iodata(rows) |> IO.iodata_to_binary()
      status = if rusty == nimble, do: "MATCH", else: "DIFF"

      mem_info =
        case nif_peak do
          nil -> "(NIF peak: disabled)"
          bytes -> "(NIF peak: #{format_bytes(bytes)})"
        end

      IO.puts("  #{name}: #{status} (#{byte_size(rusty)} bytes) #{mem_info}")

      if rusty != nimble do
        show_first_diff(rusty, nimble)
      end

      suite =
        Benchee.run(
          %{
            "NimbleCSV" => fn -> nimble_mod.dump_to_iodata(rows) end,
            "RustyCSV NIF" => fn -> rusty_mod.dump_to_iodata(rows) end
          },
          warmup: 2,
          time: 5,
          memory_time: 2,
          print: [configuration: false]
        )

      IO.puts("")

      # Extract stats from Benchee suite
      rusty_stats = Enum.find(suite.scenarios, &(&1.name == "RustyCSV NIF"))
      nimble_stats = Enum.find(suite.scenarios, &(&1.name == "NimbleCSV"))

      %{
        section: section,
        name: name,
        output_bytes: byte_size(rusty),
        correctness: status,
        nif_peak_bytes: nif_peak,
        rusty_ips: rusty_stats.run_time_data.statistics.ips,
        rusty_avg_us: rusty_stats.run_time_data.statistics.average,
        rusty_mem: rusty_stats.memory_usage_data.statistics.average,
        nimble_ips: nimble_stats.run_time_data.statistics.ips,
        nimble_avg_us: nimble_stats.run_time_data.statistics.average,
        nimble_mem: nimble_stats.memory_usage_data.statistics.average,
        speedup: nimble_stats.run_time_data.statistics.average / rusty_stats.run_time_data.statistics.average
      }
    end
  end

  # ── Results file ───────────────────────────────────────────────────

  defp save_results(results, mem_enabled) do
    timestamp = Calendar.strftime(DateTime.utc_now(), "%Y%m%dT%H%M%S")
    path = "bench/results/#{timestamp}_encode_summary.md"

    lines = [
      "# Encoding Benchmark Results - #{timestamp}",
      "",
      "## System",
      "- Elixir: #{System.version()}",
      "- OTP: #{System.otp_release()}",
      "- NIF memory tracking: #{if mem_enabled, do: "ENABLED", else: "DISABLED"}",
      "",
      "## Results",
      "",
      build_results_table(results, mem_enabled),
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

  defp build_results_table(results, _mem_enabled) do
    header = "| Section | Scenario | Output | RustyCSV ips | NimbleCSV ips | Speedup | Correctness |"
    sep = "|---------|----------|--------|-------------|---------------|---------|-------------|"

    rows =
      Enum.map(results, fn r ->
        "| #{r.section} | #{r.name} | #{format_bytes(r.output_bytes)} | #{Float.round(r.rusty_ips, 1)} | #{Float.round(r.nimble_ips, 1)} | **#{Float.round(r.speedup, 2)}x** | #{r.correctness} |"
      end)

    Enum.join([header, sep | rows], "\n")
  end

  defp build_memory_table(results, mem_enabled) do
    if mem_enabled do
      header = "| Section | Scenario | NIF Peak | BEAM (RustyCSV) | **Total (RustyCSV)** | BEAM (NimbleCSV) | Ratio |"
      sep = "|---------|----------|----------|-----------------|----------------------|------------------|-------|"

      rows =
        Enum.map(results, fn r ->
          rusty_beam = if r.rusty_mem, do: round(r.rusty_mem), else: 0
          nimble_beam = if r.nimble_mem, do: round(r.nimble_mem), else: 0
          nif_peak = r.nif_peak_bytes || 0
          rusty_total = nif_peak + rusty_beam

          ratio =
            if nimble_beam > 0 do
              "#{Float.round(rusty_total / nimble_beam, 1)}x"
            else
              "N/A"
            end

          "| #{r.section} | #{r.name} | #{format_bytes(nif_peak)} | #{format_bytes(rusty_beam)} | **#{format_bytes(rusty_total)}** | #{format_bytes(nimble_beam)} | #{ratio} |"
        end)

      Enum.join([header, sep | rows], "\n")
    else
      header = "| Section | Scenario | BEAM (RustyCSV) | BEAM (NimbleCSV) |"
      sep = "|---------|----------|-----------------|------------------|"

      rows =
        Enum.map(results, fn r ->
          rusty_mem = if r.rusty_mem, do: format_bytes(round(r.rusty_mem)), else: "N/A"
          nimble_mem = if r.nimble_mem, do: format_bytes(round(r.nimble_mem)), else: "N/A"

          "| #{r.section} | #{r.name} | #{rusty_mem} | #{nimble_mem} |"
        end)

      Enum.join([header, sep | rows], "\n")
    end
  end

  defp show_first_diff(rusty, nimble) do
    rusty_bytes = :binary.bin_to_list(rusty)
    nimble_bytes = :binary.bin_to_list(nimble)

    Enum.zip(rusty_bytes, nimble_bytes)
    |> Enum.with_index()
    |> Enum.find(fn {{r, n}, _i} -> r != n end)
    |> case do
      {{r, n}, i} ->
        IO.puts("    First byte diff at index #{i}: rusty=#{r}, nimble=#{n}")

      nil ->
        IO.puts("    (same content, different length: rusty=#{length(rusty_bytes)}, nimble=#{length(nimble_bytes)})")
    end
  end

  defp format_bytes(bytes) when bytes >= 1_048_576,
    do: "#{Float.round(bytes / 1_048_576, 1)} MB"

  defp format_bytes(bytes) when bytes >= 1_024,
    do: "#{Float.round(bytes / 1_024, 1)} KB"

  defp format_bytes(bytes), do: "#{bytes} B"

  # ── Data generators ─────────────────────────────────────────────────

  defp generate_db_export(count) do
    for i <- 1..count do
      [
        Integer.to_string(i),
        Enum.random(~w[Alice Bob Carol Dave Eve Frank Grace Heidi]),
        Enum.random(~w[Smith Johnson Williams Brown Jones Garcia Miller Davis]),
        "user#{i}@example.com",
        Enum.random([
          "New York",
          "San Francisco",
          "Portland, OR",
          "Austin",
          "Chicago",
          "Seattle",
          "Denver",
          "Miami",
          "Boston, MA",
          "Nashville"
        ]),
        "2024-#{String.pad_leading(Integer.to_string(Enum.random(1..12)), 2, "0")}-#{String.pad_leading(Integer.to_string(Enum.random(1..28)), 2, "0")}",
        Enum.random(~w[free starter pro enterprise]),
        :erlang.float_to_binary(Enum.random(0..99999) / 100, decimals: 2)
      ]
    end
  end

  defp generate_user_content(count) do
    descriptions = [
      ~s(Great product, works as advertised!),
      ~s(Not bad for the price. Could be better.),
      ~s(Arrived broken. Contacted support, they said "we'll look into it" but never followed up.),
      ~s(Love it!\nWorks perfectly with my setup.\nHighly recommend.),
      ~s(Size runs small, order one size up.),
      ~s(The "premium" version is basically the same as the regular one...),
      ~s(Pros: fast, reliable\nCons: expensive, loud fan),
      ~s(5 stars! Best purchase I've made this year.),
      ~s(Returned it. The description said "waterproof" but it's clearly not.),
      ~s(OK for basic use. Nothing special.),
      ~s(My kids love this! We bought 3, one for each of them.),
      ~s(Shipping took forever. Product itself is fine, I guess.)
    ]

    for i <- 1..count do
      [
        "SKU-#{String.pad_leading(Integer.to_string(rem(i, 9999)), 4, "0")}",
        Integer.to_string(Enum.random(1..5)),
        Enum.random([
          "Great!",
          ~s(Not worth the "premium" price),
          "Decent product, fast shipping",
          "Meh",
          "Changed my life, seriously"
        ]),
        Enum.random(descriptions),
        "2024-#{String.pad_leading(Integer.to_string(Enum.random(1..12)), 2, "0")}-#{String.pad_leading(Integer.to_string(Enum.random(1..28)), 2, "0")}"
      ]
    end
  end

  defp generate_wide_table(rows, cols) do
    for i <- 1..rows do
      for j <- 1..cols do
        case rem(j, 4) do
          0 -> Integer.to_string(Enum.random(0..9999))
          1 -> :erlang.float_to_binary(Enum.random(0..9999) / 100, decimals: 2)
          2 -> Enum.random(~w[A B C D E F])
          3 -> "val_#{i}_#{j}"
        end
      end
    end
  end

  defp generate_formula_data(count) do
    for i <- 1..count do
      trigger? = rem(i, 5) < 2

      [
        Integer.to_string(i),
        if(trigger?,
          do: Enum.random(["-$42.50", "+15%", "=SUM(A1:A10)", "@admin"]),
          else: "$#{Enum.random(1..999)}.#{String.pad_leading(Integer.to_string(Enum.random(0..99)), 2, "0")}"
        ),
        Enum.random(~w[Alice Bob Carol Dave Eve]),
        if(trigger? and rem(i, 3) == 0,
          do: "=HYPERLINK(\"https://evil.com\")",
          else: "Regular note #{i}"
        ),
        "2024-01-#{String.pad_leading(Integer.to_string(rem(i, 28) + 1), 2, "0")}"
      ]
    end
  end
end

EncodeBench.run()
