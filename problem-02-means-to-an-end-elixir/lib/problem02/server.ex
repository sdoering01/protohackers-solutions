defmodule Problem02.Server do
  use Task

  def start_link(arg) do
    IO.puts("Server start_link")
    Task.start_link(__MODULE__, :run, [arg])
  end

  def run(_arg) do
    {:ok, socket} = :gen_tcp.listen(10000, [:binary, active: false, reuseaddr: true])
    loop_acceptor(socket)
  end

  defp loop_acceptor(socket) do
    {:ok, client} = :gen_tcp.accept(socket)

    {:ok, pid} =
      Task.Supervisor.start_child(Problem02.SessionSupervisor, fn -> client_loop(client) end)

    :gen_tcp.controlling_process(client, pid)
    loop_acceptor(socket)
  end

  defp client_loop(client, price_data \\ []) do
    {:ok, packet} = :gen_tcp.recv(client, 9)

    price_data =
      case packet do
        <<?Q, mintime::signed-integer-size(32), maxtime::signed-integer-size(32)>> ->
          filtered_price_data =
            Enum.filter(price_data, fn {timestamp, _price} ->
              timestamp >= mintime && timestamp <= maxtime
            end)

          count = Enum.count(filtered_price_data)

          mean =
            filtered_price_data
            |> Enum.reduce(0, fn {_timestamp, price}, acc ->
              acc + price / count
            end)
            |> round()

          mean_bin = <<mean::big-signed-integer-size(32)>>
          :gen_tcp.send(client, mean_bin)
          price_data

        <<?I, timestamp::signed-integer-size(32), price::signed-integer-size(32)>> ->
          [{timestamp, price} | price_data]
      end
    client_loop(client, price_data)
  end
end
