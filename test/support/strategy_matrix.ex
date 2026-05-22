defmodule RustyCSV.TestStrategyMatrix do
  @moduledoc false

  @batch_strategy_atoms [:basic, :simd, :indexed, :parallel, :zero_copy]

  def batch_strategy_atoms, do: @batch_strategy_atoms
  def batch_strategy_atom_count, do: length(@batch_strategy_atoms)
end
