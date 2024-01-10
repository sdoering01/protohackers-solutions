defmodule SpeedDaemon.MeasurementServer do
  use GenServer

  alias SpeedDaemon.Session
  alias SpeedDaemon.Session.{Dispatcher, Ticket}

  defmodule State do
    defstruct measurements_by_road: %{},
              undelivered_tickets_by_road: %{},
              ticket_days_by_plate: %{}
  end

  defmodule Measurement do
    defstruct [:plate, :timestamp, :limit, :mile]
  end

  ## Client

  def start_link(_args) do
    GenServer.start_link(__MODULE__, :ok, name: __MODULE__)
  end

  def register_dispatcher(dispatcher = %Dispatcher{roads: roads}) do
    for road <- roads do
      Registry.register(SpeedDaemon.DispatcherRegistry, road, nil)
    end

    GenServer.cast(__MODULE__, {:register_dispatcher, {self(), dispatcher}})
  end

  def register_measurement(plate, timestamp, limit, road, mile) do
    GenServer.cast(__MODULE__, {:register_measurement, {plate, timestamp, limit, road, mile}})
  end

  ## Server

  def init(:ok) do
    {:ok, %State{}}
  end

  def handle_cast({:register_dispatcher, {pid, %Dispatcher{roads: roads}}}, state) do
    state = try_deliver_undelivered_tickets(roads, pid, state)
    {:noreply, state}
  end

  def handle_cast(
        {:register_measurement, {plate, timestamp, limit, road, mile}},
        state = %State{measurements_by_road: measurements_by_road}
      ) do
    measurement = %Measurement{plate: plate, timestamp: timestamp, limit: limit, mile: mile}

    tickets =
      measurements_by_road
      |> Map.get(road, [])
      # Could optimize this by putting another map by plate into the measurements map
      |> Enum.filter(&(&1.plate == plate))
      |> Enum.zip(Stream.cycle([measurement]))
      |> Enum.map(fn
        {m1, m2} when m1.timestamp > m2.timestamp -> {m2, m1}
        tpl -> tpl
      end)
      |> Enum.map(fn {m1, m2} ->
        # Timestamp in s, we want mph
        speed = abs(m2.mile - m1.mile) / (m2.timestamp - m1.timestamp) * 3600

        if speed > m1.limit do
          %Ticket{
            plate: m1.plate,
            road: road,
            mile1: m1.mile,
            timestamp1: m1.timestamp,
            mile2: m2.mile,
            timestamp2: m2.timestamp,
            speed: speed
          }
        else
          nil
        end
      end)
      |> Enum.filter(&(not is_nil(&1)))

    state = deliver_tickets_to_road(state, tickets, road)

    state =
      update_in(state.measurements_by_road[road], fn
        nil -> [measurement]
        prev -> [measurement | prev]
      end)

    {:noreply, state}
  end

  ## Private functions

  defp deliver_tickets_to_road(state, [], _road) do
    state
  end

  defp deliver_tickets_to_road(state, tickets, road) do
    dispatcher_pid =
      case Registry.lookup(SpeedDaemon.DispatcherRegistry, road) do
        [{pid, _value} | _] -> pid
        [] -> nil
      end

    deliver_tickets_to_road(state, tickets, road, dispatcher_pid)
  end

  defp deliver_tickets_to_road(state = %State{}, [], _road, _dispatcher_pid) do
    state
  end

  defp deliver_tickets_to_road(
         state = %State{},
         [ticket = %Ticket{plate: plate} | rest_tickets],
         road,
         dispatcher_pid
       ) do
    # For now check for the "Only 1 ticket per car per day" rule here, even if
    # there is currently no dispatcher at the road.
    state =
      if already_ticketed?(ticket, state.ticket_days_by_plate) do
        state
      else
        state = update_ticket_days(state, plate, ticket)

        if is_nil(dispatcher_pid) do
          update_in(state.undelivered_tickets_by_road[road], fn
            nil -> [ticket]
            tickets -> [ticket | tickets]
          end)
        else
          Session.deliver_ticket(dispatcher_pid, ticket)
          state
        end
      end

    deliver_tickets_to_road(state, rest_tickets, road, dispatcher_pid)
  end

  defp get_day(timestamp) do
    floor(timestamp / 86400)
  end

  defp already_ticketed?(ticket = %Ticket{plate: plate}, ticket_days_by_plate) do
    case Map.get(ticket_days_by_plate, plate) do
      nil ->
        false

      ticket_days ->
        not MapSet.disjoint?(ticket_days, MapSet.new(get_ticket_days(ticket)))
    end
  end

  defp get_ticket_days(%Ticket{timestamp1: timestamp1, timestamp2: timestamp2}) do
    get_day(timestamp1)..get_day(timestamp2)
  end

  defp update_ticket_days(state = %State{}, plate, ticket = %Ticket{}) do
    new_ticket_days = get_ticket_days(ticket)

    update_in(state.ticket_days_by_plate[plate], fn
      nil ->
        MapSet.new(new_ticket_days)

      ticket_days ->
        Enum.reduce(new_ticket_days, ticket_days, fn new_day, acc ->
          MapSet.put(acc, new_day)
        end)
    end)
  end

  defp try_deliver_undelivered_tickets([], _pid, state = %State{}) do
    state
  end

  defp try_deliver_undelivered_tickets(
         [road | roads],
         pid,
         state = %State{}
       ) do
    {tickets, state} = pop_in(state.undelivered_tickets_by_road[road])

    if not is_nil(tickets) do
      for ticket <- tickets do
        Session.deliver_ticket(pid, ticket)
      end
    end

    try_deliver_undelivered_tickets(roads, pid, state)
  end
end
