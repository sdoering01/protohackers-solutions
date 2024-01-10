defmodule LineReversal.Server do
  use Task
  alias LineReversal.{LRCP, AppSession}

  def start_link(_) do
    Task.start_link(__MODULE__, :run, [])
  end

  def run do
    IO.puts("Starting server")
    {:ok, lrcp} = LRCP.listen(10000)
    serve(lrcp)
  end

  defp serve(lrcp) do
    {:ok, _lrcp, session} = LRCP.accept(lrcp)

    {:ok, pid} =
      Task.Supervisor.start_child(LineReversal.AppSessionSupervisor, AppSession, :serve, [session])

    LRCP.session_controlling_process(lrcp, session, pid)
    serve(lrcp)
  end
end
