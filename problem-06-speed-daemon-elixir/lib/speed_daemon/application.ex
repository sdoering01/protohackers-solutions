defmodule SpeedDaemon.Application do
  use Application

  @impl true
  def start(_type, _args) do
    children = [
      {Registry, name: SpeedDaemon.DispatcherRegistry, keys: :duplicate},
      SpeedDaemon.MeasurementServer,
      SpeedDaemon.Server,
      {DynamicSupervisor, name: SpeedDaemon.SessionSupervisor}
    ]

    opts = [strategy: :one_for_one, name: SpeedDaemon.Supervisor]
    Supervisor.start_link(children, opts)
  end
end
