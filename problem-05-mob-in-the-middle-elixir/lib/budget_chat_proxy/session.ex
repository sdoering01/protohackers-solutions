defmodule BudgetChatProxy.Session do
  @server_addr String.to_charlist("chat.protohackers.com")
  @server_port 16963

  @address_regex ~r/^7[a-zA-Z0-9]{25,34}$/
  @replace_address "7YWHMfk9JZe0LM0g1ZauHuiSxhI"

  def proxy(client) do
    {:ok, server} =
      :gen_tcp.connect(@server_addr, @server_port, [:binary, active: false, packet: :line])

    {:ok, pid} = Task.start_link(fn -> proxy_server_to_client(client, server) end)
    :gen_tcp.controlling_process(server, pid)

    proxy_client_to_server(client, server)
  end

  defp proxy_server_to_client(client, server) do
    {:ok, packet} = :gen_tcp.recv(server, 0)
    packet = rewrite_addresses(packet)
    :ok = :gen_tcp.send(client, packet)
    proxy_server_to_client(client, server)
  end

  defp proxy_client_to_server(client, server) do
    {:ok, packet} = :gen_tcp.recv(client, 0)
    packet = rewrite_addresses(packet)
    :ok = :gen_tcp.send(server, packet)
    proxy_client_to_server(client, server)
  end

  def rewrite_addresses(packet) do
    packet
    |> String.trim_trailing("\n")
    |> String.split(" ")
    |> Enum.map(fn word ->
      if String.match?(word, @address_regex) do
        @replace_address
      else
        word
      end
    end)
    |> Enum.join(" ")
    |> Kernel.<>("\n")
  end
end
