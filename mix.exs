defmodule RustyCSV.MixProject do
  use Mix.Project

  @version "0.3.10"
  @source_url "https://github.com/jeffhuen/rustycsv"

  def project do
    [
      app: :rusty_csv,
      version: @version,
      elixir: "~> 1.14",
      start_permanent: Mix.env() == :prod,
      deps: deps(),

      # Hex
      description: description(),
      package: package(),

      # Docs
      name: "RustyCSV",
      docs: docs()
    ]
  end

  def application do
    [
      extra_applications: [:logger]
    ]
  end

  defp description do
    """
    Ultra-fast CSV parsing for Elixir. A purpose-built Rust NIF with SIMD acceleration,
    parallel parsing, and bounded-memory streaming. Drop-in NimbleCSV replacement.
    """
  end

  defp package do
    [
      name: "rusty_csv",
      maintainers: ["Jeff Huen"],
      licenses: ["MIT"],
      links: %{
        "GitHub" => @source_url,
        "Changelog" => "https://hexdocs.pm/rusty_csv/changelog.html"
      },
      files: ~w(
        lib
        native/rustycsv/src
        native/rustycsv/Cargo.toml
        native/rustycsv/Cargo.lock
        native/rustycsv/rust-toolchain.toml
        checksum-Elixir.RustyCSV.Native.exs
        .formatter.exs
        mix.exs
        README.md
        LICENSE
      )
    ]
  end

  defp docs do
    [
      main: "readme",
      name: "RustyCSV",
      source_ref: "v#{@version}",
      source_url: @source_url,
      homepage_url: @source_url,
      extras: [
        "README.md": [title: "Overview"],
        "docs/ARCHITECTURE.md": [title: "Architecture"],
        "docs/BENCHMARK.md": [title: "Real-World Benchmarks"],
        "docs/COMPLIANCE.md": [title: "Compliance & Validation"],
        "CHANGELOG.md": [title: "Changelog"],
        LICENSE: [title: "License"]
      ],
      groups_for_modules: [
        Core: [
          RustyCSV,
          RustyCSV.RFC4180,
          RustyCSV.Spreadsheet
        ],
        Streaming: [
          RustyCSV.Streaming
        ],
        "Low-Level": [
          RustyCSV.Native
        ]
      ],
      groups_for_docs: [
        Parsing: &(&1[:section] == :parsing),
        Dumping: &(&1[:section] == :dumping),
        Configuration: &(&1[:section] == :config)
      ]
    ]
  end

  defp deps do
    [
      {:rustler, "~> 0.37", optional: true},
      {:rustler_precompiled, "~> 0.8"},
      {:nimble_csv, "~> 1.2", only: [:dev, :test]},
      {:stream_data, "~> 1.0", only: [:dev, :test]},
      {:benchee, "~> 1.0", only: :dev},
      {:credo, "~> 1.7", only: [:dev, :test], runtime: false},
      {:dialyxir, "~> 1.4", only: [:dev, :test], runtime: false},
      {:ex_doc, "~> 0.31", only: :dev, runtime: false}
    ]
  end
end
