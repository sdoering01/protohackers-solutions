defmodule BudgetChatProxy.Application do
  use Application

  def start(_type, _args) do
    children = [
      BudgetChatProxy.Proxy,
      {Task.Supervisor, name: BudgetChatProxy.TaskSupervisor}
    ]

    opts = [strategy: :one_for_one, name: BudgetChatProxy.Application]
    Supervisor.start_link(children, opts)
  end
end
