defmodule BudgetChat.Server do
  use Task

  def start_link(arg) do
    Task.start_link(__MODULE__, :run, [arg])
  end

  def run(_arg) do
    {:ok, socket} = :gen_tcp.listen(10000, [:binary, reuseaddr: true, packet: :line])

    acceptor_loop(socket)
  end

  defp acceptor_loop(socket) do
    {:ok, client} = :gen_tcp.accept(socket)

    {:ok, pid} =
      Task.Supervisor.start_child(BudgetChat.TaskSupervisor, BudgetChat.Session, :serve, [client])

    :gen_tcp.controlling_process(client, pid)
    acceptor_loop(socket)
  end
end
