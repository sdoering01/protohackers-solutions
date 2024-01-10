defmodule BudgetChat.Chat do
  use GenServer

  ## Client

  def start_link([]) do
    GenServer.start_link(__MODULE__, :ok, name: __MODULE__)
  end

  def join_player(username) do
    GenServer.call(__MODULE__, {:join, username})
  end

  def broadcast(sending_username, message) do
    message = "[#{sending_username}] #{message}"

    GenServer.call(__MODULE__, {:broadcast, {sending_username, message}})
  end

  ## Server

  @impl true
  def init(:ok) do
    refs = %{}
    users = %{}
    {:ok, {refs, users}}
  end

  @impl true
  def handle_call({:join, username}, {pid, _tag}, {refs, users}) do
    for joined_pid <- Map.values(users) do
      BudgetChat.Session.send_message(joined_pid, "* #{username} has entered the room\n")
    end

    existing_users_string =
      users
      |> Map.keys()
      |> Enum.join(", ")

    BudgetChat.Session.send_message(
      pid,
      "* Users already in the room: #{existing_users_string}\n"
    )

    ref = Process.monitor(pid)
    refs = Map.put(refs, ref, username)
    users = Map.put(users, username, pid)
    {:reply, :ok, {refs, users}}
  end

  @impl true
  def handle_call({:broadcast, {sending_username, message}}, _from, {refs, users}) do
    for {username, pid} <- users, username != sending_username do
      BudgetChat.Session.send_message(pid, message)
    end

    {:reply, :ok, {refs, users}}
  end

  @impl true
  def handle_info({:DOWN, ref, :process, _object, _reason}, {refs, users}) do
    {username, refs} = Map.pop!(refs, ref)
    {_pid, users} = Map.pop!(users, username)

    for pid <- Map.values(users) do
      BudgetChat.Session.send_message(pid, "* #{username} has left the room\n")
    end

    {:noreply, {refs, users}}
  end
end
