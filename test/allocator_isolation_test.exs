defmodule RustyCSV.AllocatorIsolationTest do
  @moduledoc """
  Regression guard for the 2026-08-25 `beam.smp` segfault.

  Three VM crashes were traced to two NIFs in one BEAM each statically linking
  mimalloc. On macOS mimalloc keeps the per-thread default heap in a *fixed* TLS
  slot while keeping its arenas and page map private to the `.so`, so one copy's
  `mi_realloc` can receive a block the other copy allocated, miss the page-map
  lookup, report `usable_size == 0`, and copy zero bytes — handing back fresh,
  uninitialised memory and freeing the original. Every live value in that block
  is destroyed silently.

  RustyCSV is one half of that pair: the crashes were observed in RustyJson and
  reproduced on demand against `rusty_csv` 0.4.4, which bundled mimalloc by
  default. Either side of the pair is enough to cause it, and the damage lands
  in whichever library happens to reallocate next — nothing about RustyCSV's own
  code makes it the safe one.

  The fix is that RustyCSV no longer bundles an allocator by default. This test
  fails if that regresses, because the failure mode is silent VM-wide heap
  corruption that no functional test can catch: by the time anything observable
  goes wrong, it has gone wrong in unrelated code.

  Set `RUSTYCSV_ALLOW_BUNDLED_ALLOCATOR=1` if you are deliberately building with
  the opt-in `mimalloc` feature and have confirmed RustyCSV is the only
  mimalloc-bundling NIF in the VM. See `docs/ALLOCATOR_SAFETY.md`.
  """
  use ExUnit.Case, async: true

  # Emitted by mimalloc's own error reporting; present whenever it is linked in.
  @mimalloc_fingerprints ["mimalloc: error: ", "unable to extend the page map"]

  describe "bundled allocator" do
    @tag :allocator
    test "the loaded NIF does not statically link mimalloc" do
      case nif_path() do
        nil ->
          flunk("could not locate the loaded rustycsv NIF under priv/native")

        path ->
          blob = File.read!(path)
          found = Enum.filter(@mimalloc_fingerprints, &String.contains?(blob, &1))

          if System.get_env("RUSTYCSV_ALLOW_BUNDLED_ALLOCATOR") in ["1", "true"] do
            # Deliberate opt-in build. Nothing to enforce beyond having actually
            # located and read the artifact this assertion is meant to inspect.
            assert byte_size(blob) > 0
          else
            assert found == [],
                   """
                   #{Path.basename(path)} statically links mimalloc #{inspect(found)}.

                   Two NIFs in one BEAM that each bundle mimalloc share a fixed
                   macOS TLS slot while keeping private page maps. One copy's
                   mi_realloc then copies zero bytes instead of preserving the
                   block, which silently zeroes live terms and takes the VM down
                   in unrelated code. See docs/ALLOCATOR_SAFETY.md.

                   Build without the `mimalloc` cargo feature, or set
                   RUSTYCSV_ALLOW_BUNDLED_ALLOCATOR=1 if RustyCSV is provably the
                   only mimalloc-bundling NIF in this VM.
                   """
          end
      end
    end
  end

  defp nif_path do
    :rusty_csv
    |> :code.priv_dir()
    |> case do
      {:error, _} -> nil
      dir -> dir |> to_string() |> Path.join("native/*.{so,dll,dylib}") |> Path.wildcard()
    end
    |> case do
      nil -> nil
      [] -> nil
      paths -> Enum.max_by(paths, &File.stat!(&1).mtime)
    end
  end
end
