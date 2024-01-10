defmodule UnusualDatabase do
  use Application

  def start(_args, _type) do
    children = [
      UnusualDatabase.Server
    ]

    opts = [strategy: :one_for_one, name: UnusualDatabase]
    Supervisor.start_link(children, opts)
  end
end
