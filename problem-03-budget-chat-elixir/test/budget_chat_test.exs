defmodule BudgetChatTest do
  use ExUnit.Case
  doctest BudgetChat

  test "greets the world" do
    assert BudgetChat.hello() == :world
  end
end
