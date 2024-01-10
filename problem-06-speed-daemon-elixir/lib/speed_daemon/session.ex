defmodule SpeedDaemon.Session do
  defmodule Camera do
    defstruct [:road, :mile, :limit]
  end

  defmodule Dispatcher do
    defstruct [:roads]
  end

  defmodule Ticket do
    defstruct [:plate, :road, :mile1, :timestamp1, :mile2, :timestamp2, :speed]
  end

  defmodule Packet do
    defmodule Plate do
      defstruct [:plate, :timestamp]
    end

    defmodule WantHeartbeat do
      defstruct [:interval_ms]
    end

    defmodule IAmCamera do
      defstruct [:road, :mile, :limit]
    end

    defmodule IAmDispatcher do
      defstruct [:numroads, :roads]
    end
  end

  use GenServer, restart: :temporary

  @type_error 0x10
  @type_plate 0x20
  @type_ticket 0x21
  @type_want_heartbeat 0x40
  @type_heartbeat 0x41
  @type_i_am_camera 0x80
  @type_i_am_dispatcher 0x81

  @client_server_types [
    @type_plate,
    @type_want_heartbeat,
    @type_i_am_camera,
    @type_i_am_dispatcher
  ]

  alias __MODULE__
  alias SpeedDaemon.MeasurementServer

  defstruct [:client, type: nil, recv_buf: "", heartbeat_interval_ms: nil]

  ## Client

  def start_link(client) do
    session = %Session{client: client}
    GenServer.start_link(__MODULE__, session)
  end

  def deliver_ticket(pid, ticket = %Ticket{}) do
    GenServer.cast(pid, {:deliver_ticket, ticket})
  end

  ## Server

  def init(client) do
    {:ok, client}
  end

  def handle_cast(
        {:deliver_ticket, ticket = %Ticket{}},
        session = %Session{client: client}
      ) do
    speed = round(100 * ticket.speed)

    packet =
      <<@type_ticket, String.length(ticket.plate), ticket.plate::binary, ticket.road::size(16),
        ticket.mile1::size(16), ticket.timestamp1::size(32), ticket.mile2::size(16),
        ticket.timestamp2::size(32), speed::size(16)>>

    :ok = :gen_tcp.send(client, packet)
    {:noreply, session}
  end

  def handle_info({:tcp, _, packet}, session = %Session{recv_buf: recv_buf}) do
    session = %Session{session | recv_buf: recv_buf <> packet}

    process_recv_buf(session)
  end

  def handle_info({:tcp_closed, _socket}, session) do
    {:stop, :shutdown, session}
  end

  def handle_info(
        :heartbeat,
        session = %Session{client: client, heartbeat_interval_ms: heartbeat_interval_ms}
      ) do
    :ok = :gen_tcp.send(client, <<@type_heartbeat>>)

    schedule_heartbeat(heartbeat_interval_ms)

    {:noreply, session}
  end

  ## Private functions

  defp handle_error(%Session{client: client}, error) when is_atom(error) do
    error_str = to_string(error)
    packet = <<@type_error, String.length(error_str), error_str::binary>>
    :gen_tcp.send(client, packet)
    :gen_tcp.close(client)
  end

  defp schedule_heartbeat(after_ms) do
    Process.send_after(self(), :heartbeat, after_ms)
  end

  defp process_recv_buf(session = %Session{recv_buf: recv_buf}) do
    case parse_packet(recv_buf) do
      {:ok, {:not_enough, recv_buf}} ->
        {:noreply, %Session{session | recv_buf: recv_buf}}

      {:ok, {message, recv_buf}} ->
        session = %Session{session | recv_buf: recv_buf}

        case handle_message(session, message) do
          {:ok, session} ->
            process_recv_buf(session)

          {:error, error} ->
            handle_error(session, error)
            {:stop, :shutdown, session}
        end

      {:error, error} ->
        handle_error(session, error)
        {:stop, :shutdown, session}
    end
  end

  defp handle_message(
         session = %Session{type: %Camera{limit: limit, road: road, mile: mile}},
         %Packet.Plate{plate: plate, timestamp: timestamp}
       ) do
    MeasurementServer.register_measurement(plate, timestamp, limit, road, mile)
    {:ok, session}
  end

  defp handle_message(%Session{}, %Packet.Plate{}), do: {:error, :plate_when_no_camera}

  defp handle_message(session = %Session{heartbeat_interval_ms: nil}, %Packet.WantHeartbeat{
         interval_ms: interval_ms
       }) do
    if interval_ms > 0 do
      schedule_heartbeat(interval_ms)
    end

    {:ok, %Session{session | heartbeat_interval_ms: interval_ms}}
  end

  defp handle_message(%Session{}, %Packet.WantHeartbeat{}),
    do: {:error, :multiple_want_heartbeat}

  defp handle_message(%Session{type: type}, %Packet.IAmCamera{}) when not is_nil(type),
    do: {:error, :already_identified}

  defp handle_message(session = %Session{}, %Packet.IAmCamera{
         road: road,
         mile: mile,
         limit: limit
       }) do
    {:ok, %Session{session | type: %Camera{road: road, mile: mile, limit: limit}}}
  end

  defp handle_message(%Session{type: type}, %Packet.IAmDispatcher{}) when not is_nil(type),
    do: {:error, :already_identified}

  defp handle_message(session = %Session{}, %Packet.IAmDispatcher{roads: roads}) do
    dispatcher = %Dispatcher{roads: roads}
    MeasurementServer.register_dispatcher(dispatcher)
    {:ok, %Session{session | type: dispatcher}}
  end

  defp parse_packet(<<type, _rest::binary>>) when type not in @client_server_types do
    {:error, :invalid_type}
  end

  defp parse_packet(
         <<@type_plate, platelen, plate::binary-size(platelen), timestamp::size(32),
           rest::binary>>
       ) do
    {:ok, {%Packet.Plate{plate: plate, timestamp: timestamp}, rest}}
  end

  defp parse_packet(<<@type_want_heartbeat, interval_ds::size(32), rest::binary>>) do
    {:ok, {%Packet.WantHeartbeat{interval_ms: interval_ds * 100}, rest}}
  end

  defp parse_packet(
         <<@type_i_am_camera, road::size(16), mile::size(16), limit::size(16), rest::binary>>
       ) do
    {:ok, {%Packet.IAmCamera{road: road, mile: mile, limit: limit}, rest}}
  end

  defp parse_packet(
         <<@type_i_am_dispatcher, numroads::size(8), roads_chunk::binary-size(numroads * 2),
           rest::binary>>
       ) do
    roads =
      for <<road::size(16) <- roads_chunk>> do
        road
      end

    {:ok, {%Packet.IAmDispatcher{numroads: numroads, roads: roads}, rest}}
  end

  defp parse_packet(recv_buf) do
    {:ok, {:not_enough, recv_buf}}
  end
end
