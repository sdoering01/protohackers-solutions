defmodule BudgetChatProxy.Proxy do
  use Task

  def start_link(_) do
    Task.start_link(__MODULE__, :run, [])
  end

  def run do
    {:ok, socket} = :gen_tcp.listen(10000, [:binary, active: false, packet: :line])
    serve(socket)
  end

  def serve(socket) do
    {:ok, client} = :gen_tcp.accept(socket)

    {:ok, pid} =
      Task.Supervisor.start_child(
        BudgetChatProxy.TaskSupervisor,
        BudgetChatProxy.Session,
        :proxy,
        [client]
      )

    :gen_tcp.controlling_process(client, pid)

    serve(socket)
  end
end
