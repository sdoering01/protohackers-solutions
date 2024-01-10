defmodule SpeedDaemonTest do
  use ExUnit.Case
  doctest SpeedDaemon

  test "greets the world" do
    assert SpeedDaemon.hello() == :world
  end
end
