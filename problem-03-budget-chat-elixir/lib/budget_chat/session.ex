defmodule BudgetChat.Session do
  @registry_key :chat

  ## Client

  def serve(client) do
    serve_lobby(client)
  end

  def send_message(pid, message) do
    send(pid, {:send_message, message})
  end

  ## Server

  defp serve_lobby(client) do
    :gen_tcp.send(client, "Welcome to budgetchat! What shall I call you?\n")

    receive do
      {:tcp, _socket, username} ->
        username = String.trim(username)

        if !valid_username?(username) do
          :gen_tcp.send(client, "The username has to be alphanumeric (A-Za-z0-9)\n")
          :gen_tcp.close(client)
        else
          # BudgetChat.Chat.join_player(username)
          # serve_chat(client, username)
          join_player(client, username)
        end

      {:tcp_closed, _socket} ->
        nil

      unexpected_message ->
        IO.puts(
          :stderr,
          "Unexpected message in Session.serve_lobby: #{inspect(unexpected_message)}"
        )
    end
  end

  defp join_player(client, username) do
    sessions =
      Registry.select(BudgetChat.SessionRegistry, [{{:_, :"$2", :"$3"}, [], [{{:"$2", :"$3"}}]}])

    username_string = sessions |> Enum.map(fn {_pid, username} -> username end) |> Enum.join(",")
    send_message(self(), "* Users currently in the room: #{username_string}\n")

    for {pid, _} <- sessions do
      send_message(pid, "* #{username} entered the room\n")
    end

    {:ok, _} = Registry.register(BudgetChat.SessionRegistry, @registry_key, username)
    serve_chat(client, username)
  end

  defp serve_chat(client, username) do
    receive do
      {:tcp, _socket, message} ->
        broadcast(username, message)
        # BudgetChat.Chat.broadcast(username, message)
        serve_chat(client, username)

      {:send_message, message} ->
        :gen_tcp.send(client, message)
        serve_chat(client, username)

      {:tcp_closed, _socket} ->
        nil

      unexpected_msg ->
        IO.puts(
          :stderr,
          "Unexpected message in Session.serve_chat: #{inspect(unexpected_msg)}"
        )
    end
  end

  defp broadcast(own_username, message) do
    message = "[#{own_username}] #{message}"

    sessions =
      Registry.select(BudgetChat.SessionRegistry, [{{:_, :"$2", :"$3"}, [], [{{:"$2", :"$3"}}]}])

    for {pid, username} <- sessions, username != own_username do
      send_message(pid, message)
    end
  end

  defp valid_username?(username) do
    username
    |> String.to_charlist()
    |> Enum.all?(fn grapheme ->
      grapheme in ?a..?z or grapheme in ?A..?Z or grapheme in ?0..?9
    end)
  end
end
