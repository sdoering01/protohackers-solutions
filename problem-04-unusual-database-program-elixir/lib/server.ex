defmodule UnusualDatabase.Server do
  use Task

  @constant_keys ["version"]

  def start_link(_arg) do
    Task.start_link(__MODULE__, :run, [])
  end

  def run do
    {:ok, socket} = :gen_udp.open(10_000, [:binary, active: false])
    db = %{"version" => "Ken's Key-Value Store 1.0"}
    serve(socket, db)
  end

  def serve(socket, db) do
    {:ok, {address, port, packet}} = :gen_tcp.recv(socket, 0)

    db =
      case String.split(packet, "=", parts: 2) do
        [key] ->
          with value when not is_nil(value) <- Map.get(db, key) do
            :gen_udp.send(socket, {address, port}, "#{key}=#{value}")
          end

          db

        [key, value] when key not in @constant_keys ->
          Map.put(db, key, value)

        [_key, _value] ->
          db
      end

    serve(socket, db)
  end
end
