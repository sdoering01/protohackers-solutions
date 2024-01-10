defmodule BudgetChat.Application do
  use Application

  def start(_type, _args) do
    children = [
      BudgetChat.Server,
      {Task.Supervisor, name: BudgetChat.TaskSupervisor},
      {Registry, name: BudgetChat.SessionRegistry, keys: :duplicate}
    ]

    opts = [strategy: :one_for_one, name: BudgetChat.Supervisor]
    Supervisor.start_link(children, opts)
  end
end
