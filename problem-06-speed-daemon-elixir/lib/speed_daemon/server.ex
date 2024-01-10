defmodule SpeedDaemon.Server do
  use Task

  def start_link(_) do
    Task.start_link(__MODULE__, :run, [])
  end

  def run do
    {:ok, socket} = :gen_tcp.listen(10000, [:binary, reuseaddr: true])
    serve(socket)
  end

  def serve(socket) do
    {:ok, client} = :gen_tcp.accept(socket)

    {:ok, pid} =
      DynamicSupervisor.start_child(SpeedDaemon.SessionSupervisor, {SpeedDaemon.Session, client})

    :ok = :gen_tcp.controlling_process(client, pid)

    serve(socket)
  end
end
