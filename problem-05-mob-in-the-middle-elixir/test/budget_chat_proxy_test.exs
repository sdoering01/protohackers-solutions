defmodule BudgetChatProxyTest do
  use ExUnit.Case
  doctest BudgetChatProxy

  test "greets the world" do
    assert BudgetChatProxy.hello() == :world
  end
end
